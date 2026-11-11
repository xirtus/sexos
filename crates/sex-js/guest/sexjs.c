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
IMPORT("jit_available") int host_jit_available(void);
int sexos_baseline_jit_enabled;

#define EXPORT(name) __attribute__((export_name(name)))

static JSRuntime *rt;
static JSContext *ctx;
static int64_t deadline_ns;
/* Per-Wasm-instance source policy, inherited by compiled function metadata. */
int sexos_compile_origin = -1;
int sexos_opaque_source_seen;
/* Per-VM ambient callback/job provenance: -1 unknown, 0 owned, 1 muted.
   Every native callback/job scope restores its previous value before returning. */
int sexos_job_muted = -1;
extern int sexos_promise_flags(JSValueConst promise);
static JSValue rejection_handler = JS_UNDEFINED;
static void report(void);


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
    // ArrayBuffer arguments pass as their raw bytes (not strings, not to be freed).
    bool raw[MAX_ARGS];
    size_t total = 0;
    for (int i = 0; i < n; i++) {
        raw[i] = JS_IsArrayBuffer(argv[i + 1]);
        s[i] = raw[i] ? (const char *)JS_GetArrayBuffer(c, &l[i], argv[i + 1]) : JS_ToCStringLen(c, &l[i], argv[i + 1]);
        if (raw[i] && !s[i]) { s[i] = ""; l[i] = 0; }
        if (!s[i]) { for (int j = 0; j < i; j++) if (!raw[j]) JS_FreeCString(c, s[j]); return JS_EXCEPTION; }
        total += 4 + l[i];
    }
    char *buf = malloc(total + 1);
    if (!buf) { for (int j = 0; j < n; j++) if (!raw[j]) JS_FreeCString(c, s[j]); return JS_ThrowOutOfMemory(c); }
    char *p = buf;
    for (int i = 0; i < n; i++) {
        uint32_t len = (uint32_t)l[i];
        memcpy(p, &len, 4); p += 4;
        memcpy(p, s[i], l[i]); p += l[i];
        if (!raw[i]) JS_FreeCString(c, s[i]);
    }
    // Bit 16 of the op asks for the result as an ArrayBuffer.
    int binary = op & 0x10000;
    int r = host_call(op & 0xFFFF, buf, (int)total);
    free(buf);
    if (r < 0) return JS_NULL;
    char *out = malloc((size_t)r + 1);
    if (!out) return JS_ThrowOutOfMemory(c);
    host_result(out);
    JSValue v = binary ? JS_NewArrayBufferCopy(c, (const uint8_t *)out, (size_t)r) : JS_NewStringLen(c, out, (size_t)r);
    free(out);
    return v;
}

// ---- ES modules ----
// Module addresses are resolved and their sources supplied by the host (ops 29 and 30);
// the host fetches what a module graph needs before the graph runs (see sx_module).
#define OP_MODULE_RESOLVE 29
#define OP_MODULE_SOURCE 30
// Set while sx_module only discovers a graph: a module the host lacks loads as an
// empty stand-in (the host notes it), so every import reachable today is found.
static int discovering;
// How many modules the host lacked during the last discovery.
static int missed;

// Calls host op `op` with one or two text arguments; returns its text (malloc'd, zero
// terminated, length in *n) or NULL when the host has none.
static char *host_text(int op, const char *a, size_t al, const char *b, size_t bl, size_t *n) {
    size_t total = 4 + al + (b ? 4 + bl : 0);
    char *buf = malloc(total);
    if (!buf) return NULL;
    uint32_t l = (uint32_t)al;
    memcpy(buf, &l, 4); memcpy(buf + 4, a, al);
    if (b) { l = (uint32_t)bl; memcpy(buf + 4 + al, &l, 4); memcpy(buf + 8 + al, b, bl); }
    int r = host_call(op, buf, (int)total);
    free(buf);
    if (r < 0) return NULL;
    char *out = malloc((size_t)r + 1);
    if (!out) return NULL;
    host_result(out);
    out[r] = 0;
    *n = (size_t)r;
    return out;
}

#define OP_SOURCE_MUTED 72
static int source_muted(const char *name, size_t len) {
    size_t n = 0;
    char *policy = host_text(OP_SOURCE_MUTED, name, len, NULL, 0, &n);
    int muted = !policy || n != 1 || policy[0] != '0';
    free(policy);
    if (muted) sexos_opaque_source_seen = 1;
    return muted;
}
static JSValue promise_setup(JSContext *c, JSValueConst self, int argc, JSValueConst *argv) {
    (void)self;
    if (!JS_IsUndefined(rejection_handler) || argc != 1 || !JS_IsFunction(c, argv[0]))
        return JS_ThrowTypeError(c, "invalid promise event registration");
    rejection_handler = JS_DupValue(c, argv[0]);
    return JS_UNDEFINED;
}
static JSValue promise_flags(JSContext *c, JSValueConst self, int argc, JSValueConst *argv) {
    (void)self;
    return JS_NewInt32(c, argc ? sexos_promise_flags(argv[0]) : 3);
}
static JSValue checkpoint_empty(JSContext *c, JSValueConst self, int argc, JSValueConst *argv) {
    (void)self; (void)argc; (void)argv;
    return JS_NewBool(c, !JS_IsJobPending(rt));
}
static void promise_tracker(JSContext *c, JSValueConst promise, JSValueConst reason, bool handled, void *opaque) {
    (void)opaque;
    if (c != ctx || JS_IsUndefined(rejection_handler) || (sexos_promise_flags(promise) & 2)) return;
    JSValueConst args[] = { promise, reason, handled ? JS_TRUE : JS_FALSE };
    JSValue result = JS_Call(c, rejection_handler, JS_UNDEFINED, 3, args);
    if (JS_IsException(result)) report();
    JS_FreeValue(c, result);
}

static char *module_normalize(JSContext *c, const char *base, const char *name, void *opaque) {
    (void)opaque;
    size_t n;
    char *r = host_text(OP_MODULE_RESOLVE, base, strlen(base), name, strlen(name), &n);
    if (!r) { JS_ThrowReferenceError(c, "could not resolve module '%s'", name); return NULL; }
    char *out = js_malloc(c, n + 1);
    if (out) memcpy(out, r, n + 1);
    free(r);
    return out;
}

static JSModuleDef *module_load(JSContext *c, const char *name, void *opaque) {
    (void)opaque;
    size_t n = 0;
    char *src = host_text(OP_MODULE_SOURCE, name, strlen(name), NULL, 0, &n);
    if (!src) {
        if (!discovering) { JS_ThrowReferenceError(c, "could not load module '%s'", name); return NULL; }
        missed++;
        src = calloc(1, 1);
        if (!src) return NULL;
    }
    int previous_origin = sexos_compile_origin;
    sexos_compile_origin = source_muted(name, strlen(name));
    JSValue f = JS_Eval(c, src, n, name, JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
    sexos_compile_origin = previous_origin;
    free(src);
    if (JS_IsException(f)) return NULL;
    JSModuleDef *m = JS_VALUE_GET_PTR(f);
    JSValue meta = JS_GetImportMeta(c, m);
    if (!JS_IsException(meta)) JS_DefinePropertyValueStr(c, meta, "url", JS_NewString(c, name), JS_PROP_C_W_E);
    JS_FreeValue(c, meta);
    JS_FreeValue(c, f);
    return m;
}

// ---- Dynamic import() ----
// import() may name a module (or a graph) the host has not fetched. Such an import is
// held here; the host fetches what it noted and calls sx_modules_ready, which loads every
// held import whose graph is now complete and settles its promise.
extern int (*js_dynamic_import_hook)(JSContext *, const char *, const char *, JSValueConst *);
typedef struct Held { JSContext *c; char *base; char *spec; JSValue funcs[2]; struct Held *next; } Held;
static Held *held;
static void run_jobs(void);


// True when the graph `spec` names (from `base`) needs a module the host lacks; the host
// notes each one. Finds them as sx_module does, in a throwaway context.
static int graph_missing(const char *base, const char *spec) {
    size_t n;
    char *url = host_text(OP_MODULE_RESOLVE, base, strlen(base), spec, strlen(spec), &n);
    if (!url) return 0;
    // `import "url";` with the url as a JS string.
    char *src = malloc(2 * n + 16);
    if (!src) { free(url); return 0; }
    char *p = src;
    memcpy(p, "import \"", 8); p += 8;
    for (size_t i = 0; i < n; i++) { if (url[i] == '"' || url[i] == '\\') *p++ = '\\'; *p++ = url[i] == '\n' ? ' ' : url[i]; }
    memcpy(p, "\";", 3); p += 2;
    free(url);
    JSContext *t = JS_NewContext(rt);
    int found = 0;
    if (t) {
        discovering = 1; missed = 0;
        JSValue f = JS_Eval(t, src, (size_t)(p - src), base, JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
        if (!JS_IsException(f)) JS_ResolveModule(t, f);
        JS_FreeValue(t, JS_GetException(t));
        JS_FreeValue(t, f);
        found = missed;
        discovering = 0;
        JS_FreeContext(t);
    }
    free(src);
    return found;
}

static int hold_import(JSContext *c, const char *base, const char *spec, JSValueConst *funcs) {
    if (discovering || !graph_missing(base, spec)) return 0;
    Held *h = malloc(sizeof *h);
    char *b = strdup(base), *s = strdup(spec);
    if (!h || !b || !s) { free(h); free(b); free(s); return 0; }
    h->c = c; h->base = b; h->spec = s;
    h->funcs[0] = JS_DupValue(c, funcs[0]); h->funcs[1] = JS_DupValue(c, funcs[1]);
    h->next = held; held = h;
    return 1;
}

// Loads the held imports whose graphs are complete. Returns how many are still held.
EXPORT("sx_modules_ready") int sx_modules_ready(int budget_ms) {
    deadline_ns = budget_ms > 0 ? now_ns() + (int64_t)budget_ms * 1000000 : 0;
    Held *list = held, *keep = NULL;
    held = NULL;
    int left = 0;
    while (list) {
        Held *h = list; list = h->next;
        if (graph_missing(h->base, h->spec)) { h->next = keep; keep = h; left++; continue; }
        JSValue promise = JS_LoadModule(h->c, h->base, h->spec);
        if (JS_IsException(promise)) {
            JSValue e = JS_GetException(h->c);
            JS_FreeValue(h->c, JS_Call(h->c, h->funcs[1], JS_UNDEFINED, 1, &e));
            JS_FreeValue(h->c, e);
        } else {
            JSValue then = JS_GetPropertyStr(h->c, promise, "then");
            JS_FreeValue(h->c, JS_Call(h->c, then, promise, 2, h->funcs));
            JS_FreeValue(h->c, then);
        }
        JS_FreeValue(h->c, promise);
        JS_FreeValue(h->c, h->funcs[0]); JS_FreeValue(h->c, h->funcs[1]);
        free(h->base); free(h->spec); free(h);
    }
    // Imports held meanwhile (by the modules just run) join the ones still waiting.
    while (keep) { Held *h = keep; keep = h->next; h->next = held; held = h; }
    run_jobs();
    left = 0;
    for (Held *h = held; h; h = h->next) left++;
    deadline_ns = 0;
    return left;
}

EXPORT("sx_init") int sx_init(int memory_limit) {
    sexos_baseline_jit_enabled = host_jit_available() == 2;
    rt = JS_NewRuntime();
    if (!rt) return -1;
    if (memory_limit > 0) JS_SetMemoryLimit(rt, (size_t)memory_limit);
    JS_SetMaxStackSize(rt, 256 * 1024);
    JS_SetInterruptHandler(rt, interrupt, NULL);
    JS_SetHostPromiseRejectionTracker(rt, promise_tracker, NULL);
    JS_SetModuleLoaderFunc(rt, module_normalize, module_load, NULL);
    js_dynamic_import_hook = hold_import;
    ctx = JS_NewContext(rt);
    if (!ctx) return -1;
    JSValue global = JS_GetGlobalObject(ctx);
    JS_SetPropertyStr(ctx, global, "__host", JS_NewCFunction(ctx, js_host, "__host", 1));
    JS_SetPropertyStr(ctx, global, "__sx_promise_setup", JS_NewCFunction(ctx, promise_setup, "promise setup", 1));
    JS_SetPropertyStr(ctx, global, "__sx_promise_flags", JS_NewCFunction(ctx, promise_flags, "promise flags", 1));
    JS_SetPropertyStr(ctx, global, "__sx_checkpoint_empty", JS_NewCFunction(ctx, checkpoint_empty, "checkpoint empty", 0));
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

// Deliver a classic resource's exception without passing its source through a
// page-controlled eval function. Opaque cross-origin values are never exposed.
static void report_script(int mode) {
    JSValue error = JS_GetException(ctx);
    JSValue global = JS_GetGlobalObject(ctx);
    JSValue dispatch = JS_GetPropertyStr(ctx, global, "__sx_dispatch");
    JSValue args[2] = { JS_NewString(ctx, mode == 2 ? "script-error-muted" : "script-error"), mode == 2 ? JS_NULL : JS_DupValue(ctx, error) };
    JSValue result = JS_Call(ctx, dispatch, JS_UNDEFINED, 2, args);
    if (JS_IsException(result)) report();
    JS_FreeValue(ctx, result); JS_FreeValue(ctx, args[0]); JS_FreeValue(ctx, args[1]);
    JS_FreeValue(ctx, dispatch); JS_FreeValue(ctx, global); JS_FreeValue(ctx, error);
}

// Evaluates source as a classic script. 0 on success, -1 after reporting an exception.
EXPORT("sx_eval") int sx_eval(const char *src, int len, const char *name, int name_len, int budget_ms, int error_mode) {
    char file[128];
    int fl = name_len < 127 ? name_len : 127;
    memcpy(file, name, (size_t)fl); file[fl] = 0;
    // JS_Eval needs a terminating zero after the source.
    char *code = malloc((size_t)len + 1);
    if (!code) return -1;
    memcpy(code, src, (size_t)len); code[len] = 0;
    deadline_ns = budget_ms > 0 ? now_ns() + (int64_t)budget_ms * 1000000 : 0;
    int previous_origin = sexos_compile_origin;
    sexos_compile_origin = error_mode ? error_mode == 2 : source_muted(name, (size_t)name_len);
    if (sexos_compile_origin) sexos_opaque_source_seen = 1;
    JSValue v = JS_Eval(ctx, code, (size_t)len, file, JS_EVAL_TYPE_GLOBAL);
    sexos_compile_origin = previous_origin;
    free(code);
    int ok = 0;
    if (JS_IsException(v)) { if (error_mode) report_script(error_mode); else report(); ok = -1; }
    JS_FreeValue(ctx, v);
    run_jobs();
    deadline_ns = 0;
    return ok;
}

// A module script: `discover` 1 only finds the modules it needs that the host lacks
// (the host notes them as module_load asks), in a throwaway context, running nothing;
// 0 runs it. `name` is its address, the base for its imports. 0 on success, -1 after
// reporting an exception (or, discovering, on any failure).
EXPORT("sx_module") int sx_module(const char *src, int len, const char *name, int name_len, int budget_ms, int discover) {
    char *file = malloc((size_t)name_len + 1);
    char *code = malloc((size_t)len + 1);
    if (!file || !code) { free(file); free(code); return -1; }
    memcpy(file, name, (size_t)name_len); file[name_len] = 0;
    memcpy(code, src, (size_t)len); code[len] = 0;
    deadline_ns = budget_ms > 0 ? now_ns() + (int64_t)budget_ms * 1000000 : 0;
    int previous_origin = sexos_compile_origin;
    sexos_compile_origin = source_muted(file, (size_t)name_len);
    int ok = 0;
    if (discover) {
        JSContext *t = JS_NewContext(rt);
        if (!t) ok = -1;
        else {
            discovering = 1;
            JSValue f = JS_Eval(t, code, (size_t)len, file, JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
            if (JS_IsException(f) || JS_ResolveModule(t, f) < 0) {
                ok = -1;
                JS_FreeValue(t, JS_GetException(t));
            }
            JS_FreeValue(t, f);
            discovering = 0;
            JS_FreeContext(t);
        }
    } else {
        // Compiled first, so the entry module's import.meta.url is set before it runs.
        JSValue v = JS_Eval(ctx, code, (size_t)len, file, JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
        if (!JS_IsException(v)) {
            JSValue meta = JS_GetImportMeta(ctx, JS_VALUE_GET_PTR(v));
            if (!JS_IsException(meta)) JS_DefinePropertyValueStr(ctx, meta, "url", JS_NewString(ctx, file), JS_PROP_C_W_E);
            JS_FreeValue(ctx, meta);
            v = JS_EvalFunction(ctx, v);
        }
        if (JS_IsException(v)) { report(); ok = -1; }
        else {
            run_jobs();
            // A module's evaluation is a promise; a rejection is the module's error.
            if (JS_IsPromise(v) && JS_PromiseState(ctx, v) == JS_PROMISE_REJECTED) {
                JS_Throw(ctx, JS_PromiseResult(ctx, v));
                report();
                ok = -1;
            }
        }
        JS_FreeValue(ctx, v);
    }
    sexos_compile_origin = previous_origin;
    free(file);
    free(code);
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
        if (len == 4 && !memcmp(arg, "tick", 4)) {
            JSValue query = JS_NewString(ctx, "next-timer");
            JSValue next = JS_Call(ctx, fn, global, 1, &query);
            if (JS_IsException(next)) { report(); out = -1; }
            else JS_ToInt32(ctx, &out, next);
            JS_FreeValue(ctx, next); JS_FreeValue(ctx, query);
        }
        deadline_ns = 0;
    }
    JS_FreeValue(ctx, fn);
    JS_FreeValue(ctx, global);
    return out;
}
