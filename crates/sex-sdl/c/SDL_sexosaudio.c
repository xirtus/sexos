/* SPDX-License-Identifier: GPL-3.0-only */
/* SDL2 audio output for SexOS: the program's own sound ring (sex_app::sound), which
 * Bell mixes into the sound card. The frames are stereo signed 16-bit, at the rate
 * the program asks for (8000 to 192000 Hz). Bell converts that rate to the card's.
 *
 * SDL's audio thread (a sex-libc pthread, which is one of the program's workers)
 * mixes a buffer and hands it to PlayDevice. PlayDevice copies it into the ring.
 * WaitDevice then sleeps until Bell has taken all but about a quarter of a second,
 * so the queue in SDL drains at the card's pace. When the device closes, the frames still
 * in the ring play out first. The serial record then gets one line:
 * `[sdl.audio] rate=R frames=N nonsilent=M`.
 *
 * This backend takes the slot of SDL's dummy audio driver (SDL_AUDIO_DRIVER_DUMMY).
 * Its name is "sexos". */
#include "SDL_internal.h"

#include "SDL_audio.h"
#include "SDL_timer.h"
#include "audio/SDL_audio_c.h"
#include "audio/SDL_sysaudio.h"

#include <stdio.h>
#include "sexos_shim.h"

#define _THIS SDL_AudioDevice *_this

struct SDL_PrivateAudioData
{
    Uint8 *mixbuf;
    Uint64 frames;     /* frames handed to the ring */
    Uint64 nonsilent;  /* of those, frames that were not silence */
};

static int SEXOSAUDIO_OpenDevice(_THIS, const char *devname)
{
    (void)devname;
    _this->hidden = (struct SDL_PrivateAudioData *)SDL_calloc(1, sizeof(*_this->hidden));
    if (!_this->hidden) {
        return SDL_OutOfMemory();
    }
    _this->spec.format = AUDIO_S16SYS;
    _this->spec.channels = 2;
    if (_this->spec.freq < 8000) {
        _this->spec.freq = 8000;
    } else if (_this->spec.freq > 192000) {
        _this->spec.freq = 192000;
    }
    SDL_CalculateAudioSpec(&_this->spec);
    _this->hidden->mixbuf = (Uint8 *)SDL_malloc(_this->spec.size);
    if (!_this->hidden->mixbuf) {
        return SDL_OutOfMemory();
    }
    SDL_memset(_this->hidden->mixbuf, _this->spec.silence, _this->spec.size);
    if (!sex_sdl_audio_open((uint32_t)_this->spec.freq)) {
        return SDL_SetError("This program has no sound ring");
    }
    return 0;
}

static Uint8 *SEXOSAUDIO_GetDeviceBuf(_THIS)
{
    return _this->hidden->mixbuf;
}

static void SEXOSAUDIO_PlayDevice(_THIS)
{
    const int16_t *frames = (const int16_t *)_this->hidden->mixbuf;
    Uint64 count = _this->spec.size / 4;
    Uint64 at = 0, i;
    for (i = 0; i < count; i++) {
        if (frames[i * 2] != 0 || frames[i * 2 + 1] != 0) {
            _this->hidden->nonsilent++;
        }
    }
    while (at < count && !SDL_AtomicGet(&_this->shutdown)) {
        Uint64 took = sex_sdl_audio_write(frames + at * 2, count - at);
        at += took;
        if (took == 0) {
            SDL_Delay(2);
        }
    }
    _this->hidden->frames += at;
}

static void SEXOSAUDIO_WaitDevice(_THIS)
{
    /* Keep about a quarter of a second queued in the ring. Bell takes a card period
     * at a time and a worker's sleep can run long, so less than that plays with gaps.
     * It is also little enough that SDL_GetQueuedAudioSize follows what is heard. */
    const Uint64 ahead = SDL_max((Uint64)_this->spec.samples * 2, (Uint64)_this->spec.freq / 4);
    while (sex_sdl_audio_pending() > ahead && !SDL_AtomicGet(&_this->shutdown)) {
        SDL_Delay(2);
    }
}

static void SEXOSAUDIO_CloseDevice(_THIS)
{
    char line[96];
    int waited = 0;
    /* Let what is queued play out, for up to three seconds. */
    while (sex_sdl_audio_pending() > 0 && waited < 3000) {
        SDL_Delay(5);
        waited += 5;
    }
    sex_sdl_audio_close();
    (void)snprintf(line, sizeof(line), "[sdl.audio] rate=%d frames=%llu nonsilent=%llu",
                   _this->spec.freq, (unsigned long long)_this->hidden->frames,
                   (unsigned long long)_this->hidden->nonsilent);
    sex_sdl_record(line);
    SDL_free(_this->hidden->mixbuf);
    SDL_free(_this->hidden);
}

static SDL_bool SEXOSAUDIO_Init(SDL_AudioDriverImpl *impl)
{
    impl->OpenDevice = SEXOSAUDIO_OpenDevice;
    impl->PlayDevice = SEXOSAUDIO_PlayDevice;
    impl->WaitDevice = SEXOSAUDIO_WaitDevice;
    impl->GetDeviceBuf = SEXOSAUDIO_GetDeviceBuf;
    impl->CloseDevice = SEXOSAUDIO_CloseDevice;
    impl->OnlyHasDefaultOutputDevice = SDL_TRUE;
    impl->SupportsNonPow2Samples = SDL_TRUE;
    return SDL_TRUE;
}

AudioBootStrap DUMMYAUDIO_bootstrap = {
    "sexos", "SexOS program sound ring", SEXOSAUDIO_Init, SDL_FALSE
};
