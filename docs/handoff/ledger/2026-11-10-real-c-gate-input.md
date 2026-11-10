# Refuse the known C fallback before guest testing

An OVMF run of the coherent #143 source `01c3de2e` reached the desktop and passed
Rust std 14/14 twice, then printed the documented no-C-compiler fallback because
the macOS build lacked musl headers. The gate correctly could not accept that
fallback as the C suite, but waited for a score it could never produce. The run
was interrupted inside its cleanup scope; the guest stopped and was reaped.
No std::net or UEFI network acceptance was claimed from this partial run.

The std gate now rejects that specific compiled fallback before disk copy,
program installation or QEMU launch and explains the missing build input. Its
actual C/socket/runtime assertions are unchanged. This is an early diagnostic,
not an alternative to running the C checks or an attestation against arbitrary
untrusted binaries. Standalone builds may still deliberately use the fallback.

The actual retained stub from that run is rejected locally without starting a
guest; Python syntax and diff checks pass. The repaired C build and complete
OVMF std/socket run are being qualified separately with compiler/header hashes.
Partial evidence remains in `Projects/sexos-evidence/uefi-network-143-01c/`.

## Qualified real-C UEFI rerun

The repaired sealed image and immutable programs from source `01c3de2e` pass the
complete unchanged runtime suite under OVMF/QEMU TCG, PKU enabled, 1280x800:
Rust std 14/14 twice; C stdio/files 21/21 twice; both PDF pages and relaunch;
Rust std::net 15/15 twice (TCP, UDP, two-thread TCP, forwarded listener and the
existing real-clock bound); C sockets 23/23. The early-input guard from `5197d807`
accepts this real C binary. Gate cleanup stops and reaps the guest.

ISO SHA-256: `b87eb4b7ddad10d51730b13f3ef0f16da95d4d91d897259965ecf5e9c7a469ee`;
Chello SHA-256: `6b3f0aeb27efc9cc188fc96140b58a19a88b9a8ddd1fff1760dffe3fb2e1e0cf`.
The image uses verified musl 1.2.5 headers and Apple clang 21 for Chello. Its
unchanged Web/QJS artifacts use Homebrew clang 23.1.5; a retained correction
sidecar fixes the earlier descriptive LLVM22 label without changing any binaries.
All program, firmware, runner, helper and build-input hashes, raw serial,
screenshots and JSON results are under
`Projects/sexos-evidence/uefi-network-143-01c-musl/`. This establishes a real UEFI
networking guest path, not physical x17 network or radio behavior.
