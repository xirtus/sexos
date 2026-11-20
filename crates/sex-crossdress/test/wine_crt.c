// SPDX-License-Identifier: GPL-3.0-only
// CW4 test program: a 64-bit Windows console program linked against the Universal C
// runtime (ucrtbase.dll), the way CRT-using plugins are. Wine's ntdll loads it, ucrtbase
// and kernel32 in a Crossdress cell. No headers from any SDK: the few CRT entry points are
// declared here with their documented signatures, so it runs on Windows too.
#include <stdarg.h>
typedef unsigned long long size_t;
__declspec(dllimport) void *malloc(size_t);
__declspec(dllimport) void *calloc(size_t, size_t);
__declspec(dllimport) void free(void *);
__declspec(dllimport) size_t strlen(const char *);
__declspec(dllimport) int memcmp(const void *, const void *, size_t);
__declspec(dllimport) long strtol(const char *, char **, int);
__declspec(dllimport) int atoi(const char *);
__declspec(dllimport) void qsort(void *, size_t, size_t, int (*)(const void *, const void *));
__declspec(dllimport) double sqrt(double);
__declspec(dllimport) int puts(const char *);
__declspec(dllimport) void exit(int);
__declspec(dllimport) int __stdio_common_vsprintf(unsigned long long, char *, size_t, const char *, void *, va_list);

int _fltused = 0;
static int failures;
static void check(int ok, const char *what) {
    char line[160]; int i = 0;
    for (const char *s = ok ? "ok   " : "FAIL "; *s; s++) line[i++] = *s;
    for (const char *s = what; *s && i < 150; s++) line[i++] = *s;
    line[i] = 0;
    puts(line);
    if (!ok) failures++;
}
static int format(char *buf, size_t n, const char *fmt, ...) {
    va_list ap; va_start(ap, fmt);
    int r = __stdio_common_vsprintf(0, buf, n, fmt, 0, ap);
    va_end(ap);
    return r;
}
static int by_value(const void *a, const void *b) { return *(const int *)a - *(const int *)b; }

void start(void) {
    puts("crossdress: hello from Wine's C runtime");
    char *p = malloc(1000);
    check(p != 0, "malloc");
    free(p);
    int *z = calloc(256, sizeof *z);
    int zero = z != 0;
    for (int i = 0; z && i < 256; i++) zero &= z[i] == 0;
    check(zero, "calloc gives zeroed memory");
    free(z);
    check(strlen("crossdress") == 10, "strlen");
    check(strtol("-0x7f", 0, 16) == -127 && atoi("4096") == 4096, "strtol and atoi");
    int v[] = { 5, 3, 9, 1, 7 };
    qsort(v, 5, sizeof v[0], by_value);
    check(v[0] == 1 && v[2] == 5 && v[4] == 9, "qsort with a callback into the program");
    check(sqrt(2.25) == 1.5, "sqrt");
    char buf[64];
    int n = format(buf, sizeof buf, "%s %d %05.2f %x", "gain", 42, 3.14159, 255u);
    check(n == 16 && memcmp(buf, "gain 42 03.14 ff", 16) == 0, "sprintf (strings, ints, floats, hex)");
    puts(failures ? "crossdress: crt checks FAILED" : "crossdress: crt checks passed");
    exit(failures);
}
