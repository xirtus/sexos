/* SPDX-License-Identifier: GPL-3.0-only
 * CPython 3.13's configuration for SexOS's ordinary-program target, written by hand
 * (no configure run). It started from what configure finds for static x86-64 musl and
 * was cut down to what sex-libc provides: every HAVE_ here names a function sex-libc
 * implements or a musl header the build compiles against. Everything else (fork, exec,
 * signal delivery, users and groups, sockets, dlopen, mmap, symlinks...) is compiled
 * out of CPython, so Python code sees it as unavailable on this platform. */
#ifndef Py_PYCONFIG_H
#define Py_PYCONFIG_H

#define ALIGNOF_LONG 8
#define ALIGNOF_MAX_ALIGN_T 16
#define ALIGNOF_SIZE_T 8
#define DOUBLE_IS_LITTLE_ENDIAN_IEEE754 1
#define HAVE_ACOSH 1
#define HAVE_ALLOCA_H 1
#define HAVE_ASINH 1
#define HAVE_ATANH 1
#define HAVE_BUILTIN_ATOMIC 1
#define HAVE_CLOCK 1
#define HAVE_CLOCK_GETTIME 1
#define HAVE_CLOCK_T 1
#define HAVE_COMPUTED_GOTOS 1
#define HAVE_DIRENT_D_TYPE 1
#define HAVE_DIRENT_H 1
#define HAVE_DUP 1
#define HAVE_ENDIAN_H 1
#define HAVE_ERF 1
#define HAVE_ERFC 1
#define HAVE_ERRNO_H 1
#define HAVE_EXPM1 1
#define HAVE_FCNTL_H 1
#define HAVE_FDATASYNC 1
#define HAVE_FSEEKO 1
#define HAVE_FSYNC 1
#define HAVE_FTELLO 1
#define HAVE_FTRUNCATE 1
#define HAVE_GCC_ASM_FOR_X64 1
#define HAVE_GCC_UINT128_T 1
#define HAVE_GETENTROPY 1
#define HAVE_GETRANDOM 1
#define HAVE_INTTYPES_H 1
#define HAVE_LANGINFO_H 1
#define HAVE_LOG1P 1
#define HAVE_LOG2 1
#define HAVE_LONG_DOUBLE 1
#define HAVE_LSTAT 1
#define HAVE_MBRTOWC 1
#define HAVE_MKTIME 1
#define HAVE_NANOSLEEP 1
#define HAVE_OPENDIR 1
#define HAVE_PREAD 1
#define HAVE_PROTOTYPES 1
#define HAVE_PTHREAD_CONDATTR_SETCLOCK 1
#define HAVE_PTHREAD_H 1
#define HAVE_PWRITE 1
#define HAVE_SCHED_H 1
#define HAVE_SETJMP_H 1
#define HAVE_SETLOCALE 1
#define HAVE_SETVBUF 1
#define HAVE_SIGNAL_H 1
#define HAVE_SNPRINTF 1
#define HAVE_SSIZE_T 1
#define HAVE_STAT_TV_NSEC 1
#define HAVE_STDINT_H 1
#define HAVE_STDIO_H 1
#define HAVE_STDLIB_H 1
#define HAVE_STD_ATOMIC 1
#define HAVE_STRFTIME 1
#define HAVE_STRINGS_H 1
#define HAVE_STRING_H 1
#define HAVE_STRUCT_STAT_ST_BLKSIZE 1
#define HAVE_STRUCT_STAT_ST_BLOCKS 1
#define HAVE_STRUCT_STAT_ST_RDEV 1
#define HAVE_STRUCT_TM_TM_ZONE 1
#define HAVE_SYSCONF 1
#define HAVE_SYS_FILE_H 1
#define HAVE_SYS_PARAM_H 1
#define HAVE_SYS_RANDOM_H 1
#define HAVE_SYS_SELECT_H 1
#define HAVE_SYS_TIMES_H 1
#define HAVE_SYS_STAT_H 1
#define HAVE_SYS_TIME_H 1
#define HAVE_SYS_TYPES_H 1
#define HAVE_SYS_UIO_H 1
#define HAVE_TIMEGM 1
#define HAVE_TIMES 1
#define HAVE_TMPFILE 1
#define HAVE_TM_ZONE 1
#define HAVE_TRUNCATE 1
#define HAVE_UNISTD_H 1
#define HAVE_UTIME_H 1
#define HAVE_WCHAR_H 1
#define HAVE___UINT128_T 1
#define PTHREAD_KEY_T_IS_COMPATIBLE_WITH_INT 1
#define PY_SUPPORT_TIER 0
#define RETSIGTYPE void
#define SIZEOF_DOUBLE 8
#define SIZEOF_FLOAT 4
#define SIZEOF_FPOS_T 16
#define SIZEOF_INT 4
#define SIZEOF_LONG 8
#define SIZEOF_LONG_DOUBLE 16
#define SIZEOF_LONG_LONG 8
#define SIZEOF_OFF_T 8
#define SIZEOF_PID_T 4
#define SIZEOF_PTHREAD_KEY_T 4
#define SIZEOF_PTHREAD_T 8
#define SIZEOF_SHORT 2
#define SIZEOF_SIZE_T 8
#define SIZEOF_TIME_T 8
#define SIZEOF_UINTPTR_T 8
#define SIZEOF_VOID_P 8
#define SIZEOF_WCHAR_T 4
#define SIZEOF__BOOL 1
#define STDC_HEADERS 1
#define SYS_SELECT_WITH_SYS_TIME 1
#define WITH_DOC_STRINGS 1
#define WITH_FREELISTS 1
#define _FILE_OFFSET_BITS 64
#define _LARGEFILE_SOURCE 1
#define _POSIX_C_SOURCE 200809L
#define _PYTHONFRAMEWORK ""
#define _REENTRANT 1
#define _XOPEN_SOURCE 700
#define _XOPEN_SOURCE_EXTENDED 1
#define _GNU_SOURCE 1

/* pymalloc's small-object pools; with no mmap its arenas come from malloc. */
#define WITH_PYMALLOC 1

#endif /* Py_PYCONFIG_H */
