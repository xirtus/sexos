/* SPDX-License-Identifier: GPL-3.0-only */
/* The Rust side of SDL's SexOS backends (crates/sex-sdl/src/lib.rs): the program's
 * window, its keys and pointer, and its sound ring, as sex-app gives them. */
#ifndef SEXOS_SHIM_H
#define SEXOS_SHIM_H
#include <stdint.h>

enum {
    SEX_SDL_NONE = 0,
    SEX_SDL_KEY = 1,     /* code (set-1 or Linux-normalised), shift, control, text */
    SEX_SDL_POINTER = 2, /* x, y, buttons, kind (1 move 2 down 3 up 4 wheel 5 leave), detail */
    SEX_SDL_CLOSE = 3,
    SEX_SDL_RESIZE = 4,  /* x = width, y = height */
};

struct sex_sdl_event {
    int32_t kind;
    int32_t code;
    int32_t shift;
    int32_t control;
    int32_t text;        /* the character the key types, or 0 */
    int32_t x, y;
    int32_t buttons;
    int32_t pointer;
    int32_t detail;
};

/* The window: opened on the first call, then the same one. Returns its pixel area
 * (0x00RRGGBB, row-major at the width shown) and its length in pixels. */
uint32_t *sex_sdl_window_pixels(uint64_t *len);
void sex_sdl_window_size(int32_t *w, int32_t *h);
void sex_sdl_window_present(int32_t w, int32_t h);
void sex_sdl_window_title(const char *title);
/* 1 and the event, or 0 when nothing is waiting. */
int32_t sex_sdl_poll(struct sex_sdl_event *out);

/* Sound: stereo i16 frames at `rate` into the program's sound ring. */
int32_t sex_sdl_audio_open(uint32_t rate);
uint64_t sex_sdl_audio_write(const int16_t *frames, uint64_t count);
uint64_t sex_sdl_audio_room(void);
uint64_t sex_sdl_audio_pending(void);
void sex_sdl_audio_close(void);

/* A line in the serial record. */
void sex_sdl_record(const char *line);
#endif
