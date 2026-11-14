/* SPDX-License-Identifier: GPL-3.0-only
 * The SexOS side of CPython: its `main`, and the path configuration that replaces
 * Modules/getpath.c (which would need a frozen getpath.py and a file system that looks
 * like an installed Python).
 *
 * A SexOS program's own folder is its file system (`/` to C code). The standard
 * library is frozen into the binary (apps/python/frozen), so nothing has to be
 * installed there; `/lib` is searched for any further pure-Python modules a person
 * puts into the folder, after the directory of the script being run. */
#include "Python.h"
#include "pycore_initconfig.h"
#include "pycore_pathconfig.h"

#define SET(field, value)                                                     \
    if (config->field == NULL) {                                              \
        status = PyConfig_SetString(config, &config->field, (value));         \
        if (_PyStatus_EXCEPTION(status)) { return status; }                   \
    }

PyStatus
_PyConfig_InitPathConfig(PyConfig *config, int compute_path_config)
{
    PyStatus status = _PyPathConfig_ReadGlobal(config);
    if (_PyStatus_EXCEPTION(status) || !compute_path_config) {
        return status;
    }
    SET(program_name, L"python");
    SET(executable, L"/python");
    SET(base_executable, config->executable);
    SET(prefix, L"/");
    SET(exec_prefix, L"/");
    SET(base_prefix, L"/");
    SET(base_exec_prefix, L"/");
    SET(platlibdir, L"lib");
    SET(stdlib_dir, L"/lib/python3.13");
    if (!config->module_search_paths_set) {
        status = PyWideStringList_Append(&config->module_search_paths, L"/lib");
        if (_PyStatus_EXCEPTION(status)) {
            return status;
        }
        config->module_search_paths_set = 1;
    }
    return _PyStatus_OK();
}

PyObject *
_Py_Get_Getpath_CodeObject(void)
{
    PyErr_SetString(PyExc_RuntimeError, "getpath.py is not part of this build");
    return NULL;
}

/* `python ARGS` exactly as anywhere else (Py_BytesMain), in UTF-8 mode: SexOS has one
 * locale and its text is UTF-8. Nothing sends a program signals, so CPython installs
 * no handlers. */
#include <stdlib.h>
int
sex_python_main(int argc, char **argv)
{
    /* A program's window prints text and interprets no escape sequences: no colour
     * in tracebacks, and the plain line-by-line REPL rather than the full-screen one. */
    setenv("TERM", "dumb", 0);
    setenv("PYTHON_BASIC_REPL", "1", 0);

    PyStatus status;
    PyPreConfig preconfig;
    PyPreConfig_InitPythonConfig(&preconfig);
    preconfig.utf8_mode = 1;
    status = Py_PreInitializeFromBytesArgs(&preconfig, argc, argv);
    if (PyStatus_Exception(status)) {
        Py_ExitStatusException(status);
    }

    PyConfig config;
    PyConfig_InitPythonConfig(&config);
    config.install_signal_handlers = 0;
    status = PyConfig_SetBytesArgv(&config, argc, argv);
    if (PyStatus_Exception(status)) {
        goto fail;
    }
    status = Py_InitializeFromConfig(&config);
    if (PyStatus_Exception(status)) {
        goto fail;
    }
    PyConfig_Clear(&config);
    return Py_RunMain();

fail:
    PyConfig_Clear(&config);
    if (PyStatus_IsExit(status)) {
        return status.exitcode;
    }
    Py_ExitStatusException(status);
}
