/* SPDX-License-Identifier: GPL-3.0-only */
/* SDL2 video and input for SexOS: the program's own window (sex_app::Window).
 *
 * SDL_CreateWindow opens the window. Its framebuffer (SDL_GetWindowSurface) is the
 * window's pixel area itself when it fits, as XRGB8888, so SDL_UpdateWindowSurface
 * only tells Display which part to show. Keys, pointer and close come from Shell
 * through the window and become SDL events in SDL_PumpEvents.
 *
 * It takes the slot of SDL's dummy video driver (SDL_VIDEO_DRIVER_DUMMY), so that
 * SDL_video.c stays as it is. Its name is "sexos". */
#include "SDL_internal.h"

#include "SDL_video.h"
#include "SDL_mouse.h"
#include "video/SDL_sysvideo.h"
#include "video/SDL_pixels_c.h"
#include "events/SDL_events_c.h"
#include "events/SDL_keyboard_c.h"
#include "events/SDL_mouse_c.h"
#include "events/SDL_windowevents_c.h"
#include "events/scancodes_linux.h"

#include "sexos_shim.h"

#define FRAMEBUFFER "_SDL_SexOSFramebuffer"

/* One window per program: the SDL window that holds it. */
static SDL_Window *shown_window;
static Uint8 held_buttons;

typedef struct
{
    Uint32 *pixels;   /* the framebuffer SDL draws into */
    SDL_bool direct;  /* it is the window's pixel area itself */
    int w, h;
} Framebuffer;

static int SEXOS_VideoInit(_THIS)
{
    SDL_DisplayMode mode;
    int32_t w = 0, h = 0;
    sex_sdl_window_size(&w, &h);
    SDL_zero(mode);
    mode.format = SDL_PIXELFORMAT_RGB888;
    mode.w = w > 0 ? w : 640;
    mode.h = h > 0 ? h : 400;
    mode.refresh_rate = 60;
    if (SDL_AddBasicVideoDisplay(&mode) < 0) {
        return -1;
    }
    SDL_AddDisplayMode(&_this->displays[0], &mode);
    return 0;
}

static void SEXOS_VideoQuit(_THIS)
{
    (void)_this;
}

static int SEXOS_CreateWindow(_THIS, SDL_Window *window)
{
    (void)_this;
    if (shown_window) {
        return SDL_SetError("A SexOS program has one window");
    }
    shown_window = window;
    window->x = 0;
    window->y = 0;
    sex_sdl_window_title(window->title ? window->title : "SDL");
    SDL_SetKeyboardFocus(window);
    SDL_SendWindowEvent(window, SDL_WINDOWEVENT_SHOWN, 0, 0);
    return 0;
}

static void SEXOS_SetWindowTitle(_THIS, SDL_Window *window)
{
    (void)_this;
    sex_sdl_window_title(window->title ? window->title : "");
}

static void SEXOS_DestroyWindow(_THIS, SDL_Window *window)
{
    (void)_this;
    if (window == shown_window) {
        shown_window = NULL;
    }
}

static void SEXOS_DestroyWindowFramebuffer(_THIS, SDL_Window *window)
{
    Framebuffer *fb = (Framebuffer *)SDL_SetWindowData(window, FRAMEBUFFER, NULL);
    (void)_this;
    if (fb) {
        if (!fb->direct) {
            SDL_free(fb->pixels);
        }
        SDL_free(fb);
    }
}

static int SEXOS_CreateWindowFramebuffer(_THIS, SDL_Window *window, Uint32 *format, void **pixels, int *pitch)
{
    uint64_t len = 0;
    Uint32 *area;
    Framebuffer *fb;
    int w, h;

    SEXOS_DestroyWindowFramebuffer(_this, window);
    SDL_GetWindowSizeInPixels(window, &w, &h);
    fb = (Framebuffer *)SDL_calloc(1, sizeof(*fb));
    if (!fb) {
        return SDL_OutOfMemory();
    }
    area = sex_sdl_window_pixels(&len);
    fb->w = w;
    fb->h = h;
    if (area && (uint64_t)w * (uint64_t)h <= len) {
        fb->pixels = area;
        fb->direct = SDL_TRUE;
    } else {
        fb->pixels = (Uint32 *)SDL_calloc((size_t)w * h, 4);
        if (!fb->pixels) {
            SDL_free(fb);
            return SDL_OutOfMemory();
        }
    }
    SDL_SetWindowData(window, FRAMEBUFFER, fb);
    *format = SDL_PIXELFORMAT_RGB888;
    *pixels = fb->pixels;
    *pitch = w * 4;
    return 0;
}

static int SEXOS_UpdateWindowFramebuffer(_THIS, SDL_Window *window, const SDL_Rect *rects, int numrects)
{
    Framebuffer *fb = (Framebuffer *)SDL_GetWindowData(window, FRAMEBUFFER);
    (void)_this;
    (void)rects;
    (void)numrects;
    if (!fb) {
        return SDL_SetError("This window has no framebuffer");
    }
    if (!fb->direct) {
        /* Too big for the pixel area: show as much as fits, at the same width. */
        uint64_t len = 0;
        Uint32 *area = sex_sdl_window_pixels(&len);
        int rows = fb->w > 0 ? (int)SDL_min((uint64_t)fb->h, len / (uint64_t)fb->w) : 0;
        if (area && rows > 0) {
            SDL_memcpy(area, fb->pixels, (size_t)fb->w * rows * 4);
        }
        sex_sdl_window_present(fb->w, rows);
        return 0;
    }
    sex_sdl_window_present(fb->w, fb->h);
    return 0;
}

static void send_key(const struct sex_sdl_event *e)
{
    /* Shell passes PS/2 set-1 codes; below 0x59 set-1 and Linux codes
     * agree, so Linux's table maps them. */
    SDL_Scancode code = e->code >= 0 && e->code < (int)SDL_arraysize(linux_scancode_table)
                            ? linux_scancode_table[e->code] : SDL_SCANCODE_UNKNOWN;
    /* Shell passes the grey movement keys as their set-1 codes without the E0 prefix,
     * which the Linux table reads as the keypad: report them as the movement keys. */
    switch (e->code) {
    case 0x47: code = SDL_SCANCODE_HOME; break;
    case 0x48: code = SDL_SCANCODE_UP; break;
    case 0x49: code = SDL_SCANCODE_PAGEUP; break;
    case 0x4b: code = SDL_SCANCODE_LEFT; break;
    case 0x4d: code = SDL_SCANCODE_RIGHT; break;
    case 0x4f: code = SDL_SCANCODE_END; break;
    case 0x50: code = SDL_SCANCODE_DOWN; break;
    case 0x51: code = SDL_SCANCODE_PAGEDOWN; break;
    case 0x52: code = SDL_SCANCODE_INSERT; break;
    case 0x53: code = SDL_SCANCODE_DELETE; break;
    default: break;
    }
    /* Shell sends presses only, with the modifiers held at the time. */
    if (e->shift) {
        SDL_SendKeyboardKey(SDL_PRESSED, SDL_SCANCODE_LSHIFT);
    }
    if (e->control) {
        SDL_SendKeyboardKey(SDL_PRESSED, SDL_SCANCODE_LCTRL);
    }
    if (code != SDL_SCANCODE_UNKNOWN) {
        SDL_SendKeyboardKey(SDL_PRESSED, code);
    }
    if (e->text > 0 && e->text < 0x80 && !e->control) {
        char text[2] = { (char)e->text, 0 };
        SDL_SendKeyboardText(text);
    }
    if (code != SDL_SCANCODE_UNKNOWN) {
        SDL_SendKeyboardKey(SDL_RELEASED, code);
    }
    if (e->control) {
        SDL_SendKeyboardKey(SDL_RELEASED, SDL_SCANCODE_LCTRL);
    }
    if (e->shift) {
        SDL_SendKeyboardKey(SDL_RELEASED, SDL_SCANCODE_LSHIFT);
    }
}

static Uint8 sdl_button(int button)
{
    switch (button) {
    case 1:
        return SDL_BUTTON_MIDDLE;
    case 2:
        return SDL_BUTTON_RIGHT;
    default:
        return SDL_BUTTON_LEFT;
    }
}

static void send_pointer(SDL_Window *window, const struct sex_sdl_event *e)
{
    if (e->pointer == 5) {
        if (!held_buttons) {
            SDL_SetMouseFocus(NULL);
        }
        return;
    }
    SDL_SetMouseFocus(window);
    SDL_SendMouseMotion(window, 0, 0, e->x, e->y);
    switch (e->pointer) {
    case 2:
        held_buttons |= (Uint8)(1 << e->detail);
        SDL_SendMouseButton(window, 0, SDL_PRESSED, sdl_button(e->detail));
        break;
    case 3:
        held_buttons &= (Uint8)~(1 << e->detail);
        SDL_SendMouseButton(window, 0, SDL_RELEASED, sdl_button(e->detail));
        break;
    case 4:
        SDL_SendMouseWheel(window, 0, 0.0f, (float)e->detail, SDL_MOUSEWHEEL_NORMAL);
        break;
    default:
        break;
    }
}

static void SEXOS_PumpEvents(_THIS)
{
    struct sex_sdl_event e;
    (void)_this;
    while (sex_sdl_poll(&e)) {
        switch (e.kind) {
        case SEX_SDL_KEY:
            send_key(&e);
            break;
        case SEX_SDL_POINTER:
            if (shown_window) {
                send_pointer(shown_window, &e);
            }
            break;
        case SEX_SDL_CLOSE:
            if (shown_window) {
                SDL_SendWindowEvent(shown_window, SDL_WINDOWEVENT_CLOSE, 0, 0);
            } else {
                SDL_SendQuit();
            }
            break;
        default:
            /* The pixel area changed size. The SDL window keeps its own size: what lies
             * past the area is cut off, and a larger area shows the window at its top left. */
            break;
        }
    }
}

static void SEXOS_DeleteDevice(SDL_VideoDevice *device)
{
    SDL_free(device);
}

static SDL_VideoDevice *SEXOS_CreateDevice(void)
{
    SDL_VideoDevice *device = (SDL_VideoDevice *)SDL_calloc(1, sizeof(SDL_VideoDevice));
    if (!device) {
        SDL_OutOfMemory();
        return NULL;
    }
    device->VideoInit = SEXOS_VideoInit;
    device->VideoQuit = SEXOS_VideoQuit;
    device->PumpEvents = SEXOS_PumpEvents;
    device->CreateSDLWindow = SEXOS_CreateWindow;
    device->SetWindowTitle = SEXOS_SetWindowTitle;
    device->DestroyWindow = SEXOS_DestroyWindow;
    device->CreateWindowFramebuffer = SEXOS_CreateWindowFramebuffer;
    device->UpdateWindowFramebuffer = SEXOS_UpdateWindowFramebuffer;
    device->DestroyWindowFramebuffer = SEXOS_DestroyWindowFramebuffer;
    device->free = SEXOS_DeleteDevice;
    return device;
}

VideoBootStrap DUMMY_bootstrap = {
    "sexos", "SexOS program window",
    SEXOS_CreateDevice,
    NULL
};
