// SPDX-License-Identifier: GPL-3.0-only
// Host smoke for Splicy's export bridge: two seconds of a cut (red, then blue at frame
// 30) with a 440 Hz tone, written in each format through write/seek callbacks only.
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

struct sx_export;
typedef int (*wfn)(void *, const uint8_t *, int);
typedef int64_t (*sfn)(void *, int64_t, int);
struct sx_export *sx_export_open(int, int, int, int, int, int, int, void *, wfn, sfn, int *);
int sx_export_picture(struct sx_export *, const uint32_t *, int64_t);
int sx_export_sound(struct sx_export *, const int16_t *, int);
int sx_export_finish(struct sx_export *);
void sx_export_free(struct sx_export *);

static int w(void *f, const uint8_t *b, int n) { return fwrite(b, 1, n, f) == (size_t)n ? n : -1; }
static int64_t sk(void *f, int64_t off, int whence) {
    if (whence == 0x10000) { long at = ftell(f); fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, at, SEEK_SET); return n; }
    return fseek(f, off, whence) == 0 ? ftell(f) : -1;
}

int main(int argc, char **argv) {
    if (argc != 3) return 2;
    int format = atoi(argv[1]);
    FILE *f = fopen(argv[2], "wb");
    int err = 0;
    struct sx_export *s = sx_export_open(format, 64, 48, 30, 1, 48000, 0, f, w, sk, &err);
    if (!s) { fprintf(stderr, "open %d\n", err); return 1; }
    static uint32_t px[64 * 48];
    static int16_t pcm[1600 * 2];
    long t = 0;
    for (int n = 0; n < 60; n++) {
        uint32_t c = n < 30 ? 0x00ff0000 : 0x000000ff;
        for (int i = 0; i < 64 * 48; i++) px[i] = c;
        int r = sx_export_picture(s, px, n);
        if (r < 0) { fprintf(stderr, "picture %d: %d\n", n, r); return 1; }
        for (int i = 0; i < 1600; i++, t++) pcm[2 * i] = pcm[2 * i + 1] = (int16_t)(8000 * sin(2 * M_PI * 440 * t / 48000.0));
        r = sx_export_sound(s, pcm, 1600);
        if (r < 0) { fprintf(stderr, "sound %d: %d\n", n, r); return 1; }
    }
    int r = sx_export_finish(s);
    sx_export_free(s);
    fclose(f);
    if (r < 0) { fprintf(stderr, "finish %d\n", r); return 1; }
    return 0;
}
