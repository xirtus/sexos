// SPDX-License-Identifier: GPL-3.0-only
// Host smoke driver feeding Burlesque's sex-splice rehearsed frames into Splicy's
// crates/sex-media/src/export/bridge.c VP9 WebM encoder (sx_export_open format=0).
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

struct sx_export;
typedef int (*wfn)(void *, const uint8_t *, int);
typedef int64_t (*sfn)(void *, int64_t, int);
struct sx_export *sx_export_open(int, int, int, int, int, int, int, void *, wfn, sfn, int *);
int sx_export_picture(struct sx_export *, const uint32_t *, int64_t);
int sx_export_finish(struct sx_export *);
void sx_export_free(struct sx_export *);

static int w_cb(void *f, const uint8_t *b, int n) {
    return fwrite(b, 1, (size_t)n, (FILE *)f) == (size_t)n ? n : -1;
}

static int64_t sk_cb(void *f, int64_t off, int whence) {
    FILE *fp = (FILE *)f;
    if (whence == 0x10000) {
        long at = ftell(fp);
        if (at < 0 || fseek(fp, 0, SEEK_END) != 0) return -1;
        long sz = ftell(fp);
        if (fseek(fp, at, SEEK_SET) != 0) return -1;
        return sz;
    }
    return fseek(fp, (long)off, whence) == 0 ? ftell(fp) : -1;
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: %s <raw-frames.bin> <out.webm>\n", argv[0]);
        return 2;
    }

    FILE *in = fopen(argv[1], "rb");
    if (!in) {
        perror("fopen raw-frames");
        return 1;
    }

    uint32_t hdr[4] = {0};
    if (fread(hdr, sizeof(uint32_t), 4, in) != 4) {
        fprintf(stderr, "short header\n");
        fclose(in);
        return 1;
    }
    uint32_t width = hdr[0];
    uint32_t height = hdr[1];
    uint32_t fps = hdr[2];
    uint32_t n_frames = hdr[3];
    if (width < 16 || height < 16 || fps == 0 || n_frames == 0 || width > 4096 || height > 4096) {
        fprintf(stderr, "invalid header %ux%u @ %u fps (%u frames)\n", width, height, fps, n_frames);
        fclose(in);
        return 1;
    }

    FILE *out = fopen(argv[2], "wb");
    if (!out) {
        perror("fopen out.webm");
        fclose(in);
        return 1;
    }

    int err = 0;
    struct sx_export *s = sx_export_open(
        0, // Format::WebmVp9Opus
        (int)width,
        (int)height,
        (int)fps,
        1,
        0,  // video-only presentation export
        32, // CRF quality
        out,
        w_cb,
        sk_cb,
        &err
    );
    if (!s) {
        fprintf(stderr, "sx_export_open failed: %d\n", err);
        fclose(out);
        fclose(in);
        return 1;
    }

    size_t px_count = (size_t)width * (size_t)height;
    uint32_t *px = (uint32_t *)malloc(px_count * sizeof(uint32_t));
    if (!px) {
        sx_export_free(s);
        fclose(out);
        fclose(in);
        return 1;
    }

    for (uint32_t n = 0; n < n_frames; n++) {
        if (fread(px, sizeof(uint32_t), px_count, in) != px_count) {
            fprintf(stderr, "short frame %u\n", n);
            free(px);
            sx_export_free(s);
            fclose(out);
            fclose(in);
            return 1;
        }
        int r = sx_export_picture(s, px, (int64_t)n);
        if (r < 0) {
            fprintf(stderr, "sx_export_picture(%u) failed: %d\n", n, r);
            free(px);
            sx_export_free(s);
            fclose(out);
            fclose(in);
            return 1;
        }
    }

    int fin = sx_export_finish(s);
    free(px);
    sx_export_free(s);
    fclose(out);
    fclose(in);
    if (fin < 0) {
        fprintf(stderr, "sx_export_finish failed: %d\n", fin);
        return 1;
    }
    return 0;
}
