# Blender 4.2 SDNA tables for the .blend writer

`crates/sex-3d/src/io/blend/sdna42.rs` is generated, not hand-written.

- `mk.py` builds `sdna42.json` (struct names, member types/names, sizes) from Blender 4.2's GPL `DNA_*.h` headers.
- `gen.py` turns it plus `DNA_*_defaults.h` into `sdna42.rs`: `python3 -I gen.py OUT.rs` (run next to `sdna42.json`).

The Blender headers themselves are not kept here; fetch the 4.2 tag of Blender's source to regenerate.
