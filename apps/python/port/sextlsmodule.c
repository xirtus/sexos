/* SPDX-License-Identifier: GPL-3.0-only
 * _sextls: TLS for Python's sockets on SexOS. The network service holds the TLS
 * session (sex-tls, rustls) for each connection; this module only asks for it on a
 * connected socket (sex-libc's sexos_tls_start). apps/python/lib/ssl.py builds the
 * ssl module's usual surface on it. */
#include "Python.h"
#include <errno.h>

extern int sexos_tls_start(int fd, const char *host);

static PyObject *CertificateError;

static PyObject *sextls_start(PyObject *self, PyObject *args) {
    int fd;
    const char *host;
    if (!PyArg_ParseTuple(args, "is:start", &fd, &host)) return NULL;
    int r;
    Py_BEGIN_ALLOW_THREADS
    r = sexos_tls_start(fd, host);
    Py_END_ALLOW_THREADS
    if (r == 0) Py_RETURN_NONE;
    if (errno == 129) {
        PyErr_Format(CertificateError, "certificate verify failed: %s is not trusted", host);
        return NULL;
    }
    return PyErr_SetFromErrno(PyExc_OSError);
}

static PyMethodDef methods[] = {
    {"start", sextls_start, METH_VARARGS, "start(fd, host): TLS on a connected socket, handshake done on return."},
    {NULL, NULL, 0, NULL},
};

static int exec_module(PyObject *m) {
    CertificateError = PyErr_NewException("_sextls.CertificateError", PyExc_OSError, NULL);
    if (!CertificateError || PyModule_AddObjectRef(m, "CertificateError", CertificateError) < 0) return -1;
    return 0;
}

static PyModuleDef_Slot slots[] = {
    {Py_mod_exec, exec_module},
    {Py_mod_multiple_interpreters, Py_MOD_MULTIPLE_INTERPRETERS_NOT_SUPPORTED},
    {0, NULL},
};

static struct PyModuleDef module = {PyModuleDef_HEAD_INIT, "_sextls", NULL, 0, methods, slots};

PyMODINIT_FUNC PyInit__sextls(void) { return PyModuleDef_Init(&module); }
