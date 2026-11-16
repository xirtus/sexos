# Blender 4.2 SDNA tables for the .blend writer

`crates/sex-3d/src/io/blend/sdna42.rs` is generated, not hand-written.

- `mk.py` builds `sdna42.json` (struct names, member types/names, sizes) from Blender 4.2's GPL `DNA_*.h` headers.
- `gen.py` turns it plus `DNA_*_defaults.h` into `sdna42.rs`: `python3 -I gen.py OUT.rs` (run next to `sdna42.json`, with `rename.h` (`dna_rename_defs.h`), `dna/` and `defs/` holding the headers).
- `ROOTS` lists the structs the writer fills; the structs they embed by value come along. Every needed struct that has a `_DNA_DEFAULT_<name>` macro gets its defaults, so adding a root is usually all a new data type needs.

The Blender headers themselves are not kept here; fetch the 4.2 tag of Blender's source to regenerate.
