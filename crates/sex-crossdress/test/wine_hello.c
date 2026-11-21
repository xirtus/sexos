// SPDX-License-Identifier: GPL-3.0-only
// CW3 test program: a 64-bit Windows console program for Wine's own KERNEL32 in a
// Crossdress cell (Wine's ntdll loads it and its DLLs; KERNEL32 calls end in NT system
// calls the host serves). No C runtime; documented KERNEL32 only, so it runs on Windows.
typedef void *HANDLE;
typedef int BOOL;
typedef unsigned long DWORD;
typedef void *HMODULE;
typedef long long (__stdcall *FARPROC)(void);
__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD);
__declspec(dllimport) BOOL __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
__declspec(dllimport) void __stdcall ExitProcess(unsigned);
__declspec(dllimport) HANDLE __stdcall GetProcessHeap(void);
__declspec(dllimport) void *__stdcall HeapAlloc(HANDLE, DWORD, unsigned long long);
__declspec(dllimport) BOOL __stdcall HeapFree(HANDLE, DWORD, void *);
__declspec(dllimport) HMODULE __stdcall GetModuleHandleA(const char *);
__declspec(dllimport) FARPROC __stdcall GetProcAddress(HMODULE, const char *);
__declspec(dllimport) int __stdcall lstrlenA(const char *);
__declspec(dllimport) char *__stdcall GetCommandLineA(void);
__declspec(dllimport) DWORD __stdcall GetCurrentProcessId(void);

static HANDLE out;
static int failures;
static void put(const char *s) { DWORD n; WriteFile(out, s, (DWORD)lstrlenA(s), &n, 0); }
static void check(int ok, const char *what) { put(ok ? "ok   " : "FAIL "); put(what); put("\n"); if (!ok) failures++; }

void start(void) {
    out = GetStdHandle((DWORD)-11);
    put("crossdress: hello from Wine's kernel32\n");
    HMODULE k32 = GetModuleHandleA("kernel32.dll");
    check(k32 != 0, "kernel32.dll is loaded");
    check(GetProcAddress(k32, "HeapAlloc") != 0, "GetProcAddress finds HeapAlloc");
    check(GetProcAddress(k32, "NoSuchFunction") == 0, "GetProcAddress refuses a missing name");
    check(GetModuleHandleA("ntdll.dll") != 0, "ntdll.dll is loaded");
    char *p = HeapAlloc(GetProcessHeap(), 8, 4096);
    int zero = p != 0;
    for (int i = 0; p && i < 4096; i++) zero &= p[i] == 0;
    check(zero, "HeapAlloc on Wine's process heap gives zeroed memory");
    check(p && HeapFree(GetProcessHeap(), 0, p), "HeapFree");
    char *cmd = GetCommandLineA();
    check(cmd && lstrlenA(cmd) > 0, "GetCommandLineA");
    check(GetCurrentProcessId() != 0, "GetCurrentProcessId");
    put(failures ? "crossdress: wine checks FAILED\n" : "crossdress: wine checks passed\n");
    ExitProcess(failures);
}
