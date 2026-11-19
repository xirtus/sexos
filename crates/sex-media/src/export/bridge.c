// SPDX-License-Identifier: GPL-3.0-only
// Splicy's export: pictures (XRGB) and stereo sound (s16) in, a finished video file out
// through the caller's write/seek callbacks. Three formats:
//   0  WebM: VP9 (libvpx) + Opus
//   1  MP4:  H.264 (libx264) + AAC
//   2  MP4:  AV1 (libaom or SVT-AV1) + Opus
// Like the recorder, it opens no files and holds no grants; output is the caller's.
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

typedef int (*sx_export_write_fn)(void *, const uint8_t *, int);
typedef int64_t (*sx_export_seek_fn)(void *, int64_t, int);

struct sx_export {
    AVFormatContext *format;
    AVIOContext *io;
    AVCodecContext *video, *audio;
    AVStream *vstream, *astream;
    AVFrame *picture, *sound;
    AVPacket *packet;
    struct SwsContext *scale;
    AVAudioFifo *fifo;
    int width, height, finished;
    int64_t last_frame, samples_out;
};

void sx_export_free(struct sx_export *s) {
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

// The first encoder of `names` that this FFmpeg has.
static const AVCodec *pick(const char *const *names) {
    for (; *names; names++) {
        const AVCodec *c = avcodec_find_encoder_by_name(*names);
        if (c) return c;
    }
    return NULL;
}

static int open_stream(struct sx_export *s, AVCodecContext *c, const AVCodec *codec,
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

struct sx_export *sx_export_open(int format, int width, int height, int fps_num, int fps_den,
                                 int rate, int quality, void *opaque,
                                 sx_export_write_fn write, sx_export_seek_fn seek, int *error) {
    *error = AVERROR(EINVAL);
    if (format < 0 || format > 2 || width < 16 || height < 16 || (width | height) & 1 ||
        width > 7680 || height > 4320 || fps_num < 1 || fps_den < 1 ||
        fps_num / fps_den > 120 || (rate != 0 && rate != 48000) ||
        quality < 0 || quality > 63 || !write || !seek) return NULL;
    av_log_set_level(AV_LOG_ERROR);
    static const char *const vp9[] = {"libvpx-vp9", NULL};
    static const char *const h264[] = {"libx264", NULL};
    static const char *const av1[] = {"libaom-av1", "libsvtav1", "librav1e", NULL};
    static const char *const opus[] = {"libopus", "opus", NULL};
    static const char *const aac[] = {"aac", NULL};
    const char *const *vnames = format == 0 ? vp9 : format == 1 ? h264 : av1;
    const char *const *anames = format == 1 ? aac : opus;
    const AVCodec *vcodec = pick(vnames);
    const AVCodec *acodec = rate ? pick(anames) : NULL;
    if (!vcodec || (rate && !acodec)) { *error = AVERROR(ENOSYS); return NULL; }

    struct sx_export *s = av_mallocz(sizeof *s);
    if (!s) { *error = AVERROR(ENOMEM); return NULL; }
    s->width = width; s->height = height; s->last_frame = -1;
    *error = avformat_alloc_output_context2(&s->format, NULL, format == 0 ? "webm" : "mp4", NULL);
    if (*error < 0 || !s->format) goto fail;
    s->format->flags |= AVFMT_FLAG_CUSTOM_IO;
    s->packet = av_packet_alloc();
    s->picture = av_frame_alloc();
    s->video = avcodec_alloc_context3(vcodec);
    if (!s->packet || !s->picture || !s->video) { *error = AVERROR(ENOMEM); goto fail; }

    AVCodecContext *v = s->video;
    v->width = width; v->height = height;
    v->pix_fmt = AV_PIX_FMT_YUV420P;
    v->time_base = (AVRational){fps_den, fps_num};
    v->framerate = (AVRational){fps_num, fps_den};
    v->gop_size = 2 * fps_num / fps_den; // a key frame every two seconds, for seeking
    v->thread_count = 1;
    v->color_range = AVCOL_RANGE_MPEG;
    v->colorspace = AVCOL_SPC_BT709;
    v->color_primaries = AVCOL_PRI_BT709;
    v->color_trc = AVCOL_TRC_BT709;
    AVDictionary *vopts = NULL;
    char q[8];
    snprintf(q, sizeof q, "%d", quality ? quality : (format == 1 ? 20 : 32));
    av_dict_set(&vopts, "crf", q, 0);
    if (format == 0) { av_dict_set(&vopts, "b", "0", 0); av_dict_set(&vopts, "row-mt", "0", 0); av_dict_set(&vopts, "deadline", "good", 0); av_dict_set(&vopts, "cpu-used", "4", 0); }
    if (format == 1) { av_dict_set(&vopts, "preset", "medium", 0); av_dict_set(&vopts, "threads", "1", 0); }
    if (format == 2) { av_dict_set(&vopts, "cpu-used", "6", 0); av_dict_set(&vopts, "row-mt", "0", 0); av_dict_set(&vopts, "preset", "8", 0); }
    *error = open_stream(s, v, vcodec, &s->vstream, &vopts);
    av_dict_free(&vopts);
    if (*error < 0) goto fail;
    s->vstream->avg_frame_rate = v->framerate;

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
        a->bit_rate = 160000;
        a->time_base = (AVRational){1, rate};
        a->strict_std_compliance = FF_COMPLIANCE_EXPERIMENTAL; // FFmpeg's own Opus encoder
        *error = open_stream(s, a, acodec, &s->astream, NULL);
        if (*error < 0) goto fail;
        int fs = a->frame_size ? a->frame_size : 1024;
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

    s->scale = sws_getContext(width, height, AV_PIX_FMT_BGR0, width, height, AV_PIX_FMT_YUV420P,
                              SWS_BICUBIC | SWS_ACCURATE_RND | SWS_FULL_CHR_H_INT, NULL, NULL, NULL);
    if (!s->scale) { *error = AVERROR(ENOMEM); goto fail; }
    const int *coef = sws_getCoefficients(SWS_CS_ITU709);
    sws_setColorspaceDetails(s->scale, coef, 1, coef, 0, 0, 1 << 16, 1 << 16);
    s->picture->format = v->pix_fmt;
    s->picture->width = width; s->picture->height = height;
    *error = av_frame_get_buffer(s->picture, 32);
    if (*error < 0) goto fail;

    unsigned char *buffer = av_malloc(65536);
    if (!buffer) { *error = AVERROR(ENOMEM); goto fail; }
#if LIBAVFORMAT_VERSION_MAJOR < 61 // FFmpeg 6 (host tests on older systems): non-const buffer
    s->io = avio_alloc_context(buffer, 65536, 1, opaque, NULL, (int (*)(void *, uint8_t *, int))write, seek);
#else
    s->io = avio_alloc_context(buffer, 65536, 1, opaque, NULL, write, seek);
#endif
    if (!s->io) { av_free(buffer); *error = AVERROR(ENOMEM); goto fail; }
    s->io->seekable = AVIO_SEEKABLE_NORMAL;
    s->format->pb = s->io;
    *error = avformat_write_header(s->format, NULL);
    if (*error < 0) goto fail;
    *error = 0;
    return s;
fail:
    sx_export_free(s);
    return NULL;
}

static int drain(struct sx_export *s, AVCodecContext *c, AVStream *st) {
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

// Picture `frame` of the export (0, 1, 2, ...; gaps allowed), width*height XRGB pixels.
int sx_export_picture(struct sx_export *s, const uint32_t *pixels, int64_t frame) {
    if (!s || s->finished || !pixels || frame <= s->last_frame) return AVERROR(EINVAL);
    int r = av_frame_make_writable(s->picture);
    if (r < 0) return r;
    const uint8_t *src[1] = {(const uint8_t *)pixels};
    int stride[1] = {s->width * 4};
    sws_scale(s->scale, src, stride, 0, s->height, s->picture->data, s->picture->linesize);
    s->picture->pts = frame;
    r = avcodec_send_frame(s->video, s->picture);
    if (r < 0) return r;
    s->last_frame = frame;
    return drain(s, s->video, s->vstream);
}

static int send_sound(struct sx_export *s, int flush) {
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

// `frames` stereo s16 samples, continuing the sound so far.
int sx_export_sound(struct sx_export *s, const int16_t *pcm, int frames) {
    if (!s || s->finished || !s->audio || !pcm || frames < 0 || frames > 1 << 20) return AVERROR(EINVAL);
    enum AVSampleFormat f = s->audio->sample_fmt;
    float *l = NULL, *rr = NULL, *both = NULL;
    void *planes[2];
    if (f == AV_SAMPLE_FMT_S16) {
        planes[0] = (void *)pcm;
    } else if (f == AV_SAMPLE_FMT_FLTP) {
        l = av_malloc(sizeof(float) * frames); rr = av_malloc(sizeof(float) * frames);
        if (!l || !rr) { av_free(l); av_free(rr); return AVERROR(ENOMEM); }
        for (int i = 0; i < frames; i++) { l[i] = pcm[2 * i] / 32768.0f; rr[i] = pcm[2 * i + 1] / 32768.0f; }
        planes[0] = l; planes[1] = rr;
    } else if (f == AV_SAMPLE_FMT_FLT) {
        both = av_malloc(sizeof(float) * 2 * frames);
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

int sx_export_finish(struct sx_export *s) {
    if (!s || s->finished) return AVERROR(EINVAL);
    s->finished = 1;
    int r;
    if (s->audio) {
        if ((r = send_sound(s, 1)) < 0) return r;
        if ((r = avcodec_send_frame(s->audio, NULL)) < 0) return r;
        if ((r = drain(s, s->audio, s->astream)) < 0) return r;
    }
    if ((r = avcodec_send_frame(s->video, NULL)) < 0) return r;
    if ((r = drain(s, s->video, s->vstream)) < 0) return r;
    r = av_write_trailer(s->format);
    avio_flush(s->io);
    return r < 0 ? r : s->io->error < 0 ? s->io->error : 0;
}
