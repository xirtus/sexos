// SPDX-License-Identifier: GPL-3.0-only
// Audio codecs and muxers execute in the exporting app. Custom AVIO callbacks
// enqueue bounded writes only: no ambient paths, libc files or IPC from C.
#include <stdint.h>
#include <string.h>
#include <errno.h>
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/mem.h>
#include <libavutil/opt.h>
#include <libavutil/samplefmt.h>

typedef int (*moan_write_fn)(void *, const uint8_t *, int);
typedef int64_t (*moan_seek_fn)(void *, int64_t, int);
struct moan_encoder {
    AVFormatContext *format;
    AVCodecContext *codec;
    AVIOContext *io;
    AVStream *stream;
    AVFrame *frame;
    AVPacket *packet;
    int capacity, finished;
    int64_t accepted, expected;
};
void moan_encoder_free(struct moan_encoder *s) {
    if (!s) return;
    av_packet_free(&s->packet);
    av_frame_free(&s->frame);
    avcodec_free_context(&s->codec);
    if (s->format) { s->format->pb = NULL; avformat_free_context(s->format); }
    if (s->io) { av_freep(&s->io->buffer); avio_context_free(&s->io); }
    av_free(s);
}
struct moan_encoder *moan_encoder_open(int format, int64_t expected, void *opaque,
        moan_write_fn write, moan_seek_fn seek, int *capacity, int *error) {
    *error = AVERROR(EINVAL); *capacity = 0;
    if (format < 0 || format > 2 || expected < 1 || expected > 3662LL * 48000 || !write || !seek) return NULL;
    static const char *names[] = { "pcm_s16le", "flac", "libshine" };
    static const char *muxers[] = { "wav", "flac", "mp3" };
    const AVCodec *codec = avcodec_find_encoder_by_name(names[format]);
    if (!codec) { *error = AVERROR(ENOSYS); return NULL; }
    struct moan_encoder *s = av_mallocz(sizeof *s);
    if (!s) { *error = AVERROR(ENOMEM); return NULL; }
    s->expected = expected;
    av_log_set_level(AV_LOG_ERROR);
    *error = avformat_alloc_output_context2(&s->format, NULL, muxers[format], NULL);
    if (*error < 0 || !s->format) goto fail;
    s->format->flags |= AVFMT_FLAG_CUSTOM_IO | AVFMT_FLAG_BITEXACT;
    s->codec = avcodec_alloc_context3(codec);
    s->frame = av_frame_alloc(); s->packet = av_packet_alloc();
    s->stream = avformat_new_stream(s->format, NULL);
    if (!s->codec || !s->frame || !s->packet || !s->stream) { *error = AVERROR(ENOMEM); goto fail; }
    s->codec->sample_rate = 48000;
    s->codec->sample_fmt = format == 2 ? AV_SAMPLE_FMT_S16P : AV_SAMPLE_FMT_S16;
    s->codec->time_base = (AVRational){1, 48000};
    s->codec->ch_layout = (AVChannelLayout)AV_CHANNEL_LAYOUT_STEREO;
    s->codec->thread_count = 1;
    s->codec->flags |= AV_CODEC_FLAG_BITEXACT;
    s->codec->bit_rate = format == 2 ? 192000 : 0;
    if (s->format->oformat->flags & AVFMT_GLOBALHEADER) s->codec->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
    *error = avcodec_open2(s->codec, codec, NULL);
    if (*error < 0) goto fail;
    s->capacity = s->codec->frame_size > 0 ? s->codec->frame_size : 1024;
    if (s->capacity > 8192) { *error = AVERROR(EINVAL); goto fail; }
    s->stream->time_base = s->codec->time_base;
    s->stream->duration = expected;
    *error = avcodec_parameters_from_context(s->stream->codecpar, s->codec);
    if (*error < 0) goto fail;
    unsigned char *buffer = av_malloc(32768);
    if (!buffer) { *error = AVERROR(ENOMEM); goto fail; }
    s->io = avio_alloc_context(buffer, 32768, 1, opaque, NULL, write, seek);
    if (!s->io) { av_free(buffer); *error = AVERROR(ENOMEM); goto fail; }
    s->io->seekable = AVIO_SEEKABLE_NORMAL;
    s->format->pb = s->io;
    s->frame->format = s->codec->sample_fmt;
    s->frame->sample_rate = 48000;
    *error = av_channel_layout_copy(&s->frame->ch_layout, &s->codec->ch_layout);
    if (*error < 0) goto fail;
    s->frame->nb_samples = s->capacity;
    *error = av_frame_get_buffer(s->frame, 0);
    if (*error < 0) goto fail;
    *error = avformat_write_header(s->format, NULL);
    if (*error < 0) goto fail;
    avio_flush(s->io);
    if (s->io->error < 0) { *error = s->io->error; goto fail; }
    *capacity = s->capacity; *error = 0; return s;
fail:
    moan_encoder_free(s); return NULL;
}
static int packets(struct moan_encoder *s) {
    int r;
    while ((r = avcodec_receive_packet(s->codec, s->packet)) >= 0) {
        if (s->packet->size < 0 || s->packet->size > 1048576) { av_packet_unref(s->packet); return AVERROR(ENOSPC); }
        av_packet_rescale_ts(s->packet, s->codec->time_base, s->stream->time_base);
        s->packet->stream_index = s->stream->index;
        r = av_write_frame(s->format, s->packet);
        av_packet_unref(s->packet);
        if (r < 0) return r;
    }
    if (r != AVERROR(EAGAIN) && r != AVERROR_EOF) return r;
    if (s->finished && r != AVERROR_EOF) return AVERROR(EIO);
    avio_flush(s->io);
    return s->io->error < 0 ? s->io->error : 0;
}
int moan_encoder_frame(struct moan_encoder *s, const int16_t *pcm, int samples) {
    if (!s || s->finished || !pcm || samples < 1 || samples > s->capacity ||
        s->accepted + samples > s->expected ||
        (samples < s->capacity && s->accepted + samples != s->expected)) return AVERROR(EINVAL);
    // Capacity stays at the allocation's full frame length. FFmpeg may pad the
    // final short MP3 frame internally; unfilled storage is explicitly silence.
    s->frame->nb_samples = s->capacity;
    int r = av_frame_make_writable(s->frame);
    if (r < 0) return r;
    av_samples_set_silence(s->frame->data, 0, s->capacity, 2, s->codec->sample_fmt);
    if (s->codec->sample_fmt == AV_SAMPLE_FMT_S16P) {
        for (int c = 0; c < 2; c++) for (int i = 0; i < samples; i++)
            ((int16_t *)s->frame->data[c])[i] = pcm[2 * i + c];
    } else {
        memcpy(s->frame->data[0], pcm, (size_t)samples * 4);
    }
    s->frame->nb_samples = samples;
    s->frame->pts = s->accepted;
    r = avcodec_send_frame(s->codec, s->frame);
    if (r < 0) return r;
    s->accepted += samples;
    return packets(s);
}
int moan_encoder_finish(struct moan_encoder *s) {
    if (!s || s->finished || s->accepted != s->expected) return AVERROR(EINVAL);
    s->finished = 1;
    int r = avcodec_send_frame(s->codec, NULL);
    if (r < 0 && r != AVERROR_EOF) return r;
    r = packets(s);
    if (r < 0) return r;
    r = av_write_trailer(s->format);
    avio_flush(s->io);
    return r < 0 ? r : s->io->error < 0 ? s->io->error : 0;
}
