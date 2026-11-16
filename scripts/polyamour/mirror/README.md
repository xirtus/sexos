# Polyamour

Polyamour is the 3D suite of [SexOS](https://github.com/xirtus/sexos): modelling,
sculpting, painting, UV editing, shader and geometry nodes, rigging and
animation, physics (rigid bodies, cloth, particles, smoke and liquid), Grease
Pencil, a video sequencer, a compositor, and three render engines (a path
tracer, a real-time rasterized engine and a Workbench-style solid engine). It
reads and writes `.blend` files, and scripts use a `bpy`-style Python API.

Everything is written in Rust and licensed GPL-3.0-only.

## Where the code lives

This repository is a mirror. Polyamour is developed inside the SexOS tree and
copied here with its history by `scripts/polyamour/mirror.sh`:

| Here | In SexOS |
| --- | --- |
| `apps/polyamour` | the program: UI, workspaces, editors, operators |
| `crates/sex-3d` | the core: scene, meshes, modifiers, nodes, render engines, physics, file formats |
| `scripts/polyamour` | the `.blend` SDNA table generator and this mirror |
| `scripts/program_polyamour_gate.py` | the QEMU boot test |

Changes are made in [xirtus/sexos](https://github.com/xirtus/sexos) and reach
this repository on the next mirror run, so open pull requests there.

## Building

`apps/polyamour` uses SexOS crates by relative path, so build it from a SexOS
checkout:

```sh
cargo test --locked --release -p sex-3d -p polyamour
cargo run --release -p polyamour -- --screenshot out.png
```

The SexOS build of the program:

```sh
cargo build --locked -Z build-std=std,panic_abort \
  -Z build-std-features=compiler-builtins-mem \
  -p polyamour --target x86_64-sex-std.json --release
```

Inside SexOS, `run polyamour [file]` opens it.
