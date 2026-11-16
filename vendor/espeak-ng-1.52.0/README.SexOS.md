# eSpeak NG 1.52.0 in SexOS

Upstream: https://github.com/espeak-ng/espeak-ng, tag `1.52.0`. Licence GPL-3.0-or-later
(`COPYING`); ucd-tools is GPL-3.0-or-later with Unicode data under `COPYING.UCD`.

Used by `crates/sex-speech` (the voice of Whisper, the screen reader) and through it by
`apps/whisper-voice`. Only what the runtime needs is vendored:

- `src/libespeak-ng/`: the runtime sources and headers (no async, MBROLA, sonic,
  pcaudio or speech-player). `config.h` is the one CMake generated with
  `-DUSE_MBROLA=OFF -DUSE_LIBSONIC=OFF -DUSE_LIBPCAUDIO=OFF -DUSE_ASYNC=OFF -DUSE_SPEECHPLAYER=OFF`.
- `src/include/espeak-ng/`, `src/ucd-tools/src/`: public headers and the Unicode tables.

The compiled English data (`phontab`, `phonindex`, `phondata`, `intonations`, `en_dict`,
`lang/gmw/en`) lives in `crates/sex-speech/espeak-ng-data/` and is compiled into the
program; `scripts/build_espeak_data.sh` regenerates it from this tag.

## SexOS changes (marked `SexOS:` in the source)

- `wavegen.c`: `Flutter_ix`, `agc`, `h_switch_sign`, `cycle_count` and `amplitude2` were
  statics inside functions; they are at file scope and reset by `WavegenInit`, so every
  utterance starts from the same state and output is deterministic.

File access is not patched: `crates/sex-speech/csrc/sexos_espeak_shim.h` is force-included
and sends `fopen`/`fread`/`fgets`/`stat` to memory streams over the embedded data; any
other path is refused.
