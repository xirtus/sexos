/* SPDX-License-Identifier: GPL-3.0-only */
/* sdltest: an ordinary SDL2 program. It opens a window and draws four coloured
 * quarters and a white middle with SDL_FillRect + SDL_UpdateWindowSurface. It plays
 * one second of a 440 Hz tone through SDL_OpenAudio + SDL_QueueAudio, checks the
 * timer, waits for a key and reports each step on standard output. */
#include <SDL.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>

#define W 320
#define H 240
#define RATE 48000

static void fill(SDL_Surface *s, int x, int y, int w, int h, Uint8 r, Uint8 g, Uint8 b)
{
    SDL_Rect rect = { x, y, w, h };
    SDL_FillRect(s, &rect, SDL_MapRGB(s->format, r, g, b));
}

static Uint32 pixel(SDL_Surface *s, int x, int y)
{
    return ((Uint32 *)((Uint8 *)s->pixels + y * s->pitch))[x] & 0xffffff;
}

int main(int argc, char **argv)
{
    SDL_version v;
    SDL_Window *window;
    SDL_Surface *surface;
    SDL_AudioSpec want, got;
    Sint16 *tone;
    Uint32 start, t0, t1, left;
    int i, seconds = argc > 1 ? atoi(argv[1]) : 120;

    if (SDL_Init(SDL_INIT_VIDEO | SDL_INIT_AUDIO | SDL_INIT_TIMER) != 0) {
        printf("[sdltest] SDL_Init failed: %s\n", SDL_GetError());
        return 1;
    }
    SDL_GetVersion(&v);
    printf("[sdltest] init version=%d.%d.%d video=%s audio=%s\n", v.major, v.minor, v.patch,
           SDL_GetCurrentVideoDriver(), SDL_GetCurrentAudioDriver());

    window = SDL_CreateWindow("sdltest", SDL_WINDOWPOS_UNDEFINED, SDL_WINDOWPOS_UNDEFINED, W, H, 0);
    if (!window) {
        printf("[sdltest] SDL_CreateWindow failed: %s\n", SDL_GetError());
        return 1;
    }
    surface = SDL_GetWindowSurface(window);
    if (!surface) {
        printf("[sdltest] SDL_GetWindowSurface failed: %s\n", SDL_GetError());
        return 1;
    }
    fill(surface, 0, 0, W / 2, H / 2, 0xe0, 0x20, 0x20);         /* red */
    fill(surface, W / 2, 0, W / 2, H / 2, 0x20, 0xc0, 0x40);     /* green */
    fill(surface, 0, H / 2, W / 2, H / 2, 0x20, 0x40, 0xe0);     /* blue */
    fill(surface, W / 2, H / 2, W / 2, H / 2, 0xf0, 0xd0, 0x20); /* yellow */
    fill(surface, W / 2 - 20, H / 2 - 20, 40, 40, 0xff, 0xff, 0xff);
    if (SDL_UpdateWindowSurface(window) != 0) {
        printf("[sdltest] SDL_UpdateWindowSurface failed: %s\n", SDL_GetError());
        return 1;
    }
    printf("[sdltest] drew w=%d h=%d format=%s tl=%06x tr=%06x bl=%06x br=%06x mid=%06x\n",
           surface->w, surface->h, SDL_GetPixelFormatName(surface->format->format),
           (unsigned)pixel(surface, 10, 10), (unsigned)pixel(surface, W - 10, 10),
           (unsigned)pixel(surface, 10, H - 10), (unsigned)pixel(surface, W - 10, H - 10),
           (unsigned)pixel(surface, W / 2, H / 2));

    t0 = SDL_GetTicks();
    SDL_Delay(100);
    t1 = SDL_GetTicks();
    printf("[sdltest] timer delay100=%u\n", (unsigned)(t1 - t0));

    SDL_zero(want);
    want.freq = RATE;
    want.format = AUDIO_S16SYS;
    want.channels = 2;
    want.samples = 1024;
    want.callback = NULL;
    if (SDL_OpenAudio(&want, &got) != 0) {
        printf("[sdltest] SDL_OpenAudio failed: %s\n", SDL_GetError());
        return 1;
    }
    tone = (Sint16 *)SDL_malloc(RATE * 2 * sizeof(Sint16));
    for (i = 0; i < RATE; i++) {
        Sint16 s = (Sint16)(9000.0 * sin(2.0 * M_PI * 440.0 * i / RATE));
        tone[2 * i] = s;
        tone[2 * i + 1] = s;
    }
    if (SDL_QueueAudio(1, tone, RATE * 2 * sizeof(Sint16)) != 0) {
        printf("[sdltest] SDL_QueueAudio failed: %s\n", SDL_GetError());
        return 1;
    }
    SDL_free(tone);
    printf("[sdltest] audio open freq=%d channels=%d samples=%d queued=%u\n", got.freq, got.channels,
           got.samples, (unsigned)SDL_GetQueuedAudioSize(1));
    SDL_PauseAudio(0);
    start = SDL_GetTicks();
    while ((left = SDL_GetQueuedAudioSize(1)) > 0 && SDL_GetTicks() - start < 10000) {
        SDL_Delay(10);
    }
    printf("[sdltest] audio drained left=%u ms=%u\n", (unsigned)left, (unsigned)(SDL_GetTicks() - start));
    SDL_CloseAudio();

    printf("[sdltest] waiting for key\n");
    fflush(stdout);
    start = SDL_GetTicks();
    for (;;) {
        SDL_Event e;
        if (SDL_GetTicks() - start > (Uint32)seconds * 1000) {
            printf("[sdltest] no key\n");
            return 1;
        }
        if (!SDL_WaitEventTimeout(&e, 100)) {
            continue;
        }
        if (e.type == SDL_QUIT) {
            printf("[sdltest] quit before a key\n");
            return 1;
        }
        if (e.type == SDL_KEYDOWN) {
            printf("[sdltest] key down scancode=%d name=%s sym=%s\n", (int)e.key.keysym.scancode,
                   SDL_GetScancodeName(e.key.keysym.scancode), SDL_GetKeyName(e.key.keysym.sym));
        }
        if (e.type == SDL_TEXTINPUT) {
            printf("[sdltest] text %s\n", e.text.text);
        }
        if (e.type == SDL_KEYUP) {
            printf("[sdltest] key up scancode=%d\n", (int)e.key.keysym.scancode);
            break;
        }
    }
    SDL_DestroyWindow(window);
    SDL_Quit();
    printf("[sdltest] done\n");
    return 0;
}
