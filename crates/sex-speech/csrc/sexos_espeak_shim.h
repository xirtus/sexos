/* SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Force-included (clang -include) before every eSpeak NG source file.
 *
 * eSpeak NG reads its data (phontab, phonindex, phondata, intonations, en_dict and the
 * voice file lang/gmw/en) with fopen/fread/fgets and checks paths with stat. In SexOS
 * the voice is a confined program with no file system: the data is compiled into the
 * program (crates/sex-speech/espeak-ng-data, embedded by src/lib.rs) and these
 * macros send eSpeak's file calls to memory streams over it (sexos_espeak_shim.c).
 * Any path that is not embedded data is refused (ENOENT), so eSpeak can neither read
 * nor write anything else. Streams the shim did not make (stderr) pass through.
 */
#ifndef SEXOS_ESPEAK_SHIM_H
#define SEXOS_ESPEAK_SHIM_H

#include <stdio.h>
#include <sys/stat.h>

/* The data root eSpeak is pointed at; only paths under it can resolve. */
#define SEXOS_ESPEAK_ROOT "/sexos/espeak-ng-data"

FILE *sexos_espeak_fopen(const char *path, const char *mode);
int sexos_espeak_fclose(FILE *f);
size_t sexos_espeak_fread(void *out, size_t size, size_t n, FILE *f);
char *sexos_espeak_fgets(char *out, int n, FILE *f);
int sexos_espeak_fgetc(FILE *f);
int sexos_espeak_fseek(FILE *f, long at, int whence);
long sexos_espeak_ftell(FILE *f);
int sexos_espeak_feof(FILE *f);
int sexos_espeak_stat(const char *path, struct stat *st);

#ifndef SEXOS_ESPEAK_SHIM_IMPL
#define fopen(p, m) sexos_espeak_fopen(p, m)
#define fclose(f) sexos_espeak_fclose(f)
#define fread(o, s, n, f) sexos_espeak_fread(o, s, n, f)
#define fgets(o, n, f) sexos_espeak_fgets(o, n, f)
#define fgetc(f) sexos_espeak_fgetc(f)
#define fseek(f, a, w) sexos_espeak_fseek(f, a, w)
#define ftell(f) sexos_espeak_ftell(f)
#define feof(f) sexos_espeak_feof(f)
#define stat(p, b) sexos_espeak_stat(p, b)
#endif

#endif
