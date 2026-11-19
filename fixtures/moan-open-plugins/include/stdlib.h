/* SexOS plugin C surface: allocator only. See fixtures/moan-open-plugins/README.md. */
#ifndef SEXOS_PLUGIN_STDLIB_H
#define SEXOS_PLUGIN_STDLIB_H
#include <stddef.h>
void* malloc(size_t size);
void* calloc(size_t count, size_t size);
void* realloc(void* ptr, size_t size);
void free(void* ptr);
#endif
