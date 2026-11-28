// SPDX-License-Identifier: GPL-3.0-only
// Live recording (the browser's MediaRecorder): pictures (XRGB, any size, at the
// caller's own times) and stereo s16 sound in; a streaming WebM out through the
// caller's write callback, a cluster at a time, so a page gets playable pieces while
// it records. VP8 or VP9 (libvpx, realtime) and Opus (FFmpeg's own encoder). No seeks:
// the file has no index and no duration, like the WebM other browsers record.
// Like the recorder, it opens no files and holds no grants.
#include <stdint.h>
#include <string.h>
#include <errno.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/audio_fifo.h>
#include <libavutil/channel_layout.h>
#include <libavutil/mem.h>
#include <libavutil/opt.h>
#include <libswscale/swscale.h>

typedef int (*sx_live_write_fn)(void *, const uint8_t *, int);

struct sx_live {
    AVFormatContext *format;
    AVIOContext *io;
    AVCodecContext *video, *audio;
    AVStream *vstream, *astream;
    AVFrame *picture, *sound;
    AVPacket *packet;
    struct SwsContext *scale;
    AVAudioFifo *fifo;
    int width, height, finished;
    int64_t last_ms, samples_out;
};

void sx_live_free(struct sx_live *s) {
    if (!s) return;
    av_packet_free(&s->packet);
    av_frame_free(&s->picture);
    av_frame_free(&s->sound);
    avcodec_free_context(&s->video);
    avcodec_free_context(&s->audio);
    sws_freeContext(s->scale);
    if (s->fifo) av_audio_fifo_free(s->fifo);
    if (s->format) {
        s->format->pb = NULL;
        avformat_free_context(s->format);
    }
    if (s->io) { av_freep(&s->io->buffer); avio_context_free(&s->io); }
    av_free(s);
}

static const AVCodec *video_codec(int video) {
    return video == 1 ? avcodec_find_encoder_by_name("libvpx")
         : video == 2 ? avcodec_find_encoder_by_name("libvpx-vp9") : NULL;
}

static const AVCodec *audio_codec(void) {
    const AVCodec *c = avcodec_find_encoder_by_name("libopus");
    return c ? c : avcodec_find_encoder_by_name("opus");
}

// Whether this FFmpeg can record `video` (0 none, 1 VP8, 2 VP9) with or without sound.
int sx_live_supported(int video, int audio) {
    if (video < 0 || video > 2 || (!video && !audio)) return 0;
    if (video && !video_codec(video)) return 0;
    if (audio && !audio_codec()) return 0;
    return av_guess_format("webm", NULL, NULL) != NULL;
}

static int open_stream(struct sx_live *s, AVCodecContext *c, const AVCodec *codec,
                       AVStream **out, AVDictionary **opts) {
    if (s->format->oformat->flags & AVFMT_GLOBALHEADER) c->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    int r = avcodec_open2(c, codec, opts);
    if (r < 0) return r;
    AVStream *st = avformat_new_stream(s->format, NULL);
    if (!st) return AVERROR(ENOMEM);
    st->time_base = c->time_base;
    *out = st;
    return avcodec_parameters_from_context(st->codecpar, c);
}

// `video` 0 (sound only), 1 VP8 or 2 VP9 at width x height (even), about `fps` pictures
// a second at `vbps` bits a second; `rate` 48000 for sound at `abps`, or 0 for none.
struct sx_live *sx_live_open(int video, int width, int height, int fps, int vbps,
                             int rate, int abps, void *opaque, sx_live_write_fn write, int *error) {
    *error = AVERROR(EINVAL);
    if (video < 0 || video > 2 || (!video && !rate) || (rate != 0 && rate != 48000) || !write) return NULL;
    if (video && (width < 16 || height < 16 || (width | height) & 1 || width > 3840 || height > 2160 ||
                  fps < 1 || fps > 60 || vbps < 100000 || vbps > 20000000)) return NULL;
    if (rate && (abps < 6000 || abps > 510000)) return NULL;
    av_log_set_level(AV_LOG_ERROR);
    const AVCodec *vcodec = video ? video_codec(video) : NULL;
    const AVCodec *acodec = rate ? audio_codec() : NULL;
    if ((video && !vcodec) || (rate && !acodec)) { *error = AVERROR(ENOSYS); return NULL; }

    struct sx_live *s = av_mallocz(sizeof *s);
    if (!s) { *error = AVERROR(ENOMEM); return NULL; }
    s->width = width; s->height = height; s->last_ms = -1;
    *error = avformat_alloc_output_context2(&s->format, NULL, "webm", NULL);
    if (*error < 0 || !s->format) goto fail;
    s->format->flags |= AVFMT_FLAG_CUSTOM_IO | AVFMT_FLAG_FLUSH_PACKETS;
    s->packet = av_packet_alloc();
    if (!s->packet) { *error = AVERROR(ENOMEM); goto fail; }

    if (video) {
        s->picture = av_frame_alloc();
        s->video = avcodec_alloc_context3(vcodec);
        if (!s->picture || !s->video) { *error = AVERROR(ENOMEM); goto fail; }
        AVCodecContext *v = s->video;
        v->width = width; v->height = height;
        v->pix_fmt = AV_PIX_FMT_YUV420P;
        // Pictures come when they come (a camera, or a canvas that drew): milliseconds.
        v->time_base = (AVRational){1, 1000};
        v->framerate = (AVRational){fps, 1};
        v->gop_size = 2 * fps; // a key frame every two seconds or so, for seeking
        v->bit_rate = vbps;
        v->thread_count = 1;
        v->color_range = AVCOL_RANGE_MPEG;
        v->colorspace = AVCOL_SPC_BT709;
        v->color_primaries = AVCOL_PRI_BT709;
        v->color_trc = AVCOL_TRC_BT709;
        AVDictionary *vopts = NULL;
        // Realtime: nothing held back for later pictures, the fastest speed.
        av_dict_set(&vopts, "deadline", "realtime", 0);
        av_dict_set(&vopts, "cpu-used", "8", 0);
        av_dict_set(&vopts, "lag-in-frames", "0", 0);
        av_dict_set(&vopts, "auto-alt-ref", "0", 0);
        if (video == 2) { av_dict_set(&vopts, "row-mt", "0", 0); av_dict_set(&vopts, "tile-columns", "0", 0); }
        *error = open_stream(s, v, vcodec, &s->vstream, &vopts);
        av_dict_free(&vopts);
        if (*error < 0) goto fail;
        s->vstream->avg_frame_rate = v->framerate;
        s->picture->format = v->pix_fmt;
        s->picture->width = width; s->picture->height = height;
        *error = av_frame_get_buffer(s->picture, 32);
        if (*error < 0) goto fail;
    }

    if (rate) {
        s->audio = avcodec_alloc_context3(acodec);
        s->sound = av_frame_alloc();
        if (!s->audio || !s->sound) { *error = AVERROR(ENOMEM); goto fail; }
        AVCodecContext *a = s->audio;
        a->sample_rate = rate;
        av_channel_layout_default(&a->ch_layout, 2);
        a->sample_fmt = AV_SAMPLE_FMT_FLTP;
#if LIBAVCODEC_VERSION_INT >= AV_VERSION_INT(61, 13, 100)
        const void *fmts = NULL;
        int nfmts = 0;
        if (avcodec_get_supported_config(NULL, acodec, AV_CODEC_CONFIG_SAMPLE_FORMAT, 0, &fmts, &nfmts) >= 0 && fmts && nfmts > 0)
            a->sample_fmt = ((const enum AVSampleFormat *)fmts)[0];
#else
        if (acodec->sample_fmts) a->sample_fmt = acodec->sample_fmts[0];
#endif
        a->bit_rate = abps;
        a->time_base = (AVRational){1, rate};
        a->strict_std_compliance = FF_COMPLIANCE_EXPERIMENTAL; // FFmpeg's own Opus encoder
        *error = open_stream(s, a, acodec, &s->astream, NULL);
        if (*error < 0) goto fail;
        int fs = a->frame_size ? a->frame_size : 960;
        s->fifo = av_audio_fifo_alloc(a->sample_fmt, 2, fs * 4);
        s->sound->format = a->sample_fmt;
        s->sound->nb_samples = fs;
        s->sound->sample_rate = rate;
        if (!s->fifo) { *error = AVERROR(ENOMEM); goto fail; }
        *error = av_channel_layout_copy(&s->sound->ch_layout, &a->ch_layout);
        if (*error < 0) goto fail;
        *error = av_frame_get_buffer(s->sound, 0);
        if (*error < 0) goto fail;
    }

    unsigned char *buffer = av_malloc(65536);
    if (!buffer) { *error = AVERROR(ENOMEM); goto fail; }
#if LIBAVFORMAT_VERSION_MAJOR < 61 // FFmpeg 6 (host tests on older systems): non-const buffer
    s->io = avio_alloc_context(buffer, 65536, 1, opaque, NULL, (int (*)(void *, uint8_t *, int))write, NULL);
#else
    s->io = avio_alloc_context(buffer, 65536, 1, opaque, NULL, write, NULL);
#endif
    if (!s->io) { av_free(buffer); *error = AVERROR(ENOMEM); goto fail; }
    s->io->seekable = 0;
    s->format->pb = s->io;
    AVDictionary *mopts = NULL;
    // A live stream: no index, sizes left open, clusters of at most a second.
    av_dict_set(&mopts, "live", "1", 0);
    av_dict_set(&mopts, "cluster_time_limit", "1000", 0);
    *error = avformat_write_header(s->format, &mopts);
    av_dict_free(&mopts);
    if (*error < 0) goto fail;
    avio_flush(s->io);
    *error = 0;
    return s;
fail:
    sx_live_free(s);
    return NULL;
}

static int drain(struct sx_live *s, AVCodecContext *c, AVStream *st) {
    int r;
    while ((r = avcodec_receive_packet(c, s->packet)) >= 0) {
        av_packet_rescale_ts(s->packet, c->time_base, st->time_base);
        s->packet->stream_index = st->index;
        r = av_interleaved_write_frame(s->format, s->packet);
        if (r < 0) return r;
    }
    if (r != AVERROR(EAGAIN) && r != AVERROR_EOF) return r;
    return s->io->error < 0 ? s->io->error : 0;
}

// A picture of w x h XRGB pixels (scaled to the recording's size) shown at `at_ms`
// from the start (increasing).
int sx_live_picture(struct sx_live *s, const uint32_t *pixels, int w, int h, int64_t at_ms) {
    if (!s || s->finished || !s->video || !pixels || w < 1 || h < 1 || w > 8192 || h > 8192 || at_ms <= s->last_ms)
        return AVERROR(EINVAL);
    s->scale = sws_getCachedContext(s->scale, w, h, AV_PIX_FMT_BGR0, s->width, s->height, AV_PIX_FMT_YUV420P,
                                    SWS_BILINEAR, NULL, NULL, NULL);
    if (!s->scale) return AVERROR(ENOMEM);
    const int *coef = sws_getCoefficients(SWS_CS_ITU709);
    sws_setColorspaceDetails(s->scale, coef, 1, coef, 0, 0, 1 << 16, 1 << 16);
    int r = av_frame_make_writable(s->picture);
    if (r < 0) return r;
    const uint8_t *src[1] = {(const uint8_t *)pixels};
    int stride[1] = {w * 4};
    sws_scale(s->scale, src, stride, 0, h, s->picture->data, s->picture->linesize);
    s->picture->pts = at_ms;
    r = avcodec_send_frame(s->video, s->picture);
    if (r < 0) return r;
    s->last_ms = at_ms;
    return drain(s, s->video, s->vstream);
}

static int send_sound(struct sx_live *s, int flush) {
    int fs = s->sound->nb_samples;
    while (av_audio_fifo_size(s->fifo) >= fs || (flush && av_audio_fifo_size(s->fifo) > 0)) {
        int r = av_frame_make_writable(s->sound);
        if (r < 0) return r;
        int n = av_audio_fifo_read(s->fifo, (void **)s->sound->data, fs);
        if (n < fs) av_samples_set_silence(s->sound->data, n, fs - n, 2, s->audio->sample_fmt);
        s->sound->pts = s->samples_out;
        s->samples_out += fs;
        r = avcodec_send_frame(s->audio, s->sound);
        if (r < 0) return r;
        r = drain(s, s->audio, s->astream);
        if (r < 0) return r;
    }
    return 0;
}

// `frames` stereo s16 samples at 48 kHz, continuing the sound so far.
int sx_live_sound(struct sx_live *s, const int16_t *pcm, int frames) {
    if (!s || s->finished || !s->audio || !pcm || frames < 0 || frames > 1 << 20) return AVERROR(EINVAL);
    enum AVSampleFormat f = s->audio->sample_fmt;
    float *l = NULL, *rr = NULL, *both = NULL;
    void *planes[2];
    if (f == AV_SAMPLE_FMT_S16) {
        planes[0] = (void *)pcm;
    } else if (f == AV_SAMPLE_FMT_FLTP) {
        l = av_malloc(sizeof(float) * (frames + 1)); rr = av_malloc(sizeof(float) * (frames + 1));
        if (!l || !rr) { av_free(l); av_free(rr); return AVERROR(ENOMEM); }
        for (int i = 0; i < frames; i++) { l[i] = pcm[2 * i] / 32768.0f; rr[i] = pcm[2 * i + 1] / 32768.0f; }
        planes[0] = l; planes[1] = rr;
    } else if (f == AV_SAMPLE_FMT_FLT) {
        both = av_malloc(sizeof(float) * 2 * (frames + 1));
        if (!both) return AVERROR(ENOMEM);
        for (int i = 0; i < 2 * frames; i++) both[i] = pcm[i] / 32768.0f;
        planes[0] = both;
    } else {
        return AVERROR(ENOSYS);
    }
    int r = av_audio_fifo_write(s->fifo, planes, frames);
    av_free(l); av_free(rr); av_free(both);
    if (r < frames) return r < 0 ? r : AVERROR(ENOMEM);
    return send_sound(s, 0);
}

// Ends the cluster being written, so everything encoded so far reaches the caller
// (MediaRecorder's timeslice and requestData).
int sx_live_flush(struct sx_live *s) {
    if (!s || s->finished) return AVERROR(EINVAL);
    int r = av_interleaved_write_frame(s->format, NULL);
    if (r < 0) return r;
    r = av_write_frame(s->format, NULL);
    if (r < 0) return r;
    avio_flush(s->io);
    return s->io->error < 0 ? s->io->error : 0;
}

int sx_live_finish(struct sx_live *s) {
    if (!s || s->finished) return AVERROR(EINVAL);
    s->finished = 1;
    int r;
    if (s->audio) {
        if ((r = send_sound(s, 1)) < 0) return r;
        if ((r = avcodec_send_frame(s->audio, NULL)) < 0) return r;
        if ((r = drain(s, s->audio, s->astream)) < 0) return r;
    }
    if (s->video) {
        if ((r = avcodec_send_frame(s->video, NULL)) < 0) return r;
        if ((r = drain(s, s->video, s->vstream)) < 0) return r;
    }
    r = av_write_trailer(s->format);
    avio_flush(s->io);
    return r < 0 ? r : s->io->error < 0 ? s->io->error : 0;
}
