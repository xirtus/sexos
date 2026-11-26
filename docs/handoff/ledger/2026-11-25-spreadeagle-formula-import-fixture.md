# Spreadeagle imported formula compatibility fixture

- Branch: `codex/spreadeagle-formula-fixture`, stacked on `codex/spreadeagle-import-warnings` (#664).
- Gap: importing an ODS sheet named `Q1:Q2` shortened it to `Q1Q2`, but a formula referring to the original name failed restoration. The reference translator also split the colon inside the quoted name as a range separator. The ODS importer then left the formula's cached value instead of a recalculating formula.
- Change: parse ODF range separators outside quoted sheet names and map ODS source names to the engine's imported names before restoring formulas. A hand-written ODS fixture checks nested and quoted arguments, cross-sheet formulas, and recalculation; the host test saves and reopens XLSX, imports CSV, and recalculates from imported data.
- Focused host verification: `cargo test --manifest-path apps/spreadeagle/Cargo.toml imported_ods_formula_arguments_recalculate -- --nocapture` and `openformula_becomes_excel`.
- CI guest evidence: `scripts/program_spreadeagle_gate.py` fetches the fixture on SexOS, checks the visible formula results and records `spreadeagle-formula-compat.ppm`. Full build and QEMU gate run on GitHub CI.
- Remaining: broader Excel/Calc compatibility corpus, including arrays and locale-sensitive formulas. GridCraft still normalizes ODS sheet names to XLSX rules; Spreadeagle keeps formulas working with those imported names.
