# Open plugins rebuilt for SexOS (Moan test cases)

These are unmodified upstream LADSPA and LV2 plugin sources (in `vendor/`)
compiled for SexOS by `scripts/build_moan_open_plugins.sh`, which Moan's
plugin host tests and the `moanrack` qualification window load.

| Plugin | Format | Source | Licence |
|---|---|---|---|
| T5 Second Order Low Pass / High Pass | LADSPA | `vendor/ladspa-t5` (Juergen Herrmann, t-5 ladspa plugins) | 3-clause BSD ("do with it whatever you want") |
| Simple Amplifier (`eg-amp`) | LV2 | `vendor/lv2-plugins/eg-amp.lv2` (LV2 book examples) | ISC |
| 4 Pole Resonant Low-Pass (`lp4pole`) | LV2 | `vendor/lv2-plugins/blop-lp4pole.lv2` (blop-lv2) | GPL-3.0-or-later |

Headers: `vendor/ladspa/ladspa.h` (LGPL-2.1-or-later), `vendor/lv2-headers`
(ISC).

## What "rebuilt for SexOS" means

SexOS has no libc for plugins and no dynamic linker. A plugin is a
freestanding x86_64 shared object whose undefined symbols resolve only
against Moan's helper import table (`crates/moan-plugin/src/imports.rs` and
the allocator in `mapped.rs`): `malloc`/`calloc`/`realloc`/`free`/`strdup`,
the C math library, and memory/string functions. Nothing a plugin imports can
reach files, clocks, threads or the network.

* `include/` declares exactly that surface (plus the clang freestanding
  headers). POSIX headers the sources include out of habit are empty here.
* `ladspa-t5/helpers.h` replaces the T5 helper that opened a `/dev/shm`
  file so another process could change parameters at run time. On SexOS
  parameters come from the host; the replacement keeps the gain helper and
  descriptor cleanup and makes the shared-memory hooks inert.
* Libraries are linked with `-ffunction-sections --gc-sections` and a version
  script that exports only `ladspa_descriptor` / `lv2_descriptor`, so code a
  plugin never calls (and its imports) is dropped.
* Constructors (`__attribute__((constructor))`, used by the T5 plugins to build
  their descriptors) run from `DT_INIT_ARRAY`, which the helper's loader now
  admits.

The same sources are also built natively on the host so the tests can check
the DSP output against the SexOS host code paths.
