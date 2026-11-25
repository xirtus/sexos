# Quil open-tab definitions

- Gap: Ctrl+Shift+G rejected a definition returned outside the active file.
- Change: send up to eight bounded, same-language open-tab snapshots over the
  Quil Language Service pipe. A declaration jump uses the returned URI only
  after it matches the request's exact tab map and both editor revisions.
  The helper receives no Files grant and refuses ambiguous peer declarations.
- Proof: native LSP helper test covers a unique peer beside an unrelated
  declaration and then an ambiguous peer;
  QEMU gate opens two Rust tabs, requires the target peer's path in the jump
  receipt, and captures
  `quil-lsp-cross-file-definition.ppm`.
- Checks: x86 target checks for the app and helper, Python syntax and diff
  checks. Full build and QEMU proof run on GitHub CI.
