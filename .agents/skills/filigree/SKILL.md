---
name: filigree
description: >
  Guide for Filigree, the SexOS font manager, specimen comparator, and Spindle
  typography bridge. Covers headless CLI (`list`, `info`, `install-check`,
  `activate`, `deactivate`, `dupes`, `compare`, `spindle-hand-off` with `--json`),
  `/system/fonts/active.toml` non-destructive manifest, 6-family comparison, and
  `filagree` Spindle alias.
---

# Filigree

Filigree (`apps/filigree`, `/bin/filigree`) is the SexOS font manager, specimen comparator, and Spindle typography bridge built on Weft (`crates/filigree-core`, `crates/sex-font`).

Spindle bare-name alias `filagree` resolves to `/bin/filigree`.

## Headless CLI (`--json` supported on all commands)

| Command | Usage | Purpose |
|---|---|---|
| `list` | `filigree list [--mono] [--starred] [--active\|--inactive] [--class <cls>] [--json]` | List unified font families (`cls`: `serif`, `sans`, `script`, `decorative`, `mono`) |
| `info` | `filigree info <query> [--face <idx>] [--json]` | Inspect family/face metadata, weights, coverage, and paths |
| `install-check` | `filigree install-check [name] [--json]` | Validate `.ttf`, `.otf`, `.ttc`/`.otc`, or `.zip` bundles in memory before disk writes |
| `activate` | `filigree activate <family\|face-id> [--face] [--json]` | Enable a family or specific face and emit updated `/system/fonts/active.toml` |
| `deactivate` | `filigree deactivate <family\|face-id> [--face] [--json]` | Disable a family or face non-destructively via `/system/fonts/active.toml` |
| `dupes` | `filigree dupes [--json]` | Detect `exact_sha256` and `version_shadowed` duplicates with a `tidy_plan` |
| `compare` | `filigree compare <family...> [--word <str>] [--size <px>] [--json]` | Render/compare up to 6 families side-by-side (`MAX_COMPARE_SLOTS = 6`) |
| `spindle-hand-off` | `filigree spindle-hand-off <family\|id\|path> [--json]` | Emit updated `/system/settings.toml` with Spindle typography metrics |

## Non-Destructive Activation Manifest (`/system/fonts/active.toml`)

- `activate` and `deactivate` never delete font files; they update `/system/fonts/active.toml` (`DEFAULT_MANIFEST_PATH`) by family or face reference (`id`, `sha256_hex`, or `path`).
- Pre-install validation (`install-check`) inspects font tables and enforces a 64 MiB per-member decompression cap (`DEFAULT_MAX_ARCHIVE_MEMBER_BYTES`) and unsafe-path checks on `.zip` bundles before installation.

## 6-Family Comparison & Spindle Handoff

- **Specimen Comparison (`compare`)**: Compares up to 6 families simultaneously (`MAX_COMPARE_SLOTS = 6`, default word `"Hamburgefonstiv"`, default size `48px`).
- **Spindle Bridge (`spindle-hand-off`)**: Resolves `family`, `path`, `face_index`, `units_per_em`, and `advance_units`, and merges them into `/system/settings.toml` (`DEFAULT_SETTINGS_PATH`).
