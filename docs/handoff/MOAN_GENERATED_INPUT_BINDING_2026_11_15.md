# Moan helper build input custody

Files derives the trusted helper digest from `iso_root/programs/moanplug` at
build time. This generated ELF is outside the literal Rust source/asset closure.
The Files build stage must declare
`consumed_inputs = ["iso_root/programs/moanplug"]` and run after the helper build
and loader byte scan.

`scripts/sexos_build_trace.sh` records the declared inputs before Cargo, then
requires identical bytes after compilation and staging. A completed receipt
associates their SHA-256 and lengths with the exact staged Files ELF. Starting a
new ISO build removes previous receipts. A missing input, incomplete receipt or
changed input aborts the trace.

The master gate validates these receipts against the sealed build specification
and current input/output bytes before boot. It records them separately as
`consumed_build_inputs` in provenance. The native evidence freezer validates them
again and archives both helper and consuming Files ELF under `generated-inputs/`.
The declaration itself must be in the unchanged production source manifest.
This preserves the distinction between source inputs and generated artifacts.

Host checks exercise helper substitution during the actual production trace,
missing/changed inputs, incomplete receipts, altered consuming outputs, injected
receipts, external symlinks, archival copying and refusal of a newly declared
input absent from an older build's provenance. These are host custody checks;
they do not qualify a guest plugin policy or attest against a hostile build host.
