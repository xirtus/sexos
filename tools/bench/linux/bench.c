// SPDX-License-Identifier: GPL-3.0-only
// The same tests as apps/bench, as a Linux program, so both systems can be timed on the
// same emulated machine. Build static and run as /init in an initramfs:
//   gcc -O2 -static -o init bench.c
// Each line: [bench] name=... ops=... cycles=... per=...
#define _GNU_SOURCE
#include <fcntl.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/reboot.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/wait.h>
#include <unistd.h>
#include <x86intrin.h>

#define MIB (1024 * 1024)
static void add(const char *name, unsigned long ops, unsigned long cycles, const char *unit) {
    printf("[bench] name=%s ops=%lu cycles=%lu per=%s\n", name, ops, cycles / (ops ? ops : 1), unit);
    fflush(stdout);
}

int main(void) {
    mount("devtmpfs", "/dev", "devtmpfs", 0, 0);
    mount("tmpfs", "/tmp", "tmpfs", 0, 0);
    // The stock kernel builds NVMe as modules: load any the initramfs carries, in order.
    const char *mods[] = {"/mods/nvme-keyring.ko", "/mods/nvme-auth.ko", "/mods/nvme-core.ko", "/mods/nvme.ko"};
    for (int i = 0; i < 4; i++) { int m = open(mods[i], O_RDONLY); if (m >= 0) { syscall(SYS_finit_module, m, "", 0); close(m); } }
    unsigned long n, t;
    n = 20000; t = __rdtsc();
    for (unsigned long i = 0; i < n; i++) syscall(SYS_getppid);
    add("null_syscall", n, __rdtsc() - t, "call");

    // What a protection-key switch costs on this machine (SexOS does three per call).
    // Needs PKU in the CPU and kernel; skipped otherwise.
    unsigned int a, b, c, d;
    __asm__ volatile("cpuid" : "=a"(a), "=b"(b), "=c"(c), "=d"(d) : "a"(7), "c"(0));
    if (c & (1u << 4)) {  // OSPKE: the kernel turned keys on
        n = 2000; t = __rdtsc();
        for (unsigned long i = 0; i < n; i++) __asm__ volatile("wrpkru" :: "a"(0), "c"(0), "d"(0));
        add("wrpkru", n, __rdtsc() - t, "write");
    } else printf("[bench.note] no OSPKE\n");

    n = 2000; t = __rdtsc();
    for (unsigned long i = 0; i < n; i++) sched_yield();
    add("yield", n, __rdtsc() - t, "yield");

    // One byte to another process through a pipe and one byte back: the round trip a
    // SexOS call and reply replaces (ipc_roundtrip).
    int there[2], back[2];
    if (pipe(there) == 0 && pipe(back) == 0) {
        pid_t child = fork();
        if (child == 0) {
            close(there[1]); close(back[0]);
            char c;
            while (read(there[0], &c, 1) == 1) write(back[1], &c, 1);
            _exit(0);
        }
        close(there[0]); close(back[1]);
        char c = 'x';
        n = 2000; t = __rdtsc();
        for (unsigned long i = 0; i < n; i++) { write(there[1], &c, 1); read(back[0], &c, 1); }
        add("ipc_roundtrip", n, __rdtsc() - t, "trip");
        close(there[1]);
        waitpid(child, NULL, 0);
    }

    t = __rdtsc();
    unsigned char *big = malloc(32 * MIB);
    memset(big, 1, 32 * MIB);
    add("alloc_touch", 32, __rdtsc() - t, "MiB");
    unsigned char *copy = malloc(8 * MIB); memset(copy, 0, 8 * MIB);
    t = __rdtsc();
    for (int i = 0; i < 4; i++) memcpy(copy, big + i * MIB, 8 * MIB);
    add("memcpy", 32, __rdtsc() - t, "MiB");

    // Files on the same kind of disk SexOS uses (NVMe), raw, so no filesystem is
    // needed in the initramfs; and on tmpfs, which is what Linux programs usually hit.
    const char *disk = "/dev/nvme0n1";
    int fd = -1;
    for (int i = 0; i < 100 && fd < 0; i++) { fd = open(disk, O_RDWR); if (fd < 0) usleep(100000); }
    if (fd >= 0) {
        t = __rdtsc();
        for (int off = 0; off < 4 * MIB; off += 64 * 1024) pwrite(fd, big + off, 64 * 1024, 64 * MIB + off);
        fsync(fd);
        add("file_write", 4, __rdtsc() - t, "MiB");
        unsigned char *back = malloc(4 * MIB);
        posix_fadvise(fd, 0, 0, POSIX_FADV_DONTNEED);
        t = __rdtsc();
        for (int off = 0; off < 4 * MIB; off += 64 * 1024) pread(fd, back + off, 64 * 1024, 64 * MIB + off);
        add("file_read", 4, __rdtsc() - t, "MiB");
        close(fd);
    } else printf("[bench.note] no %s\n", disk);
    fd = open("/tmp/bench.dat", O_CREAT | O_RDWR | O_TRUNC, 0644);
    t = __rdtsc();
    for (int off = 0; off < 4 * MIB; off += 64 * 1024) write(fd, big + off, 64 * 1024);
    fsync(fd);
    add("tmpfs_write", 4, __rdtsc() - t, "MiB");
    close(fd);
    struct stat st;
    n = 500; t = __rdtsc();
    for (unsigned long i = 0; i < n; i++) stat("/tmp/bench.dat", &st);
    add("file_stat", n, __rdtsc() - t, "call");
    printf("[bench.done]\n"); fflush(stdout);
    sync();
    reboot(RB_POWER_OFF);
    return 0;
}
