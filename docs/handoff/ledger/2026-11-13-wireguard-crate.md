# WireGuard protocol crate (workstream K6, part 1)

- New `crates/sex-wg` (GPL-3.0, no_std + alloc, no I/O), written from the WireGuard paper: Noise_IKpsk2 initiation/response, transport data with 16-byte padding, 2048-counter replay window, cookie replies (XChaCha20-Poly1305) and mac2, TAI64N replay protection, and the timers (REKEY_TIMEOUT retry, REKEY_AFTER_TIME, REJECT_AFTER_TIME, keepalive, persistent keepalive, give up after REKEY_ATTEMPT_TIME). Own BLAKE2s/HMAC/HKDF; X25519 on fiat-crypto (copied from sex-tls); ChaCha20-Poly1305 from RustCrypto.
- Tests: BLAKE2s vectors, both roles against each other, replay/tamper/stranger refusal, cookie -> mac2, timers, and interop with an independent Python peer (hashlib + cryptography) over real UDP.
- Next (K6 part 2, needs sexnet): a tunnel link in sexnet that routes chosen prefixes through `Tunnel`, the QEMU gate with a host WireGuard peer, and a config page.
