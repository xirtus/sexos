/* SPDX-License-Identifier: GPL-3.0-only
 * SQLite's file layer for SexOS (SQLITE_OS_OTHER): files through sex-libc's
 * open/pread/pwrite, which reach only what the program was granted. A program's files
 * are its own (Files hands each grant to one program), so locks are bookkeeping only;
 * WAL and shared memory are compiled out (SQLITE_OMIT_WAL), so journals are the
 * classic rollback kind. */
#include "sqlite3.h"
#include <errno.h>
#include <fcntl.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/random.h>
#include <time.h>
#include <unistd.h>

typedef struct SexFile {
    sqlite3_file base;
    int fd;
    int lock;
    int delete_on_close;
    char *path;
} SexFile;

static int sexClose(sqlite3_file *f) {
    SexFile *p = (SexFile *)f;
    close(p->fd);
    if (p->delete_on_close && p->path) unlink(p->path);
    sqlite3_free(p->path);
    return SQLITE_OK;
}

static int sexRead(sqlite3_file *f, void *buf, int n, sqlite3_int64 at) {
    SexFile *p = (SexFile *)f;
    int got = 0;
    while (got < n) {
        ssize_t r = pread(p->fd, (char *)buf + got, n - got, at + got);
        if (r < 0) { if (errno == EINTR) continue; return SQLITE_IOERR_READ; }
        if (r == 0) break;
        got += (int)r;
    }
    if (got < n) { memset((char *)buf + got, 0, n - got); return SQLITE_IOERR_SHORT_READ; }
    return SQLITE_OK;
}

static int sexWrite(sqlite3_file *f, const void *buf, int n, sqlite3_int64 at) {
    SexFile *p = (SexFile *)f;
    int put = 0;
    while (put < n) {
        ssize_t r = pwrite(p->fd, (const char *)buf + put, n - put, at + put);
        if (r < 0) { if (errno == EINTR) continue; return errno == ENOSPC ? SQLITE_FULL : SQLITE_IOERR_WRITE; }
        if (r == 0) return SQLITE_IOERR_WRITE;
        put += (int)r;
    }
    return SQLITE_OK;
}

static int sexTruncate(sqlite3_file *f, sqlite3_int64 size) {
    return ftruncate(((SexFile *)f)->fd, size) ? SQLITE_IOERR_TRUNCATE : SQLITE_OK;
}

static int sexSync(sqlite3_file *f, int flags) {
    (void)flags;
    return fsync(((SexFile *)f)->fd) ? SQLITE_IOERR_FSYNC : SQLITE_OK;
}

static int sexFileSize(sqlite3_file *f, sqlite3_int64 *size) {
    struct stat st;
    if (fstat(((SexFile *)f)->fd, &st)) return SQLITE_IOERR_FSTAT;
    *size = st.st_size;
    return SQLITE_OK;
}

static int sexLock(sqlite3_file *f, int level) { ((SexFile *)f)->lock = level; return SQLITE_OK; }
static int sexUnlock(sqlite3_file *f, int level) { ((SexFile *)f)->lock = level; return SQLITE_OK; }
static int sexCheckReservedLock(sqlite3_file *f, int *out) { (void)f; *out = 0; return SQLITE_OK; }
static int sexFileControl(sqlite3_file *f, int op, void *arg) { (void)f; (void)op; (void)arg; return SQLITE_NOTFOUND; }
static int sexSectorSize(sqlite3_file *f) { (void)f; return 4096; }
static int sexDeviceCharacteristics(sqlite3_file *f) { (void)f; return SQLITE_IOCAP_SAFE_APPEND | SQLITE_IOCAP_POWERSAFE_OVERWRITE; }

static const sqlite3_io_methods sex_io = {
    1, sexClose, sexRead, sexWrite, sexTruncate, sexSync, sexFileSize, sexLock, sexUnlock,
    sexCheckReservedLock, sexFileControl, sexSectorSize, sexDeviceCharacteristics,
};

static int temp_counter;

static int sexOpen(sqlite3_vfs *vfs, sqlite3_filename name, sqlite3_file *f, int flags, int *out_flags) {
    (void)vfs;
    SexFile *p = (SexFile *)f;
    memset(p, 0, sizeof *p);
    p->fd = -1;
    char temp[64];
    const char *path = name;
    if (!path) {
        /* A temporary file: a fresh name in the program's own folder, removed on close. */
        sqlite3_snprintf(sizeof temp, temp, ".sqlite-temp-%d", ++temp_counter);
        path = temp;
        flags |= SQLITE_OPEN_DELETEONCLOSE | SQLITE_OPEN_CREATE | SQLITE_OPEN_READWRITE;
    }
    int oflags = (flags & SQLITE_OPEN_READWRITE) ? O_RDWR : O_RDONLY;
    if (flags & SQLITE_OPEN_CREATE) oflags |= O_CREAT;
    if (flags & SQLITE_OPEN_EXCLUSIVE) oflags |= O_EXCL;
    int fd = open(path, oflags, 0644);
    if (fd < 0 && (flags & SQLITE_OPEN_READWRITE) && !(flags & SQLITE_OPEN_CREATE)) {
        fd = open(path, O_RDONLY);
        if (fd >= 0) flags = (flags & ~SQLITE_OPEN_READWRITE) | SQLITE_OPEN_READONLY;
    }
    if (fd < 0) return SQLITE_CANTOPEN;
    p->fd = fd;
    p->delete_on_close = (flags & SQLITE_OPEN_DELETEONCLOSE) != 0;
    if (p->delete_on_close) p->path = sqlite3_mprintf("%s", path);
    p->base.pMethods = &sex_io;
    if (out_flags) *out_flags = flags;
    return SQLITE_OK;
}

static int sexDelete(sqlite3_vfs *vfs, const char *path, int sync_dir) {
    (void)vfs; (void)sync_dir;
    if (unlink(path) && errno != ENOENT) return SQLITE_IOERR_DELETE;
    return SQLITE_OK;
}

static int sexAccess(sqlite3_vfs *vfs, const char *path, int what, int *out) {
    (void)vfs;
    *out = access(path, what == SQLITE_ACCESS_EXISTS ? F_OK : R_OK | (what == SQLITE_ACCESS_READWRITE ? W_OK : 0)) == 0;
    return SQLITE_OK;
}

static int sexFullPathname(sqlite3_vfs *vfs, const char *path, int n, char *out) {
    (void)vfs;
    if (path[0] == '/' || !getcwd(out, n)) { sqlite3_snprintf(n, out, "%s", path); return SQLITE_OK; }
    size_t len = strlen(out);
    sqlite3_snprintf(n - (int)len, out + len, "%s%s", (len && out[len - 1] == '/') ? "" : "/", path);
    return SQLITE_OK;
}

static int sexRandomness(sqlite3_vfs *vfs, int n, char *out) {
    (void)vfs;
    int got = 0;
    while (got < n) {
        int k = n - got > 256 ? 256 : n - got;
        if (getentropy(out + got, k)) break;
        got += k;
    }
    return got;
}

static int sexSleep(sqlite3_vfs *vfs, int us) { (void)vfs; usleep(us); return us; }

static int sexCurrentTimeInt64(sqlite3_vfs *vfs, sqlite3_int64 *out) {
    (void)vfs;
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    /* Julian day in milliseconds. */
    *out = (sqlite3_int64)210866760000000LL + (sqlite3_int64)ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
    return SQLITE_OK;
}

static int sexCurrentTime(sqlite3_vfs *vfs, double *out) {
    sqlite3_int64 ms;
    sexCurrentTimeInt64(vfs, &ms);
    *out = ms / 86400000.0;
    return SQLITE_OK;
}

static int sexGetLastError(sqlite3_vfs *vfs, int n, char *out) { (void)vfs; (void)n; (void)out; return errno; }

static sqlite3_vfs sex_vfs = {
    2, sizeof(SexFile), 512, 0, "sexos", 0,
    sexOpen, sexDelete, sexAccess, sexFullPathname, 0, 0, 0, 0,
    sexRandomness, sexSleep, sexCurrentTime, sexGetLastError, sexCurrentTimeInt64,
};

int sqlite3_os_init(void) { return sqlite3_vfs_register(&sex_vfs, 1); }
int sqlite3_os_end(void) { return SQLITE_OK; }
