# sex-wasm

WebAssembly guests for SexOS on [wasmi](https://github.com/wasmi-labs/wasmi): programs
that run inside a SexOS program, isolated by the interpreter instead of a hardware key.
See the crate docs in `src/lib.rs` for the `sex` and WASI import sets, and
`docs/handoff/SEXOS_FEATURE_LEDGER_2026_11_02.md` for status.

```rust
let guest = sex_wasm::Guest { name: b"greet", args: &[b"Andreas"],
    grants: sex_wasm::Grants { folder: Some(b"guests/greet".to_vec()), net: false },
    limits: Default::default() };
let (outcome, host) = sex_wasm::run(wasm_bytes, &guest, host);
```

Tests: `cargo test -p sex-wasm`.
