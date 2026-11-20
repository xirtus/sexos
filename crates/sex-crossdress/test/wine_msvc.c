// SPDX-License-Identifier: GPL-3.0-only
// CW4 test program: imports its C runtime the way MSVC builds do, through API set names
// (api-ms-win-crt-*.dll) and vcruntime140.dll, which Wine's ntdll resolves through the
// API set schema in the PEB. Built with clang + lld-link, no SDK; runs on Windows too.
#include <stdarg.h>
typedef unsigned long long size_t;
__declspec(dllimport) void *malloc(size_t);               // api-ms-win-crt-heap
__declspec(dllimport) void free(void *);
__declspec(dllimport) size_t strlen(const char *);        // api-ms-win-crt-string
__declspec(dllimport) int puts(const char *);             // api-ms-win-crt-stdio
__declspec(dllimport) int __stdio_common_vsprintf(unsigned long long, char *, size_t, const char *, void *, va_list);
__declspec(dllimport) double sqrt(double);                // api-ms-win-crt-math
__declspec(dllimport) void exit(int);                     // api-ms-win-crt-runtime
__declspec(dllimport) void *memcpy(void *, const void *, size_t); // vcruntime140
__declspec(dllimport) void *memset(void *, int, size_t);
__declspec(dllimport) int memcmp(const void *, const void *, size_t);

int _fltused = 0;
static int failures;
static void check(int ok, const char *what) {
    char line[160]; size_t i = 0, n = strlen(what);
    memcpy(line, ok ? "ok   " : "FAIL ", 5); i = 5;
    if (n > 150) n = 150;
    memcpy(line + i, what, n); line[i + n] = 0;
    puts(line);
    if (!ok) failures++;
}
static int format(char *buf, size_t n, const char *fmt, ...) {
    va_list ap; va_start(ap, fmt);
    int r = __stdio_common_vsprintf(0, buf, n, fmt, 0, ap);
    va_end(ap);
    return r;
}

void start(void) {
    puts("crossdress: hello from an MSVC-style program (API sets, vcruntime140)");
    char *p = malloc(64);
    check(p != 0, "malloc through api-ms-win-crt-heap");
    memset(p, 'x', 63); p[63] = 0;
    check(strlen(p) == 63, "memset (vcruntime140) and strlen (api-ms-win-crt-string)");
    free(p);
    char buf[32];
    check(format(buf, sizeof buf, "%.3f", sqrt(2.0)) == 5 && memcmp(buf, "1.414", 5) == 0, "sqrt and sprintf through API sets");
    puts(failures ? "crossdress: msvc checks FAILED" : "crossdress: msvc checks passed");
    exit(failures);
}
