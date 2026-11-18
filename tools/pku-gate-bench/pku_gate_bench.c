// Phase 6 (PKU gates) microbenchmark. Runs on the Linux CI host, not in SexOS.
//
// It measures what a user-level PKU gate would cost on the runner's real CPU:
// RDPKRU, WRPKRU, an ERIM-style checked open/close pair, a full modelled gate
// (open, check, stack switch, call, switch back, close, check), the first touch
// of a protected page after opening it, and a Linux syscall round trip for scale.
// Numbers are TSC ticks per operation: median of REPS batches of ITERS each.
//
// Build: cc -O2 -o pku_gate_bench pku_gate_bench.c
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>
#include <x86intrin.h>
#include <cpuid.h>

#define ITERS 20000
#define REPS 15

static inline uint64_t t0(void) { _mm_lfence(); uint64_t t = __rdtsc(); _mm_lfence(); return t; }
static inline uint64_t t1(void) { unsigned aux; uint64_t t = __rdtscp(&aux); _mm_lfence(); return t; }

static inline uint32_t rdpkru(void) {
    uint32_t eax, edx;
    __asm__ volatile(".byte 0x0f,0x01,0xee" : "=a"(eax), "=d"(edx) : "c"(0));
    return eax;
}
static inline void wrpkru(uint32_t v) {
    __asm__ volatile(".byte 0x0f,0x01,0xef" :: "a"(v), "c"(0), "d"(0) : "memory");
}
static volatile uint32_t EXPECT_OPEN, EXPECT_CLOSED;

// ERIM-style checked write: after WRPKRU, compare EAX with an expected value held in
// memory (an operand independent of the value written), so a jump straight to the
// WRPKRU with another EAX lands in ud2. The real stubs (crates/sex-gate) compare with an
// immediate; a memory operand is the closest this C bench can get with runtime keys.
static inline void wrpkru_checked(uint32_t v, const volatile uint32_t *expect) {
    __asm__ volatile(
        ".byte 0x0f,0x01,0xef\n\t"
        "cmp %1, %%eax\n\t"
        "je 1f\n\t"
        "ud2\n"
        "1:"
        :: "a"(v), "m"(*expect), "c"(0), "d"(0) : "memory", "cc");
}

static volatile uint64_t sink;
__attribute__((noinline)) static void service(void) { sink++; }

static uint8_t gate_stack[1 << 16] __attribute__((aligned(64)));

// Open, check, switch to the service stack, call, switch back, close, check.
static inline void gate_call(uint32_t open, uint32_t closed) {
    wrpkru_checked(open, &EXPECT_OPEN);
    __asm__ volatile(
        "mov %%rsp, %%rbx\n\t"
        "mov %0, %%rsp\n\t"
        "call *%1\n\t"
        "mov %%rbx, %%rsp\n\t"
        :: "r"(gate_stack + sizeof gate_stack), "r"(service)
        : "rbx", "rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11", "memory", "cc");
    wrpkru_checked(closed, &EXPECT_CLOSED);
}

static int cmp_u64(const void *a, const void *b) {
    uint64_t x = *(const uint64_t *)a, y = *(const uint64_t *)b;
    return x < y ? -1 : x > y;
}

typedef void (*body_fn)(void);
static uint32_t OPEN, CLOSED;
static volatile uint8_t *PAGE;

static void b_empty(void) { __asm__ volatile("" ::: "memory"); }
static void b_rdpkru(void) { sink += rdpkru(); }
static void b_wrpkru_same(void) { wrpkru(CLOSED); }
static void b_open_close(void) { wrpkru(OPEN); wrpkru(CLOSED); }
static void b_open_close_checked(void) { wrpkru_checked(OPEN, &EXPECT_OPEN); wrpkru_checked(CLOSED, &EXPECT_CLOSED); }
static void b_gate(void) { gate_call(OPEN, CLOSED); }
static void b_gate_touch(void) { wrpkru_checked(OPEN, &EXPECT_OPEN); sink += PAGE[0]; PAGE[64] = 1; wrpkru_checked(CLOSED, &EXPECT_CLOSED); }
static void b_call_only(void) {
    __asm__ volatile(
        "mov %%rsp, %%rbx\n\t" "mov %0, %%rsp\n\t" "call *%1\n\t" "mov %%rbx, %%rsp\n\t"
        :: "r"(gate_stack + sizeof gate_stack), "r"(service)
        : "rbx", "rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11", "memory", "cc");
}
static void b_syscall(void) { sink += syscall(SYS_getppid); }

static double measure(body_fn f, int iters) {
    uint64_t r[REPS];
    for (int i = 0; i < 1000; i++) f();
    for (int k = 0; k < REPS; k++) {
        uint64_t a = t0();
        for (int i = 0; i < iters; i++) f();
        uint64_t b = t1();
        r[k] = b - a;
    }
    qsort(r, REPS, sizeof r[0], cmp_u64);
    return (double)r[REPS / 2] / iters;
}

int main(void) {
    unsigned a, b, c, d;
    char brand[49] = {0};
    for (unsigned i = 0; i < 3; i++) {
        __cpuid(0x80000002 + i, a, b, c, d);
        memcpy(brand + 16 * i, &a, 4); memcpy(brand + 16 * i + 4, &b, 4);
        memcpy(brand + 16 * i + 8, &c, 4); memcpy(brand + 16 * i + 12, &d, 4);
    }
    __cpuid_count(7, 0, a, b, c, d);
    int pku = (c >> 3) & 1, ospke = (c >> 4) & 1;
    printf("[pku.bench] cpu=\"%s\" pku=%d ospke=%d\n", brand, pku, ospke);
    if (!pku || !ospke) { printf("[pku.bench] SKIP no usable PKU on this host\n"); return 0; }

    long key = syscall(SYS_pkey_alloc, 0, 0);
    if (key < 1) { printf("[pku.bench] SKIP pkey_alloc failed\n"); return 0; }
    PAGE = mmap(NULL, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_POPULATE, -1, 0);
    if (PAGE == MAP_FAILED || syscall(SYS_pkey_mprotect, (void *)PAGE, 4096, PROT_READ | PROT_WRITE, key)) {
        printf("[pku.bench] SKIP pkey_mprotect failed\n"); return 0;
    }
    uint32_t base = rdpkru();
    OPEN = base & ~(3u << (2 * key));       // key readable + writable
    CLOSED = base | (3u << (2 * key));      // access + write disabled
    EXPECT_OPEN = OPEN;
    EXPECT_CLOSED = CLOSED;
    wrpkru(CLOSED);

    struct { const char *name; body_fn f; int iters; } cases[] = {
        {"empty_loop", b_empty, ITERS},
        {"rdpkru", b_rdpkru, ITERS},
        {"wrpkru_same_value", b_wrpkru_same, ITERS},
        {"wrpkru_open_close", b_open_close, ITERS},
        {"wrpkru_open_close_checked", b_open_close_checked, ITERS},
        {"stack_switch_call_only", b_call_only, ITERS},
        {"gate_call_model", b_gate, ITERS},
        {"gate_open_touch_close", b_gate_touch, ITERS},
        {"linux_syscall_getppid", b_syscall, ITERS / 10},
    };
    double empty = 0;
    for (unsigned i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        double v = measure(cases[i].f, cases[i].iters);
        if (i == 0) empty = v;
        printf("[pku.bench] %-28s tsc_per_op=%8.1f net=%8.1f\n", cases[i].name, v, v - empty);
    }
    wrpkru(base);
    return 0;
}
