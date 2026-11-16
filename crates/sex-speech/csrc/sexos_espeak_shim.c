/* SPDX-License-Identifier: GPL-3.0-or-later
 *
 * Memory streams over the embedded eSpeak NG data (see sexos_espeak_shim.h), and the
 * small glue the Rust side (crates/sex-speech/src/lib.rs) calls: synthesise one
 * utterance at a time into a Rust callback, each from a fresh start of eSpeak.
 */
#define SEXOS_ESPEAK_SHIM_IMPL
#include "sexos_espeak_shim.h"

#include <errno.h>
#include <stdlib.h>
#include <string.h>

#include <espeak-ng/espeak_ng.h>
#include <espeak-ng/speak_lib.h>

/* From crates/sex-speech/src/lib.rs: the embedded file named `name` (relative to the
 * data root, e.g. "phondata" or "lang/gmw/en"), or NULL. */
extern const unsigned char *sexos_speech_file(const char *name, size_t *len);
/* From lib.rs: hands `n` samples to the utterance's sink; nonzero stops synthesis. */
extern int sexos_speech_samples(void *sink, const short *wav, int n);
/* eSpeak NG's own generator (common.c), reseeded per utterance so output repeats. */
extern void espeak_srand(long seed);

struct mem_stream {
	const unsigned char *data;
	size_t len, at;
};

/* The directories eSpeak checks exist (stat says "directory"). */
static const char *const DIRS[] = { "", "lang", "lang/gmw" };

static const char *relative(const char *path)
{
	size_t n = strlen(SEXOS_ESPEAK_ROOT);
	if (path == NULL || strncmp(path, SEXOS_ESPEAK_ROOT, n) != 0)
		return NULL;
	if (path[n] == 0)
		return "";
	if (path[n] != '/')
		return NULL;
	return path + n + 1;
}

/* The open memory streams. eSpeak keeps at most a few open at once. */
#define MAX_OPEN 8
static struct mem_stream *open_streams[MAX_OPEN];

/* The stream `f` when this file made it; NULL for a real FILE (stderr). */
static struct mem_stream *ours(FILE *f)
{
	for (int i = 0; f != NULL && i < MAX_OPEN; i++)
		if (open_streams[i] == (struct mem_stream *)f)
			return open_streams[i];
	return NULL;
}

FILE *sexos_espeak_fopen(const char *path, const char *mode)
{
	const char *name = relative(path);
	size_t len = 0;
	const unsigned char *data = name ? sexos_speech_file(name, &len) : NULL;
	if (data == NULL || mode == NULL || mode[0] != 'r' || strchr(mode, '+') != NULL) {
		errno = data == NULL ? ENOENT : EACCES;
		return NULL;
	}
	for (int i = 0; i < MAX_OPEN; i++) {
		if (open_streams[i] == NULL) {
			struct mem_stream *m = malloc(sizeof *m);
			if (m == NULL) {
				errno = ENOMEM;
				return NULL;
			}
			m->data = data;
			m->len = len;
			m->at = 0;
			open_streams[i] = m;
			return (FILE *)m;
		}
	}
	errno = EMFILE;
	return NULL;
}

int sexos_espeak_fclose(FILE *f)
{
	for (int i = 0; i < MAX_OPEN; i++) {
		if (f != NULL && open_streams[i] == (struct mem_stream *)f) {
			open_streams[i] = NULL;
			free(f);
			return 0;
		}
	}
	return fclose(f);
}

size_t sexos_espeak_fread(void *out, size_t size, size_t n, FILE *f)
{
	struct mem_stream *m = ours(f);
	if (m == NULL)
		return fread(out, size, n, f);
	if (size == 0)
		return 0;
	size_t whole = (m->len - m->at) / size;
	if (n > whole)
		n = whole;
	memcpy(out, m->data + m->at, n * size);
	m->at += n * size;
	return n;
}

char *sexos_espeak_fgets(char *out, int n, FILE *f)
{
	struct mem_stream *m = ours(f);
	if (m == NULL)
		return fgets(out, n, f);
	if (n <= 0 || m->at >= m->len)
		return NULL;
	int i = 0;
	while (i < n - 1 && m->at < m->len) {
		char c = (char)m->data[m->at++];
		out[i++] = c;
		if (c == '\n')
			break;
	}
	out[i] = 0;
	return out;
}

int sexos_espeak_fgetc(FILE *f)
{
	struct mem_stream *m = ours(f);
	if (m == NULL)
		return fgetc(f);
	return m->at < m->len ? m->data[m->at++] : EOF;
}

int sexos_espeak_fseek(FILE *f, long at, int whence)
{
	struct mem_stream *m = ours(f);
	if (m == NULL)
		return fseek(f, at, whence);
	long base = whence == SEEK_SET ? 0 : whence == SEEK_CUR ? (long)m->at : (long)m->len;
	if (base + at < 0 || (size_t)(base + at) > m->len) {
		errno = EINVAL;
		return -1;
	}
	m->at = (size_t)(base + at);
	return 0;
}

long sexos_espeak_ftell(FILE *f)
{
	struct mem_stream *m = ours(f);
	return m == NULL ? ftell(f) : (long)m->at;
}

int sexos_espeak_feof(FILE *f)
{
	struct mem_stream *m = ours(f);
	return m == NULL ? feof(f) : m->at >= m->len;
}

int sexos_espeak_stat(const char *path, struct stat *st)
{
	const char *name = relative(path);
	if (name == NULL) {
		errno = ENOENT;
		return -1;
	}
	memset(st, 0, sizeof *st);
	for (size_t i = 0; i < sizeof DIRS / sizeof DIRS[0]; i++) {
		if (strcmp(name, DIRS[i]) == 0) {
			st->st_mode = S_IFDIR | 0555;
			return 0;
		}
	}
	size_t len = 0;
	if (sexos_speech_file(name, &len) == NULL) {
		errno = ENOENT;
		return -1;
	}
	st->st_mode = S_IFREG | 0444;
	st->st_size = (off_t)len;
	return 0;
}

/* ---- Glue for lib.rs ---- */

static int synth_callback(short *wav, int n, espeak_EVENT *events)
{
	void *sink = events ? events[0].user_data : NULL;
	if (sink == NULL)
		return 1;
	if (wav == NULL || n <= 0)
		return sexos_speech_samples(sink, NULL, 0);
	return sexos_speech_samples(sink, wav, n);
}

/* Starts eSpeak NG on the embedded data with the English voice. Returns the sample
 * rate (22050), or a negative eSpeak status. */
int sexos_speech_init(void)
{
	espeak_ng_ERROR_CONTEXT context = NULL;
	espeak_ng_InitializePath("/sexos");
	espeak_ng_STATUS status = espeak_ng_Initialize(&context);
	if (status != ENS_OK)
		return -(int)(status & 0x7fffffff);
	/* 100 ms buffers: the callback (and so a stop request) runs at least that often. */
	status = espeak_ng_InitializeOutput(ENOUTPUT_MODE_SYNCHRONOUS, 100, NULL);
	if (status != ENS_OK)
		return -(int)(status & 0x7fffffff);
	espeak_SetSynthCallback(synth_callback);
	status = espeak_ng_SetVoiceByName("gmw/en");
	if (status != ENS_OK)
		return -(int)(status & 0x7fffffff);
	return espeak_ng_GetSampleRate();
}

/* Speaks `len` bytes of UTF-8 `text` at `wpm` words a minute into `sink`.
 * Returns 0, or a negative eSpeak status. */
int sexos_speech_say(const char *text, size_t len, int wpm, void *sink)
{
	/* Every utterance starts eSpeak afresh, so the same text gives the same samples
	 * whatever was spoken before (eSpeak keeps speed, intonation and wave state
	 * between calls; the wave generator's own statics are reset by a SexOS patch
	 * in wavegen.c). The data is in memory, so this costs little. */
	espeak_ng_Terminate();
	int rate = sexos_speech_init();
	if (rate < 0)
		return rate;
	espeak_ng_STATUS status;
	espeak_ng_SetParameter(espeakRATE, wpm, 0);
	espeak_srand(1);
	status = espeak_ng_Synthesize(text, len + 1, 0, POS_CHARACTER, 0,
		espeakCHARS_UTF8, NULL, sink);
	/* A stop asked for through the callback ends synthesis this way. */
	return status == ENS_OK || status == ENS_SPEECH_STOPPED ? 0 : -(int)(status & 0x7fffffff);
}
