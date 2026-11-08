// SPDX-License-Identifier: GPL-3.0-only
// The guest side of sex-js: QuickJS-NG with one native function, `__host`, through which
// the prelude reaches the page. Built for wasm32-wasip1 as a reactor by
// scripts/build_quickjs_wasm.sh; the host (crates/sex-js) runs it on wasmi.
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "quickjs.h"

#define IMPORT(name) __attribute__((import_module("dom"), import_name(name)))
// One page operation. Arguments are packed as (u32 length, bytes) pairs. Returns the
// result's length, kept by the host until `result` copies it, or -1 for none.
IMPORT("call") int host_call(int op, const char *args, int len);
IMPORT("result") void host_result(char *out);

#define EXPORT(name) __attribute__((export_name(name)))

static JSRuntime *rt;
static JSContext *ctx;
static int64_t deadline_ns;

static int64_t now_ns(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

// A script that runs too long is stopped, so a page cannot hang the browser.
static int interrupt(JSRuntime *r, void *opaque) {
    (void)r; (void)opaque;
    return deadline_ns && now_ns() > deadline_ns;
}

#define MAX_ARGS 16
static JSValue js_host(JSContext *c, JSValueConst this_val, int argc, JSValueConst *argv) {
    (void)this_val;
    int op = 0;
    if (argc < 1 || JS_ToInt32(c, &op, argv[0])) return JS_EXCEPTION;
    int n = argc - 1 > MAX_ARGS ? MAX_ARGS : argc - 1;
    const char *s[MAX_ARGS];
    size_t l[MAX_ARGS];
    size_t total = 0;
    for (int i = 0; i < n; i++) {
        s[i] = JS_ToCStringLen(c, &l[i], argv[i + 1]);
        if (!s[i]) { for (int j = 0; j < i; j++) JS_FreeCString(c, s[j]); return JS_EXCEPTION; }
        total += 4 + l[i];
    }
    char *buf = malloc(total + 1);
    if (!buf) { for (int j = 0; j < n; j++) JS_FreeCString(c, s[j]); return JS_ThrowOutOfMemory(c); }
    char *p = buf;
    for (int i = 0; i < n; i++) {
        uint32_t len = (uint32_t)l[i];
        memcpy(p, &len, 4); p += 4;
        memcpy(p, s[i], l[i]); p += l[i];
        JS_FreeCString(c, s[i]);
    }
    int r = host_call(op, buf, (int)total);
    free(buf);
    if (r < 0) return JS_NULL;
    char *out = malloc((size_t)r + 1);
    if (!out) return JS_ThrowOutOfMemory(c);
    host_result(out);
    JSValue v = JS_NewStringLen(c, out, (size_t)r);
    free(out);
    return v;
}

EXPORT("sx_init") int sx_init(int memory_limit) {
    rt = JS_NewRuntime();
    if (!rt) return -1;
    if (memory_limit > 0) JS_SetMemoryLimit(rt, (size_t)memory_limit);
    JS_SetMaxStackSize(rt, 256 * 1024);
    JS_SetInterruptHandler(rt, interrupt, NULL);
    ctx = JS_NewContext(rt);
    if (!ctx) return -1;
    JSValue global = JS_GetGlobalObject(ctx);
    JS_SetPropertyStr(ctx, global, "__host", JS_NewCFunction(ctx, js_host, "__host", 1));
    JS_FreeValue(ctx, global);
    return 0;
}

EXPORT("sx_alloc") void *sx_alloc(int n) { return malloc((size_t)n); }
EXPORT("sx_free") void sx_free(void *p) { free(p); }

// Reports an exception through op 1 (log, level 2) and clears it.
static void report(void) {
    JSValue e = JS_GetException(ctx);
    const char *msg = JS_ToCString(ctx, e);
    JSValue stack = JS_IsError(ctx, e) ? JS_GetPropertyStr(ctx, e, "stack") : JS_UNDEFINED;
    const char *st = JS_IsUndefined(stack) ? NULL : JS_ToCString(ctx, stack);
    size_t ml = msg ? strlen(msg) : 0, sl = st ? strlen(st) : 0;
    char *buf = malloc(4 + 1 + 4 + ml + 1 + sl);
    if (buf) {
        uint32_t one = 1, len = (uint32_t)(ml + (sl ? 1 + sl : 0));
        memcpy(buf, &one, 4); buf[4] = '2';
        memcpy(buf + 5, &len, 4);
        if (ml) memcpy(buf + 9, msg, ml);
        if (sl) { buf[9 + ml] = '\n'; memcpy(buf + 10 + ml, st, sl); }
        host_call(1, buf, (int)(9 + len));
        free(buf);
    }
    if (msg) JS_FreeCString(ctx, msg);
    if (st) JS_FreeCString(ctx, st);
    JS_FreeValue(ctx, stack);
    JS_FreeValue(ctx, e);
}

static void run_jobs(void) {
    JSContext *c;
    for (int i = 0; i < 10000; i++) {
        int r = JS_ExecutePendingJob(rt, &c);
        if (r == 0) break;
        if (r < 0) report();
    }
}

// Evaluates source as a classic script. 0 on success, -1 after reporting an exception.
EXPORT("sx_eval") int sx_eval(const char *src, int len, const char *name, int name_len, int budget_ms) {
    char file[128];
    int fl = name_len < 127 ? name_len : 127;
    memcpy(file, name, (size_t)fl); file[fl] = 0;
    // JS_Eval needs a terminating zero after the source.
    char *code = malloc((size_t)len + 1);
    if (!code) return -1;
    memcpy(code, src, (size_t)len); code[len] = 0;
    deadline_ns = budget_ms > 0 ? now_ns() + (int64_t)budget_ms * 1000000 : 0;
    JSValue v = JS_Eval(ctx, code, (size_t)len, file, JS_EVAL_TYPE_GLOBAL);
    free(code);
    int ok = 0;
    if (JS_IsException(v)) { report(); ok = -1; }
    JS_FreeValue(ctx, v);
    run_jobs();
    deadline_ns = 0;
    return ok;
}

// Calls the prelude's dispatcher with one string argument; returns its integer result.
EXPORT("sx_dispatch") int sx_dispatch(const char *arg, int len, int budget_ms) {
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue fn = JS_GetPropertyStr(ctx, global, "__sx_dispatch");
    int out = 0;
    if (JS_IsFunction(ctx, fn)) {
        JSValue a = JS_NewStringLen(ctx, arg, (size_t)len);
        deadline_ns = budget_ms > 0 ? now_ns() + (int64_t)budget_ms * 1000000 : 0;
        JSValue r = JS_Call(ctx, fn, global, 1, &a);
        if (JS_IsException(r)) { report(); out = -1; }
        else JS_ToInt32(ctx, &out, r);
        JS_FreeValue(ctx, r);
        JS_FreeValue(ctx, a);
        run_jobs();
        deadline_ns = 0;
    }
    JS_FreeValue(ctx, fn);
    JS_FreeValue(ctx, global);
    return out;
}
