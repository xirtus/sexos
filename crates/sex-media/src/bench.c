// SPDX-License-Identifier: GPL-3.0-only
// sex-media's SIMD bench: decodes a whole file held in memory, or encodes synthetic
// pictures, with either every SIMD level the CPU and kernel allow (AVX, AVX2, FMA3)
// or SSE only. libvpx and libaom choose their functions once per program, so a run
// picks one level at its start (sx_bench_simd) and the bench program runs twice to
// compare. Rust reaches it through sex_media::bench.
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/cpu.h>
#include <libavutil/opt.h>
#include <libavutil/pixdesc.h>
#include <libswscale/swscale.h>
#include <libswresample/swresample.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <x86intrin.h>

// dav1d and x264 keep their own CPU detection; both are optional archives.
extern void dav1d_set_cpu_flags_mask(unsigned mask) __attribute__((weak));
extern uint32_t x264_cpu_detect(void) __attribute__((weak));

#define AVX_FLAGS (AV_CPU_FLAG_AVX | AV_CPU_FLAG_AVXSLOW | AV_CPU_FLAG_AVX2 | AV_CPU_FLAG_FMA3 | \
                   AV_CPU_FLAG_FMA4 | AV_CPU_FLAG_XOP | AV_CPU_FLAG_AVX512 | AV_CPU_FLAG_AVX512ICL)
// x264.h: AVX (1<<9), XOP, FMA4, FMA3, AVX2 (1<<15), AVX512 (1<<16).
#define X264_AVX_FLAGS ((1u << 9) | (1u << 10) | (1u << 11) | (1u << 12) | (1u << 15) | (1u << 16))

// 0: every level the CPU and kernel allow; 1: SSE only; 2: up to AVX2 (no AVX-512).
static int level;

struct sx_bench {
    int frames;
    int width, height;
    int64_t bytes;      // encoded bytes (encode) or packet bytes read (decode)
    uint64_t cycles;    // TSC cycles spent decoding or encoding
    uint64_t hash;      // FNV-1a of every decoded plane row, or of every packet
    uint64_t convert_cycles; // decode only: turning each picture into BGRA (as players
                        // do, bilinear) or each sound frame into 48 kHz stereo i16
    char codec[16];
};

// Chooses the SIMD level for the rest of the program, before any codec opens:
// 0 = all the CPU and kernel allow, 1 = SSE only, 2 = up to AVX2/FMA3 (no AVX-512).
void sx_bench_simd(int which) {
    level = which;
    av_force_cpu_flags(-1);
    unsigned flags = av_get_cpu_flags();
    if (level == 1) av_force_cpu_flags(flags & ~AVX_FLAGS);
    if (level == 2) av_force_cpu_flags(flags & ~(AV_CPU_FLAG_AVX512 | AV_CPU_FLAG_AVX512ICL));
    // dav1d x86: SSE2 1, SSSE3 2, SSE4.1 4, AVX2 8, AVX-512 (Ice Lake) 16.
    if (dav1d_set_cpu_flags_mask) dav1d_set_cpu_flags_mask(level == 1 ? 7u : level == 2 ? 15u : ~0u);
    // libvpx and libaom read these masks when they first detect the CPU (HAS_* bits:
    // AVX 0x40, AVX2 0x80, then AVX-512 (0x100 in libvpx, 0x200 in libaom, where
    // 0x100 is SSE4.2)).
    setenv("VPX_SIMD_CAPS_MASK", level == 1 ? "0x3f" : level == 2 ? "0xff" : "0xffffffff", 1);
    setenv("AOM_SIMD_CAPS_MASK", level == 1 ? "0x13f" : level == 2 ? "0x1ff" : "0xffffffff", 1);
}

unsigned sx_bench_cpu_flags(void) { return (unsigned)av_get_cpu_flags(); }

static uint64_t fnv(uint64_t h, const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; i++) { h ^= p[i]; h *= 0x100000001b3ull; }
    return h;
}

struct mem { const uint8_t *data; size_t len, at; };

static int mem_read(void *opaque, uint8_t *buf, int size) {
    struct mem *m = opaque;
    size_t n = m->len - m->at;
    if (n == 0) return AVERROR_EOF;
    if (n > (size_t)size) n = size;
    memcpy(buf, m->data + m->at, n);
    m->at += n;
    return (int)n;
}

static int64_t mem_seek(void *opaque, int64_t offset, int whence) {
    struct mem *m = opaque;
    if (whence == AVSEEK_SIZE) return m->len;
    int64_t to = whence == SEEK_CUR ? (int64_t)m->at + offset : whence == SEEK_END ? (int64_t)m->len + offset : offset;
    if (to < 0 || to > (int64_t)m->len) return AVERROR(EINVAL);
    m->at = to;
    return to;
}

static uint64_t hash_frame(uint64_t h, const AVFrame *f) {
    const AVPixFmtDescriptor *d = av_pix_fmt_desc_get(f->format);
    if (!d) return h;
    int bpc = (d->comp[0].depth + 7) / 8;
    for (int p = 0; p < 4 && f->data[p]; p++) {
        int w = f->width, rows = f->height;
        if (p == 1 || p == 2) { w = AV_CEIL_RSHIFT(w, d->log2_chroma_w); rows = AV_CEIL_RSHIFT(rows, d->log2_chroma_h); }
        for (int y = 0; y < rows; y++) h = fnv(h, f->data[p] + (ptrdiff_t)y * f->linesize[p], (size_t)w * bpc);
    }
    return h;
}

// Decodes every picture of the first video stream (or every sound frame when there
// is none). With show_w x show_h > 0 each picture is also scaled into BGRA at that
// size and each sound frame resampled to 48 kHz stereo i16, as sex-media's player
// bridge does, timed apart. Returns 0 or an FFmpeg error.
int sx_bench_decode(const uint8_t *data, size_t len, int show_w, int show_h, struct sx_bench *out) {
    memset(out, 0, sizeof *out);
    out->hash = 0xcbf29ce484222325ull;
    struct mem m = { data, len, 0 };
    AVFormatContext *fmt = avformat_alloc_context();
    unsigned char *buf = av_malloc(65536);
    AVIOContext *io = buf ? avio_alloc_context(buf, 65536, 0, &m, mem_read, NULL, mem_seek) : NULL;
    AVCodecContext *cc = NULL;
    AVPacket *pkt = av_packet_alloc();
    AVFrame *frame = av_frame_alloc();
    struct SwsContext *sws = NULL;
    SwrContext *swr = NULL;
    uint8_t *shown = show_w > 0 && show_h > 0 ? av_malloc((size_t)show_w * show_h * 4) : NULL;
    int16_t *pcm = av_malloc(2 * 2 * 16384);
    int r = (!fmt || !io || !pkt || !frame || !pcm) ? AVERROR(ENOMEM) : 0;
    if (r == 0) {
        fmt->pb = io;
        fmt->flags |= AVFMT_FLAG_CUSTOM_IO;
        r = avformat_open_input(&fmt, NULL, NULL, NULL);
        if (r < 0) fmt = NULL;
    }
    if (r == 0) r = avformat_find_stream_info(fmt, NULL);
    const AVCodec *codec = NULL;
    int stream = -1;
    if (r >= 0) {
        stream = av_find_best_stream(fmt, AVMEDIA_TYPE_VIDEO, -1, -1, &codec, 0);
        if (stream < 0) stream = av_find_best_stream(fmt, AVMEDIA_TYPE_AUDIO, -1, -1, &codec, 0);
        r = stream < 0 ? stream : 0;
    }
    if (r == 0) {
        cc = avcodec_alloc_context3(codec);
        r = cc ? avcodec_parameters_to_context(cc, fmt->streams[stream]->codecpar) : AVERROR(ENOMEM);
    }
    if (r >= 0) { cc->thread_count = 1; r = avcodec_open2(cc, codec, NULL); }
    if (r >= 0) {
        strncpy(out->codec, codec->name, sizeof out->codec - 1);
        int video = cc->codec_type == AVMEDIA_TYPE_VIDEO;
        uint64_t spent = 0;
        int draining = 0;
        for (;;) {
            if (!draining) {
                r = av_read_frame(fmt, pkt);
                if (r < 0) { draining = 1; pkt->data = NULL; pkt->size = 0; }
                else if (pkt->stream_index != stream) { av_packet_unref(pkt); continue; }
                else out->bytes += pkt->size;
            }
            uint64_t t0 = __rdtsc();
            r = avcodec_send_packet(cc, draining ? NULL : pkt);
            if (!draining) av_packet_unref(pkt);
            if (r < 0 && r != AVERROR(EAGAIN) && r != AVERROR_EOF) { spent += __rdtsc() - t0; break; }
            int got;
            while ((got = avcodec_receive_frame(cc, frame)) >= 0) {
                spent += __rdtsc() - t0;
                out->frames++;
                if (video) { out->width = frame->width; out->height = frame->height; out->hash = hash_frame(out->hash, frame); }
                uint64_t c0 = __rdtsc();
                if (video && shown) {
                    sws = sws_getCachedContext(sws, frame->width, frame->height, frame->format, show_w, show_h,
                                               AV_PIX_FMT_BGRA, SWS_BILINEAR, NULL, NULL, NULL);
                    uint8_t *dst[4] = { shown }; int stride[4] = { show_w * 4 };
                    if (sws) sws_scale(sws, (const uint8_t *const *)frame->data, frame->linesize, 0, frame->height, dst, stride);
                } else if (!video && show_w > 0) {
                    if (!swr) {
                        AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
                        if (swr_alloc_set_opts2(&swr, &stereo, AV_SAMPLE_FMT_S16, 48000, &frame->ch_layout,
                                                frame->format, frame->sample_rate, 0, NULL) < 0 || swr_init(swr) < 0) swr_free(&swr);
                    }
                    uint8_t *dst[1] = { (uint8_t *)pcm };
                    if (swr) swr_convert(swr, dst, 16384, (const uint8_t **)frame->extended_data, frame->nb_samples);
                }
                out->convert_cycles += __rdtsc() - c0;
                av_frame_unref(frame);
                t0 = __rdtsc();
            }
            spent += __rdtsc() - t0;
            if (got == AVERROR_EOF) { r = 0; break; }
            if (got != AVERROR(EAGAIN)) { r = got; break; }
        }
        out->cycles = spent;
    }
    sws_freeContext(sws);
    swr_free(&swr);
    av_free(shown);
    av_free(pcm);
    av_frame_free(&frame);
    av_packet_free(&pkt);
    avcodec_free_context(&cc);
    avformat_close_input(&fmt);
    if (io) { av_freep(&io->buffer); avio_context_free(&io); }
    return r;
}

// A moving picture: diagonal bands, a sliding square and a little texture, so the
// encoder has motion to search and detail to code.
static void paint(AVFrame *f, int n) {
    for (int y = 0; y < f->height; y++) {
        uint8_t *row = f->data[0] + (ptrdiff_t)y * f->linesize[0];
        for (int x = 0; x < f->width; x++) {
            int v = ((x + y + 3 * n) & 63) * 3 + (((x * 7) ^ (y * 13)) & 15);
            int in = x >= (n * 8) % f->width && x < (n * 8) % f->width + 96 && y >= f->height / 3 && y < f->height / 3 + 96;
            row[x] = in ? 235 : (uint8_t)(16 + v % 200);
        }
    }
    for (int p = 1; p < 3; p++)
        for (int y = 0; y < f->height / 2; y++) {
            uint8_t *row = f->data[p] + (ptrdiff_t)y * f->linesize[p];
            for (int x = 0; x < f->width / 2; x++) row[x] = (uint8_t)(128 + ((x * p + y + n) & 31) - 16);
        }
}

// Encodes `frames` synthetic pictures of w x h with the named encoder (libx264,
// libvpx-vp9, libaom-av1, ffv1, or flac for sound: two seconds per "frame").
// Returns 0 or an FFmpeg error.
int sx_bench_encode(const char *name, int w, int h, int frames, struct sx_bench *out) {
    memset(out, 0, sizeof *out);
    out->hash = 0xcbf29ce484222325ull;
    const AVCodec *codec = avcodec_find_encoder_by_name(name);
    if (!codec) return AVERROR_ENCODER_NOT_FOUND;
    strncpy(out->codec, codec->name, sizeof out->codec - 1);
    AVCodecContext *cc = avcodec_alloc_context3(codec);
    AVFrame *f = av_frame_alloc();
    AVPacket *pkt = av_packet_alloc();
    int r = (!cc || !f || !pkt) ? AVERROR(ENOMEM) : 0;
    AVDictionary *opts = NULL;
    int audio = codec->type == AVMEDIA_TYPE_AUDIO;
    if (r == 0) {
        cc->thread_count = 1;
        cc->time_base = (AVRational){1, audio ? 48000 : 30};
        if (audio) {
            cc->sample_rate = 48000;
            cc->sample_fmt = AV_SAMPLE_FMT_S16;
            av_channel_layout_default(&cc->ch_layout, 2);
            cc->compression_level = 8;
        } else {
            cc->width = w; cc->height = h;
            cc->pix_fmt = AV_PIX_FMT_YUV420P;
            cc->framerate = (AVRational){30, 1};
            cc->gop_size = 60;
            cc->bit_rate = 2000000;
            if (!strcmp(name, "libx264")) {
                av_dict_set(&opts, "preset", "medium", 0);
                if (level && x264_cpu_detect) {
                    char asm_flags[32];
                    snprintf(asm_flags, sizeof asm_flags, "asm=%u", x264_cpu_detect() & ~(level == 1 ? X264_AVX_FLAGS : 1u << 16));
                    av_dict_set(&opts, "x264-params", asm_flags, 0);
                }
            } else if (!strcmp(name, "libvpx-vp9")) {
                av_dict_set(&opts, "deadline", "good", 0);
                av_dict_set(&opts, "cpu-used", "4", 0);
            } else if (!strcmp(name, "libaom-av1")) {
                av_dict_set(&opts, "usage", "realtime", 0);
                av_dict_set(&opts, "cpu-used", "8", 0);
            }
        }
        r = avcodec_open2(cc, codec, &opts);
        av_dict_free(&opts);
    }
    if (r >= 0) {
        if (audio) {
            f->nb_samples = cc->frame_size ? cc->frame_size : 4608;
            f->format = cc->sample_fmt;
            av_channel_layout_copy(&f->ch_layout, &cc->ch_layout);
        } else {
            f->width = w; f->height = h; f->format = cc->pix_fmt;
        }
        r = av_frame_get_buffer(f, 0);
    }
    uint64_t spent = 0;
    // Sound: `frames` x 2 s of a chord with a slow sweep, in encoder-sized blocks.
    int total = audio ? frames * 96000 / (f->nb_samples ? f->nb_samples : 1) : frames;
    for (int n = 0; r >= 0 && n <= total; n++) {
        AVFrame *in = NULL;
        if (n < total) {
            if ((r = av_frame_make_writable(f)) < 0) break;
            if (audio) {
                int16_t *s = (int16_t *)f->data[0];
                for (int i = 0; i < f->nb_samples; i++) {
                    int t = n * f->nb_samples + i;
                    int a = ((t * 440 / 48) % 1000) - 500, b = ((t * (554 + (t >> 14)) / 48) % 1000) - 500;
                    s[2 * i] = (int16_t)(a * 20 + b * 12);
                    s[2 * i + 1] = (int16_t)(a * 12 + b * 20);
                }
            } else {
                paint(f, n);
            }
            f->pts = audio ? (int64_t)n * f->nb_samples : n;
            in = f;
        }
        uint64_t t0 = __rdtsc();
        r = avcodec_send_frame(cc, in);
        while (r >= 0) {
            int got = avcodec_receive_packet(cc, pkt);
            if (got == AVERROR(EAGAIN) || got == AVERROR_EOF) break;
            if (got < 0) { r = got; break; }
            out->bytes += pkt->size;
            out->hash = fnv(out->hash, pkt->data, pkt->size);
            av_packet_unref(pkt);
        }
        spent += __rdtsc() - t0;
        if (!in) break;
    }
    if (r >= 0) r = 0;
    out->frames = audio ? total : frames;
    out->width = w; out->height = h;
    out->cycles = spent;
    av_packet_free(&pkt);
    av_frame_free(&f);
    avcodec_free_context(&cc);
    return r;
}
