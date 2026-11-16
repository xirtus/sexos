/* SPDX-License-Identifier: GPL-3.0-only
 * Crossdress CW1 test program: a real 64-bit Windows console program with no C runtime,
 * built with clang + lld-link (build.sh). It only uses documented KERNEL32 functions, so
 * it runs on Windows too, and its output there is the reference for the cell's.
 * Checks: console output, the TEB/PEB through gs, VirtualAlloc anywhere and at a fixed
 * low address, and four hardware exceptions caught by a vectored exception handler. */
typedef unsigned long long u64;
typedef unsigned long DWORD;
typedef void *HANDLE;
typedef int BOOL;
typedef struct { DWORD code; DWORD flags; void *next; void *address; DWORD params; u64 info[15]; } EXCEPTION_RECORD;
typedef struct { EXCEPTION_RECORD *record; unsigned char *context; } EXCEPTION_POINTERS;
typedef long (*VEH)(EXCEPTION_POINTERS *);

__declspec(dllimport) HANDLE GetStdHandle(DWORD);
__declspec(dllimport) BOOL WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) __attribute__((noreturn)) void ExitProcess(unsigned);
__declspec(dllimport) void *AddVectoredExceptionHandler(DWORD, VEH);
__declspec(dllimport) void *VirtualAlloc(void *, u64, DWORD, DWORD);
__declspec(dllimport) DWORD GetCurrentThreadId(void);
__declspec(dllimport) u64 GetTickCount64(void);

extern unsigned char __ImageBase[];
static const char read_only[] = "this string lives in .rdata";
/* An absolute pointer in data: needs a DIR64 base relocation when the image moves. */
static const char *volatile absolute = read_only;

static HANDLE out;
static int failed, passed;
static volatile u64 skip;
static volatile DWORD last_code;
static volatile u64 last_kind, last_address, last_rip;

static unsigned len(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void put(const char *s) { DWORD w; WriteFile(out, s, len(s), &w, 0); }
static void hex(u64 v) {
    char b[19]; b[0] = '0'; b[1] = 'x';
    for (int i = 0; i < 16; i++) { unsigned d = (v >> (60 - 4 * i)) & 15; b[2 + i] = d < 10 ? '0' + d : 'a' + d - 10; }
    b[18] = 0; put(b);
}
static void check(int ok, const char *what) {
    put(ok ? "ok   " : "FAIL "); put(what); put("\n");
    if (ok) passed++; else failed++;
}

static long handler(EXCEPTION_POINTERS *e) {
    last_code = e->record->code;
    last_kind = e->record->params > 0 ? e->record->info[0] : 99;
    last_address = e->record->params > 1 ? e->record->info[1] : 0;
    u64 *rip = (u64 *)(e->context + 0xF8);
    last_rip = *rip;
    *rip += skip;              /* step over the faulting instruction */
    return -1;                 /* EXCEPTION_CONTINUE_EXECUTION */
}

static void read_at(void *p) { skip = 2; __asm__ volatile("movl (%%rax), %%eax" :: "a"(p) : "memory"); }
static void write_at(void *p) { skip = 3; __asm__ volatile("movb $1, (%%rax)" :: "a"(p) : "memory"); }
static void breakpoint(void) { skip = 1; __asm__ volatile("int3"); }
static void divide_by_zero(void) { skip = 2; __asm__ volatile("xorl %%ecx, %%ecx\n\tdivl %%ecx" ::: "eax", "ecx", "edx"); }

void start(void) {
    out = GetStdHandle((DWORD)-11);
    put("crossdress: hello from a Windows program\n");

    u64 teb, peb;
    __asm__ volatile("movq %%gs:0x30, %0" : "=r"(teb));
    __asm__ volatile("movq %%gs:0x60, %0" : "=r"(peb));
    put("teb "); hex(teb); put(" peb "); hex(peb); put(" image "); hex((u64)__ImageBase); put("\n");
    check(teb != 0 && *(u64 *)(teb + 0x30) == teb, "TEB in gs points at itself");
    check(peb != 0 && *(u64 *)(peb + 0x10) == (u64)__ImageBase, "PEB ImageBaseAddress is this image");
    check((u64)__ImageBase == 0x140000000ull, "loaded at its preferred base");
    check(absolute == read_only, "absolute pointers in data point into the loaded image");
    check(GetCurrentThreadId() != 0 && *(DWORD *)(teb + 0x48) == GetCurrentThreadId(), "thread id matches the TEB");

    unsigned char *mem = VirtualAlloc(0, 1 << 16, 0x3000, 4);  /* MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE */
    int same = mem != 0;
    for (int i = 0; same && i < (1 << 16); i += 4096) mem[i] = (unsigned char)i ^ 0x5A;
    for (int i = 0; same && i < (1 << 16); i += 4096) same = mem[i] == ((unsigned char)i ^ 0x5A);
    put("virtualalloc "); hex((u64)mem); put("\n");
    check(same, "VirtualAlloc gives writable memory");
    unsigned char *low = VirtualAlloc((void *)0x10000000, 1 << 16, 0x3000, 4);
    check(low == (void *)0x10000000 && (low[100] = 7, low[100] == 7), "VirtualAlloc at a fixed low address");
    void *reserved = VirtualAlloc(0, 1 << 16, 0x2000, 1);       /* MEM_RESERVE only, PAGE_NOACCESS */
    check(reserved != 0, "VirtualAlloc reserves without committing");

    check(AddVectoredExceptionHandler(1, handler) != 0, "vectored exception handler added");
    read_at(0);
    check(last_code == 0xC0000005 && last_kind == 0 && last_address == 0, "null read is an access violation (read, 0x0)");
    write_at((void *)read_only);
    check(last_code == 0xC0000005 && last_kind == 1 && last_address == (u64)read_only, "write to .rdata is an access violation (write)");
    read_at(reserved);
    check(last_code == 0xC0000005 && last_address == (u64)reserved, "reserved memory is not accessible");
    breakpoint();
    check(last_code == 0x80000003, "int3 is a breakpoint exception");
    divide_by_zero();
    check(last_code == 0xC0000094, "divide by zero is caught");
    check(GetTickCount64() >= 0, "GetTickCount64 answers");

    put(failed ? "crossdress: FAILED " : "crossdress: all checks passed "); hex(passed); put("\n");
    ExitProcess(failed);
}
