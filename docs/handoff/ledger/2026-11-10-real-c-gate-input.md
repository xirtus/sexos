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
