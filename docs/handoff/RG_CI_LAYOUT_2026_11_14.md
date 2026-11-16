# ripgrep executable layout repair

Integration `fb635078`, CI run `43595715822`, failed before guest gates because
the real program loader's forbidden-byte scanner rejected `rg`. All 24 layout
retries reported the same address (`0x34273d`): `apps/rg/build.rs` did not read
the existing trace's `SEX_LAYOUT_SEED`, so those retries did not relink anything.

An independent guest build reproduced the hit at `0x342a9d`. Disassembly identifies
`core::ptr::drop_glue<Box<regex_syntax::ast::ClassSet>>` calling
`__rustc::__rust_dealloc` through bytes `e8 0f 01 ef ff`. The call displacement
contains `0f 01 ef`, which is rejected at every offset even though it is not an
instruction boundary. Relative function layout can remove this accidental hit.

The rg build now honors the trace's existing section-shuffle retry exactly as
Play/Web do, for `.text*` and `.rodata*`. Seed 1 produced a guest ELF with no hits;
the 8 MiB GNU stack request is preserved. No loader, scanner, allowed-byte list,
upstream ripgrep source, or kernel changes are involved. Linker flags and the
sex-std dependency are limited to the Hermit target, so the same vendored ripgrep
source also builds natively for a functional fixture.

Validation on 2026-11-14:

- Guest release build with `SEX_LAYOUT_SEED=1`, the normal PIC/PIE flags and
  `build-std=std,panic_abort` passed. `scripts/key_bytes.py` returned zero hits.
- `llvm-readelf -lW` confirms GNU_STACK MemSiz `0x800000`.
- Native release build passed. Eight fixture checks passed: numbered search,
  case-insensitive file matches, counts, Markdown glob filtering, Unicode ranges,
  nested character-class intersection, no-match status 1, invalid-regex status 2.
- `rustfmt +stable --check apps/rg/build.rs` passed.

CI still must build its exact integration candidate and run `rg_fd_gate.py` in
QEMU. Native functional checks and a clean guest ELF do not replace that gate.
