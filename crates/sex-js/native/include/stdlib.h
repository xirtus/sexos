#pragma once
#include <stddef.h>
#define abort sexjs_abort
void sexjs_abort(void) __attribute__((noreturn));
void *malloc(size_t);
void free(void *);
