/* SPDX-License-Identifier: GPL-3.0-only
 * The built-in modules of SexOS's CPython (what configure would write from
 * Modules/config.c.in and Modules/Setup.*). Everything is linked into the one binary;
 * there are no extension modules to load. */
#include "Python.h"

extern PyObject* PyMarshal_Init(void);
extern PyObject* PyInit__imp(void);
extern PyObject* PyInit_gc(void);
extern PyObject* PyInit__ast(void);
extern PyObject* PyInit__tokenize(void);
extern PyObject* _PyWarnings_Init(void);
extern PyObject* PyInit__string(void);

#define MODULES(X) \
    X(posix) X(errno) X(_io) X(_codecs) X(_weakref) X(_abc) X(_functools)         \
    X(_collections) X(itertools) X(_operator) X(_signal) X(_stat) X(_thread)       \
    X(time) X(atexit) X(faulthandler) X(_tracemalloc) X(_suggestions) X(_sre)      \
    X(_sysconfig) X(_typing) X(_symtable) X(_opcode) X(math) X(cmath) X(_struct)   \
    X(_random) X(_json) X(_bisect) X(_heapq) X(binascii) X(array) X(_datetime)     \
    X(_contextvars) X(_queue) X(_statistics) X(_sha2)                     \
    X(_md5) X(_sha1) X(_sha3) X(_blake2) X(zlib) X(select) X(_socket) X(unicodedata) X(_sqlite3) X(_sextls)

#define DECLARE(name) extern PyObject* PyInit_##name(void);
MODULES(DECLARE)
#define ENTRY(name) {#name, PyInit_##name},

struct _inittab _PyImport_Inittab[] = {
    MODULES(ENTRY)
    {"marshal", PyMarshal_Init},
    {"_imp", PyInit__imp},
    {"_ast", PyInit__ast},
    {"_tokenize", PyInit__tokenize},
    {"builtins", NULL},
    {"sys", NULL},
    {"gc", PyInit_gc},
    {"_warnings", _PyWarnings_Init},
    {"_string", PyInit__string},
    {0, 0}
};
