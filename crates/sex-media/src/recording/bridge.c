// SPDX-License-Identifier: GPL-3.0-only
// FFV1/Matroska inside the caller's domain. Output is an owned bounded queue;
// neither callback opens files, waits for IPC, nor receives a framebuffer grant.
#include <stdint.h>
#include <string.h>
#include <errno.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/mem.h>
#include <libavutil/opt.h>

typedef int (*sx_record_write_fn)(void *, const uint8_t *, int);
typedef int64_t (*sx_record_seek_fn)(void *, int64_t, int);
struct sx_record {
    AVFormatContext *format;
    AVCodecContext *codec;
    AVIOContext *io;
    AVFrame *frame;
    AVPacket *packet;
    AVStream *stream;
    int width, height, fps, finished;
    int64_t last;
};

void sx_record_free(struct sx_record *s) {
    if (!s) return;
    av_packet_free(&s->packet);
    av_frame_free(&s->frame);
    avcodec_free_context(&s->codec);
    if (s->format) {
        s->format->pb = NULL; // custom IO is ours, even on an aborted container
        avformat_free_context(s->format);
    }
    if (s->io) { av_freep(&s->io->buffer); avio_context_free(&s->io); }
    av_free(s);
}

struct sx_record *sx_record_open(int width, int height, int fps, void *opaque,
                               sx_record_write_fn write, sx_record_seek_fn seek, int *error) {
    *error = AVERROR(EINVAL);
    if (width < 1 || height < 1 || width > 8192 || height > 4320 ||
        (uint64_t)width * height > 4194304 || fps < 1 || fps > 60 || !write || !seek) return NULL;
    av_log_set_level(AV_LOG_ERROR);
    const AVCodec *codec = avcodec_find_encoder(AV_CODEC_ID_FFV1);
    if (!codec) { *error = AVERROR(ENOSYS); return NULL; }
    struct sx_record *s = av_mallocz(sizeof *s);
    if (!s) { *error = AVERROR(ENOMEM); return NULL; }
    s->width = width; s->height = height; s->fps = fps; s->last = AV_NOPTS_VALUE;
    *error = avformat_alloc_output_context2(&s->format, NULL, "matroska", NULL);
    if (*error < 0 || !s->format) goto fail;
    s->format->flags |= AVFMT_FLAG_CUSTOM_IO | AVFMT_FLAG_BITEXACT;
    s->codec = avcodec_alloc_context3(codec);
    s->frame = av_frame_alloc(); s->packet = av_packet_alloc();
    s->stream = avformat_new_stream(s->format, NULL);
    if (!s->codec || !s->frame || !s->packet || !s->stream) { *error = AVERROR(ENOMEM); goto fail; }
    s->codec->width = width; s->codec->height = height;
    s->codec->pix_fmt = AV_PIX_FMT_BGR0;
    s->codec->color_range = AVCOL_RANGE_JPEG;
    s->codec->color_primaries = AVCOL_PRI_BT709;
    s->codec->color_trc = AVCOL_TRC_IEC61966_2_1;
    s->codec->colorspace = AVCOL_SPC_RGB;
    s->codec->time_base = (AVRational){1, 1000};
    s->codec->framerate = (AVRational){fps, 1};
    s->codec->gop_size = 1; s->codec->level = 3;
    s->codec->thread_count = 1;
    s->codec->max_pixels = 4194304;
    s->codec->flags |= AV_CODEC_FLAG_BITEXACT;
    if (s->format->oformat->flags & AVFMT_GLOBALHEADER) s->codec->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    *error = av_opt_set_int(s->codec->priv_data, "slicecrc", 1, 0);
    if (*error < 0) goto fail;
    *error = avcodec_open2(s->codec, codec, NULL);
    if (*error < 0) goto fail;
    s->stream->time_base = s->codec->time_base;
    s->stream->avg_frame_rate = s->codec->framerate;
    *error = avcodec_parameters_from_context(s->stream->codecpar, s->codec);
    if (*error < 0) goto fail;
    unsigned char *buffer = av_malloc(32768);
    if (!buffer) { *error = AVERROR(ENOMEM); goto fail; }
    s->io = avio_alloc_context(buffer, 32768, 1, opaque, NULL, write, seek);
    if (!s->io) { av_free(buffer); *error = AVERROR(ENOMEM); goto fail; }
    s->io->seekable = AVIO_SEEKABLE_NORMAL;
    s->format->pb = s->io;
    s->frame->format = s->codec->pix_fmt;
    s->frame->width = width; s->frame->height = height;
    *error = av_frame_get_buffer(s->frame, 32);
    if (*error < 0) goto fail;
    *error = avformat_write_header(s->format, NULL);
    if (*error < 0) goto fail;
    avio_flush(s->io);
    if (s->io->error < 0) { *error = s->io->error; goto fail; }
    *error = 0;
    return s;
fail:
    sx_record_free(s);
    return NULL;
}

static int sx_record_packets(struct sx_record *s) {
    int r;
    while ((r = avcodec_receive_packet(s->codec, s->packet)) >= 0) {
        if (s->packet->size < 0 || s->packet->size > 24 * 1024 * 1024) {
            av_packet_unref(s->packet); return AVERROR(ENOSPC);
        }
        if (s->packet->duration == 0) s->packet->duration = 1000 / s->fps;
        av_packet_rescale_ts(s->packet, s->codec->time_base, s->stream->time_base);
        s->packet->stream_index = s->stream->index;
        r = av_write_frame(s->format, s->packet);
        av_packet_unref(s->packet);
        if (r < 0) return r;
    }
    if (r != AVERROR(EAGAIN) && r != AVERROR_EOF) return r;
    // Matroska advertises AVFMT_ALLOW_FLUSH. Keep one operation's output
    // bounded instead of accumulating an unbounded in-memory cluster.
    r = av_write_frame(s->format, NULL);
    avio_flush(s->io);
    return r < 0 ? r : s->io->error < 0 ? s->io->error : 0;
}

int sx_record_frame(struct sx_record *s, const uint32_t *pixels, int64_t pts) {
    if (!s || s->finished || !pixels || pts < 0 || pts > 600000 ||
        (s->last != AV_NOPTS_VALUE && pts <= s->last)) return AVERROR(EINVAL);
    int r = av_frame_make_writable(s->frame);
    if (r < 0) return r;
    for (int y = 0; y < s->height; y++)
        memcpy(s->frame->data[0] + (size_t)y * s->frame->linesize[0],
               pixels + (size_t)y * s->width, (size_t)s->width * 4);
    s->frame->pts = pts;
    r = avcodec_send_frame(s->codec, s->frame);
    if (r < 0) return r;
    s->last = pts;
    return sx_record_packets(s);
}

int sx_record_finish(struct sx_record *s) {
    if (!s || s->finished) return AVERROR(EINVAL);
    s->finished = 1;
    int r = avcodec_send_frame(s->codec, NULL);
    if (r < 0 && r != AVERROR_EOF) return r;
    r = sx_record_packets(s);
    if (r < 0) return r;
    r = av_write_trailer(s->format);
    avio_flush(s->io);
    return r < 0 ? r : s->io->error < 0 ? s->io->error : 0;
}
