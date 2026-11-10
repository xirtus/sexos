// SPDX-License-Identifier: GPL-3.0-only
// sex-media's bridge to FFmpeg: open a stream through callbacks, decode its video and its
// sound, scale each picture into the caller's 0x00RRGGBB pixels and turn sound into
// 48 kHz stereo i16 frames. Rust sees only these calls and plain structs, never
// FFmpeg's own structures.
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libswscale/swscale.h>
#include <libswresample/swresample.h>
#include <libavutil/imgutils.h>
#include <libavutil/channel_layout.h>
#include <string.h>

typedef int (*sx_read_fn)(void *opaque, uint8_t *buf, int size);
typedef int64_t (*sx_seek_fn)(void *opaque, int64_t offset, int whence);

struct sx_info {
    int width, height;          // 0 when there is no picture
    int fps_num, fps_den;
    int64_t duration_ms;
    char codec[16];
    int sample_rate, channels;  // 0 when there is no sound
    char audio_codec[16];
    char format[16];
};

struct decoder {
    AVCodecContext *codec;
    int stream;
    int done;                   // the decoder has given its last frame
};

struct sx {
    AVFormatContext *format;
    AVIOContext *io;
    struct decoder video, audio;
    AVPacket *packet;
    AVFrame *frame;
    struct SwsContext *scale;
    SwrContext *resample;
    int draining;
    int absolute;               // times are the stream's own (Media Source), not from its start
    int scale_w, scale_h, scale_from_w, scale_from_h, scale_fmt;
};

void sx_close(struct sx *s);

static int open_decoder(struct sx *s, enum AVMediaType type, struct decoder *d, const char **name) {
    const AVCodec *codec = NULL;
    d->stream = av_find_best_stream(s->format, type, -1, -1, &codec, 0);
    d->done = 1;
    if (d->stream < 0 || !codec) { d->stream = -1; return 0; }
    AVStream *st = s->format->streams[d->stream];
    d->codec = avcodec_alloc_context3(codec);
    if (!d->codec) return AVERROR(ENOMEM);
    int r = avcodec_parameters_to_context(d->codec, st->codecpar);
    if (r >= 0) { d->codec->pkt_timebase = st->time_base; r = avcodec_open2(d->codec, codec, NULL); }
    if (r < 0) { avcodec_free_context(&d->codec); d->stream = -1; return r; }
    d->done = 0;
    *name = codec->name;
    return 0;
}

static void copy_name(char *to, const char *from) {
    strncpy(to, from ? from : "", 15);
    to[15] = 0;
}

struct sx *sx_open(void *opaque, sx_read_fn rd, sx_seek_fn sk, struct sx_info *info, int *err, int growing) {
    av_log_set_level(AV_LOG_ERROR);
    memset(info, 0, sizeof *info);
    struct sx *s = av_mallocz(sizeof *s);
    if (!s) { *err = AVERROR(ENOMEM); return NULL; }
    s->video.stream = s->audio.stream = -1;
    // A growing stream reads in small steps, so the read position (which the player
    // checks against what has arrived) stays close to what the demuxer has taken.
    int io_size = growing ? 16384 : 65536;
    unsigned char *buf = av_malloc(io_size);
    s->io = buf ? avio_alloc_context(buf, io_size, 0, opaque, rd, NULL, sk) : NULL;
    s->format = avformat_alloc_context();
    if (!s->io || !s->format) { *err = AVERROR(ENOMEM); sx_close(s); return NULL; }
    s->format->pb = s->io;
    s->format->flags |= AVFMT_FLAG_CUSTOM_IO;
    // Read a growing stream in order, as from a pipe: then the MP4 parser takes one
    // fragment at a time instead of indexing every fragment up to the last byte.
    if (growing) s->io->seekable = 0;
    // A Media Source stream may start anywhere (a seek, a live edge): its times are
    // presentation times as they stand, not counted from its first packet.
    s->absolute = growing;
    if ((*err = avformat_open_input(&s->format, NULL, NULL, NULL)) < 0) { s->format = NULL; sx_close(s); return NULL; }
    // A stream still arriving (Media Source, a download: Source::growing) is not probed further: probing
    // reads ahead until it runs into bytes that have not come yet, and the container
    // parser loses its place there. Its header (an fMP4 moov) names the codecs.
    if (!growing && (*err = avformat_find_stream_info(s->format, NULL)) < 0) { sx_close(s); return NULL; }
    const char *vname = NULL, *aname = NULL;
    if ((*err = open_decoder(s, AVMEDIA_TYPE_VIDEO, &s->video, &vname)) < 0) { sx_close(s); return NULL; }
    if ((*err = open_decoder(s, AVMEDIA_TYPE_AUDIO, &s->audio, &aname)) < 0) { sx_close(s); return NULL; }
    if (s->video.stream < 0 && s->audio.stream < 0) { *err = AVERROR_DECODER_NOT_FOUND; sx_close(s); return NULL; }
    s->packet = av_packet_alloc();
    s->frame = av_frame_alloc();
    if (!s->packet || !s->frame) { *err = AVERROR(ENOMEM); sx_close(s); return NULL; }
    if (s->video.stream >= 0) {
        AVStream *st = s->format->streams[s->video.stream];
        AVRational rate = st->avg_frame_rate.num ? st->avg_frame_rate : st->r_frame_rate;
        info->width = s->video.codec->width;
        info->height = s->video.codec->height;
        info->fps_num = rate.num;
        info->fps_den = rate.den;
        copy_name(info->codec, vname);
    }
    if (s->audio.stream >= 0) {
        info->sample_rate = s->audio.codec->sample_rate;
        info->channels = s->audio.codec->ch_layout.nb_channels;
        copy_name(info->audio_codec, aname);
    }
    copy_name(info->format, s->format->iformat->name);
    info->duration_ms = s->format->duration > 0 ? s->format->duration / 1000 : 0;
    *err = 0;
    return s;
}

// Times are measured from the file's start, shared by every stream, so a sound track
// that begins after the pictures keeps its offset. A growing stream keeps its own times.
static int64_t pts_ms(struct sx *s, int stream, AVFrame *f) {
    int64_t t = f->best_effort_timestamp;
    if (t == AV_NOPTS_VALUE) return -1;
    AVStream *st = s->format->streams[stream];
    int64_t ms = av_rescale_q(t, st->time_base, (AVRational){1, 1000});
    if (!s->absolute && s->format->start_time != AV_NOPTS_VALUE) ms -= s->format->start_time / 1000;
    return ms;
}

static int picture(struct sx *s, uint32_t *out, int out_w, int out_h) {
    AVFrame *f = s->frame;
    if (!s->scale || s->scale_w != out_w || s->scale_h != out_h || s->scale_from_w != f->width ||
        s->scale_from_h != f->height || s->scale_fmt != f->format) {
        sws_freeContext(s->scale);
        s->scale = sws_getContext(f->width, f->height, f->format, out_w, out_h, AV_PIX_FMT_BGRA,
                                  SWS_BILINEAR, NULL, NULL, NULL);
        if (!s->scale) return AVERROR(EINVAL);
        s->scale_w = out_w; s->scale_h = out_h;
        s->scale_from_w = f->width; s->scale_from_h = f->height; s->scale_fmt = f->format;
    }
    uint8_t *dst[4] = { (uint8_t *)out, NULL, NULL, NULL };
    int stride[4] = { out_w * 4, 0, 0, 0 };
    sws_scale(s->scale, (const uint8_t *const *)f->data, f->linesize, 0, f->height, dst, stride);
    return 1;
}

static int sound(struct sx *s, int16_t *pcm, int cap, int *frames) {
    AVFrame *f = s->frame;
    if (!s->resample) {
        AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
        int r = swr_alloc_set_opts2(&s->resample, &stereo, AV_SAMPLE_FMT_S16, 48000,
                                    &f->ch_layout, f->format, f->sample_rate, 0, NULL);
        if (r < 0 || (r = swr_init(s->resample)) < 0) return r < 0 ? r : AVERROR(EINVAL);
    }
    uint8_t *dst[1] = { (uint8_t *)pcm };
    int n = swr_convert(s->resample, dst, cap, (const uint8_t **)f->extended_data, f->nb_samples);
    if (n < 0) return n;
    *frames = n;
    return 2;
}

// The next thing to present. Returns 1 for a picture scaled to out_w x out_h into
// `out`, 2 for `*frames` frames of 48 kHz stereo i16 sound in `pcm` (room for `cap`),
// 0 at the end, or an AVERROR. `*at_ms` is its time from the stream's start, or -1.
// A growing stream that ran out of bytes ends here too (its decoders give up every
// frame they hold); the caller may open again on what follows.
int sx_next(struct sx *s, uint32_t *out, int out_w, int out_h, int16_t *pcm, int cap, int *frames, int64_t *at_ms) {
    *frames = 0;
    for (;;) {
        // Sound first: it is small, and its ring must never run dry while pictures wait.
        struct decoder *order[2] = { &s->audio, &s->video };
        for (int i = 0; i < 2; i++) {
            struct decoder *d = order[i];
            if (d->done) continue;
            int r = avcodec_receive_frame(d->codec, s->frame);
            if (r == AVERROR_EOF) { d->done = 1; continue; }
            if (r == AVERROR(EAGAIN)) continue;
            if (r < 0) return r;
            *at_ms = pts_ms(s, d->stream, s->frame);
            r = d == &s->video ? picture(s, out, out_w, out_h) : sound(s, pcm, cap, frames);
            av_frame_unref(s->frame);
            return r;
        }
        if (s->audio.done && s->video.done) return 0;
        // Both decoders were told the end and neither has a frame left.
        if (s->draining) return 0;
        // A growing source (Media Source) answers EIO for bytes that have not arrived
        // yet: forget it and read again, now that more may have come.
        if (s->io->error == AVERROR(EIO) || s->io->eof_reached) { s->io->error = 0; s->io->eof_reached = 0; }
        int r = av_read_frame(s->format, s->packet);
        // A download read where its bytes have not come yet (a seek ahead): not the
        // end; the caller waits for them and asks again.
        if (r == AVERROR(EIO) && !s->absolute) return 3;
        if (r == AVERROR_EOF || r == AVERROR(EIO)) {
            s->draining = 1;
            if (!s->video.done) avcodec_send_packet(s->video.codec, NULL);
            if (!s->audio.done) avcodec_send_packet(s->audio.codec, NULL);
            continue;
        }
        if (r < 0) return r;
        struct decoder *d = s->packet->stream_index == s->video.stream ? &s->video
                          : s->packet->stream_index == s->audio.stream ? &s->audio : NULL;
        if (d && !d->done) r = avcodec_send_packet(d->codec, s->packet);
        av_packet_unref(s->packet);
        // A damaged packet costs its own frame, not the whole film.
        if (r < 0 && r != AVERROR(EAGAIN) && r != AVERROR_INVALIDDATA) return r;
    }
}

// Go to `ms` from the file's start: the decoders restart at the key frame at or
// before it. Returns 0 or an AVERROR.
int sx_seek(struct sx *s, int64_t ms) {
    int64_t ts = av_rescale(ms < 0 ? 0 : ms, AV_TIME_BASE, 1000);
    if (s->format->start_time != AV_NOPTS_VALUE) ts += s->format->start_time;
    int r = avformat_seek_file(s->format, -1, INT64_MIN, ts, ts, 0);
    if (r < 0) return r;
    struct decoder *ds[2] = { &s->video, &s->audio };
    for (int i = 0; i < 2; i++) {
        if (!ds[i]->codec) continue;
        avcodec_flush_buffers(ds[i]->codec);
        ds[i]->done = 0;
    }
    if (s->resample) swr_init(s->resample);
    s->draining = 0;
    return 0;
}

void sx_close(struct sx *s) {
    if (!s) return;
    sws_freeContext(s->scale);
    swr_free(&s->resample);
    av_frame_free(&s->frame);
    av_packet_free(&s->packet);
    avcodec_free_context(&s->video.codec);
    avcodec_free_context(&s->audio.codec);
    if (s->format) avformat_close_input(&s->format);
    if (s->io) { av_freep(&s->io->buffer); avio_context_free(&s->io); }
    av_free(s);
}

// FFmpeg's error text, for the window.
void sx_error(int err, char *buf, int len) { av_strerror(err, buf, len); }
