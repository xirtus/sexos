/* SPDX-License-Identifier: GPL-3.0-only
 * The native tier's runtime: what wasm2c's output of quickjs.wasm needs (memory,
 * tables, traps), plus the entry points sex-js calls. Linear memory keeps explicit
 * bounds checks, so scripts stay confined to it as they were under wasmi. Memory
 * comes from Rust's allocator (sexjs_rt_realloc). Traps unwind to the entry point
 * with __builtin_setjmp/longjmp; no Rust frame is ever between the two. */
#include "qjs.h"
#include <math.h>

void *sexjs_rt_realloc(void *ptr, size_t old_size, size_t new_size); /* zeroes growth */

WASM_RT_THREAD_LOCAL uint32_t wasm_rt_call_stack_depth;
static WASM_RT_THREAD_LOCAL void **catcher; /* the innermost entry point's jump buffer */
static WASM_RT_THREAD_LOCAL int trap_code;
/* PRIVATE bounded diagnostic, read only after the trap has unwound. */
const char *sexjs_debug_frames[1025];
uintptr_t sexjs_debug_sp[1025];
static uint32_t debug_entry_depth, debug_trap_depth, debug_unbalanced;
static uintptr_t debug_entry_sp;
static uint32_t debug_trap_entry;
static uintptr_t debug_trap_entry_sp;
uint32_t sexjs_debug_highwater;
static uint32_t debug_trap_highwater;
uint32_t sxn_diag_value(uint32_t key) {
  switch (key) {
    case 0: return debug_trap_entry;
    case 1: return debug_trap_depth;
    case 2: return debug_unbalanced;
    case 3: return debug_trap_highwater;
    default: return 0;
  }
}
const char *sxn_diag_name(uint32_t depth) { return depth <= 1024 ? sexjs_debug_frames[depth] : 0; }
uintptr_t sxn_diag_distance(uint32_t depth) {
  uintptr_t at = depth <= 1024 ? sexjs_debug_sp[depth] : 0;
  return !at ? 0 : at > debug_trap_entry_sp ? at - debug_trap_entry_sp : debug_trap_entry_sp - at;
}

bool wasm_rt_is_initialized(void) { return true; }

void wasm_rt_trap(wasm_rt_trap_t code) {
  debug_trap_depth = wasm_rt_call_stack_depth;
  debug_trap_entry = debug_entry_depth;
  debug_trap_entry_sp = debug_entry_sp;
  debug_trap_highwater = sexjs_debug_highwater;
  trap_code = (int)code == 0 ? 1 : (int)code;
  __builtin_longjmp(catcher, 1);
}

void sexjs_abort(void) { wasm_rt_trap(WASM_RT_TRAP_UNREACHABLE); }

void wasm_rt_allocate_memory(wasm_rt_memory_t *m, uint64_t pages, uint64_t max_pages, bool is64
#ifdef SEXJS_WASM_CUSTOM_PAGES
                             , uint32_t page_size
#endif
                             ) {
#ifdef SEXJS_WASM_CUSTOM_PAGES
  if (page_size != 65536) wasm_rt_trap(WASM_RT_TRAP_OOB);
  m->page_size = page_size;
#endif
  m->data = sexjs_rt_realloc(0, 0, pages << 16);
  if (!m->data && pages) wasm_rt_trap(WASM_RT_TRAP_OOB);
  m->pages = pages; m->max_pages = max_pages; m->size = pages << 16; m->is64 = is64;
}

uint64_t wasm_rt_grow_memory(wasm_rt_memory_t *m, uint64_t delta) {
  uint64_t old = m->pages, now = old + delta;
  if (now < old || now > m->max_pages) return (uint64_t)-1;
  if (delta) {
    uint8_t *d = sexjs_rt_realloc(m->data, old << 16, now << 16);
    if (!d) return (uint64_t)-1;
    m->data = d; m->pages = now; m->size = now << 16;
  }
  return old;
}

void wasm_rt_free_memory(wasm_rt_memory_t *m) { sexjs_rt_realloc(m->data, m->size, 0); m->data = 0; }

void wasm_rt_allocate_funcref_table(wasm_rt_funcref_table_t *t, uint32_t elements, uint32_t max) {
  t->data = sexjs_rt_realloc(0, 0, (size_t)elements * sizeof(wasm_rt_funcref_t));
  t->size = elements; t->max_size = max;
}

void wasm_rt_free_funcref_table(wasm_rt_funcref_table_t *t) {
  sexjs_rt_realloc(t->data, (size_t)t->size * sizeof(wasm_rt_funcref_t), 0); t->data = 0;
}

/* Entry points. Each returns 0, or the trap code if the engine trapped (it is unusable
 * then, as under wasmi). Nested entries (a page op that calls back in) restore the
 * outer catcher. */
#define ENTER                                                   \
  void *jb[5]; void **outer = catcher; uint32_t depth = wasm_rt_call_stack_depth; \
  uint32_t old_debug_depth = debug_entry_depth, old_highwater = sexjs_debug_highwater; \
  uintptr_t old_debug_sp = debug_entry_sp;                       \
  debug_entry_depth = depth; debug_entry_sp = (uintptr_t)__builtin_frame_address(0); \
  debug_trap_depth = 0;                                         \
  sexjs_debug_highwater = depth;                                \
  catcher = jb;                                                 \
  if (__builtin_setjmp(jb)) { catcher = outer; wasm_rt_call_stack_depth = depth; \
    debug_entry_depth = old_debug_depth; debug_entry_sp = old_debug_sp; sexjs_debug_highwater = old_highwater; return trap_code; }
#define LEAVE if (wasm_rt_call_stack_depth != depth) ++debug_unbalanced; \
  debug_entry_depth = old_debug_depth; debug_entry_sp = old_debug_sp; sexjs_debug_highwater = old_highwater; catcher = outer; return 0;

size_t sxn_size(void) { return sizeof(w2c_sexjs); }

int sxn_instantiate(w2c_sexjs *i, void *ctx, uint64_t max_bytes) {
  ENTER
  wasm2c_sexjs_instantiate(i, ctx, ctx);
  /* The caller's cap holds from the start, as under wasmi's limiter. */
  if (i->w2c_memory.size > max_bytes) { catcher = outer; return WASM_RT_TRAP_OOB; }
  if (max_bytes >> 16 < i->w2c_memory.max_pages) i->w2c_memory.max_pages = max_bytes >> 16;
  w2c_sexjs_0x5Finitialize(i);
  LEAVE
}
void sxn_free(w2c_sexjs *i) { wasm2c_sexjs_free(i); }
uint8_t *sxn_memory(w2c_sexjs *i, uint64_t *size) { *size = i->w2c_memory.size; return i->w2c_memory.data; }

int sxn_init(w2c_sexjs *i, u32 heap, u32 *out) { ENTER *out = w2c_sexjs_sx_init(i, heap); LEAVE }
int sxn_alloc(w2c_sexjs *i, u32 n, u32 *out) { ENTER *out = w2c_sexjs_sx_alloc(i, n); LEAVE }
int sxn_release(w2c_sexjs *i, u32 p) { ENTER w2c_sexjs_sx_free(i, p); LEAVE }
int sxn_eval(w2c_sexjs *i, u32 a, u32 b, u32 c, u32 d, u32 e, u32 f, u32 *out) { ENTER *out = w2c_sexjs_sx_eval(i, a, b, c, d, e, f); LEAVE }
int sxn_module(w2c_sexjs *i, u32 a, u32 b, u32 c, u32 d, u32 e, u32 f, u32 *out) { ENTER *out = w2c_sexjs_sx_module(i, a, b, c, d, e, f); LEAVE }
int sxn_ready(w2c_sexjs *i, u32 a, u32 *out) { ENTER *out = w2c_sexjs_sx_modules_ready(i, a); LEAVE }
int sxn_dispatch(w2c_sexjs *i, u32 a, u32 b, u32 c, u32 *out) { ENTER *out = w2c_sexjs_sx_dispatch(i, a, b, c); LEAVE }

/* Round to nearest, ties to even (the default mode; nothing changes it). Not every
 * target's builtins have nearbyint, and SSE2 has no rounding instruction. */
double sexjs_nearbyint(double x) {
  const double big = 4503599627370496.0; /* 2^52 */
  if (!(__builtin_fabs(x) < big)) return x;
  double r = x >= 0 ? (x + big) - big : (x - big) + big;
  return __builtin_copysign(r, x);
}
float sexjs_nearbyintf(float x) {
  const float big = 8388608.0f; /* 2^23 */
  if (!(__builtin_fabsf(x) < big)) return x;
  float r = x >= 0 ? (x + big) - big : (x - big) + big;
  return __builtin_copysignf(r, x);
}
