// SPDX-License-Identifier: GPL-3.0-only
// Play's bridge to FFmpeg: open a stream through callbacks, decode its video, and
// scale each frame into the caller's 0x00RRGGBB pixels. Rust sees only these
// three calls and plain structs, never FFmpeg's own structures.
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libswscale/swscale.h>
#include <libavutil/imgutils.h>
#include <string.h>

typedef int (*sx_read_fn)(void *opaque, uint8_t *buf, int size);
typedef int64_t (*sx_seek_fn)(void *opaque, int64_t offset, int whence);

struct sx_info {
    int width, height;
    int fps_num, fps_den;
    int64_t duration_ms;
    char codec[16];
};

struct sx {
    AVFormatContext *format;
    AVIOContext *io;
    AVCodecContext *codec;
    AVPacket *packet;
    AVFrame *frame;
    struct SwsContext *scale;
    int stream;
    int draining;
    int scale_w, scale_h, scale_from_w, scale_from_h, scale_fmt;
};

void sx_close(struct sx *s);

struct sx *sx_open(void *opaque, sx_read_fn rd, sx_seek_fn sk, struct sx_info *info, int *err) {
    av_log_set_level(AV_LOG_ERROR);
    struct sx *s = av_mallocz(sizeof *s);
    if (!s) { *err = AVERROR(ENOMEM); return NULL; }
    unsigned char *buf = av_malloc(32768);
    s->io = buf ? avio_alloc_context(buf, 32768, 0, opaque, rd, NULL, sk) : NULL;
    s->format = avformat_alloc_context();
    if (!s->io || !s->format) { *err = AVERROR(ENOMEM); sx_close(s); return NULL; }
    s->format->pb = s->io;
    s->format->flags |= AVFMT_FLAG_CUSTOM_IO;
    if ((*err = avformat_open_input(&s->format, NULL, NULL, NULL)) < 0) { s->format = NULL; sx_close(s); return NULL; }
    if ((*err = avformat_find_stream_info(s->format, NULL)) < 0) { sx_close(s); return NULL; }
    const AVCodec *decoder = NULL;
    s->stream = av_find_best_stream(s->format, AVMEDIA_TYPE_VIDEO, -1, -1, &decoder, 0);
    if (s->stream < 0 || !decoder) { *err = s->stream < 0 ? s->stream : AVERROR_DECODER_NOT_FOUND; sx_close(s); return NULL; }
    AVStream *st = s->format->streams[s->stream];
    s->codec = avcodec_alloc_context3(decoder);
    if (!s->codec) { *err = AVERROR(ENOMEM); sx_close(s); return NULL; }
    if ((*err = avcodec_parameters_to_context(s->codec, st->codecpar)) < 0 ||
        (*err = avcodec_open2(s->codec, decoder, NULL)) < 0) { sx_close(s); return NULL; }
    s->packet = av_packet_alloc();
    s->frame = av_frame_alloc();
    if (!s->packet || !s->frame) { *err = AVERROR(ENOMEM); sx_close(s); return NULL; }
    AVRational rate = st->avg_frame_rate.num ? st->avg_frame_rate : st->r_frame_rate;
    info->width = s->codec->width;
    info->height = s->codec->height;
    info->fps_num = rate.num;
    info->fps_den = rate.den;
    info->duration_ms = s->format->duration > 0 ? s->format->duration / 1000 : 0;
    strncpy(info->codec, decoder->name, sizeof info->codec - 1);
    info->codec[sizeof info->codec - 1] = 0;
    *err = 0;
    return s;
}

// Decodes the next frame and scales it to out_w x out_h into `out` (row-major,
// out_w pixels per row). Returns 1 for a frame, 0 at the end, or an AVERROR.
int sx_next(struct sx *s, uint32_t *out, int out_w, int out_h) {
    for (;;) {
        int r = avcodec_receive_frame(s->codec, s->frame);
        if (r == 0) break;
        if (r == AVERROR_EOF) return 0;
        if (r != AVERROR(EAGAIN)) return r;
        if (s->draining) return 0;
        r = av_read_frame(s->format, s->packet);
        if (r == AVERROR_EOF) {
            s->draining = 1;
            avcodec_send_packet(s->codec, NULL);
            continue;
        }
        if (r < 0) return r;
        if (s->packet->stream_index == s->stream) r = avcodec_send_packet(s->codec, s->packet);
        av_packet_unref(s->packet);
        if (r < 0 && r != AVERROR(EAGAIN)) return r;
    }
    AVFrame *f = s->frame;
    if (!s->scale || s->scale_w != out_w || s->scale_h != out_h || s->scale_from_w != f->width ||
        s->scale_from_h != f->height || s->scale_fmt != f->format) {
        sws_freeContext(s->scale);
        s->scale = sws_getContext(f->width, f->height, f->format, out_w, out_h, AV_PIX_FMT_BGR0,
                                  SWS_BILINEAR, NULL, NULL, NULL);
        if (!s->scale) return AVERROR(EINVAL);
        s->scale_w = out_w; s->scale_h = out_h;
        s->scale_from_w = f->width; s->scale_from_h = f->height; s->scale_fmt = f->format;
    }
    uint8_t *dst[4] = { (uint8_t *)out, NULL, NULL, NULL };
    int stride[4] = { out_w * 4, 0, 0, 0 };
    sws_scale(s->scale, (const uint8_t *const *)f->data, f->linesize, 0, f->height, dst, stride);
    av_frame_unref(f);
    return 1;
}

void sx_close(struct sx *s) {
    if (!s) return;
    sws_freeContext(s->scale);
    av_frame_free(&s->frame);
    av_packet_free(&s->packet);
    avcodec_free_context(&s->codec);
    if (s->format) avformat_close_input(&s->format);
    if (s->io) { av_freep(&s->io->buffer); avio_context_free(&s->io); }
    av_free(s);
}

// FFmpeg's error text, for the window.
void sx_error(int err, char *buf, int len) { av_strerror(err, buf, len); }
