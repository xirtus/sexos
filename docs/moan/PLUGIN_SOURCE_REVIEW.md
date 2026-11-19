# Source and ABI review

SoundCraft was inspected at2688ccac8859c45cc0830fb193c5979952507b16, not inferred
from its product page. Its [CLAP crate](https://github.com/storytold/soundcraft/blob/2688ccac8859c45cc0830fb193c5979952507b16/crates/clap-host/src/lib.rs)
loads plugins in-process and explicitly describes native crash propagation. The
[manifest](https://github.com/storytold/soundcraft/blob/2688ccac8859c45cc0830fb193c5979952507b16/crates/clap-host/Cargo.toml)
uses clap-sys0.5 and libloading0.9. Its [VST3 host](https://github.com/storytold/soundcraft/blob/2688ccac8859c45cc0830fb193c5979952507b16/crates/vst3-host/src/lib.rs)
has the same in-process limitation. Standard folder discovery, OS dynamic loading,
native editors and process-lifetime library caching are not SexOS mechanisms.
Useful lessons are the real lifecycle, bounded plugin-owned counts and separation
between main/audio work. No SoundCraft implementation was copied into this slice.
Its [workspace license](https://github.com/storytold/soundcraft/blob/2688ccac8859c45cc0830fb193c5979952507b16/Cargo.toml)
is MIT OR Apache2; that does not supply a sandbox or prove its advertised parity.

CLAP source/header qualification is pinned toa47f6badb49d948fd009998f28309cdab78979c9
in [free-audio/clap](https://github.com/free-audio/clap/tree/a47f6badb49d948fd009998f28309cdab78979c9).
The MIT license, full headers and unchanged C template are retained in
vendor/clap-fixture. The template's real external entry/factory/lifecycle processes
a stereo swap. The build-time crash/hang/restart variants are explicitly local
negative fixtures, not upstream behavior or production effects.

The [official VST3 SDK](https://github.com/steinbergmedia/vst3sdk) and its current
MIT license were checked directly. The Rust [vst3 bindings](https://github.com/coupler-rs/vst3-rs/tree/2d86c810be3e22367f92438800cd96a3e9cd3a37)
are separately MIT OR Apache2. This slice vendors generated0.3.0 bindings and
com-scrape-types0.1.1 (33f1859fbdcfa247c1fdd17d565cb5285860e4ce), changing only
standard-library paths to core/alloc and enabling std for unchanged host tests.
No vtable/UID/layout was rewritten. Published licenses/notices are retained.

The Gain fixture is the complete upstream vst3-rs example with unchanged DSP,
ported to core/alloc, explicit host allocation callbacks and freestanding module
exports. scripts/build_moan_plugin_fixtures.sh builds its real ET_DYN module.
This qualifies a source-built freestanding ABI boundary; ordinary desktop binaries,
OS runtime loading and a full third-party plugin compatibility suite remain absent.
