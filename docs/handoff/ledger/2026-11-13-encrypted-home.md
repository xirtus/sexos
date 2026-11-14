# Encrypted home (workstream E9)

- `/home` can be its own encrypted volume: a sexfs inside `/.home.sexcrypt` on the system disk, every block XTS-AES-256 (IEEE 1619) under a random 512-bit key, wrapped in a 4 KiB header by an Argon2id (RFC 9106) key from the password (16 MiB, 4 passes). The system disk stays unencrypted.
- New crate `crates/sex-crypt` (tests against OpenSSL XTS and an independent Argon2id vector). New protocol file `sex-pdx/src/home_lock.rs`: kinds SETUP 24, UNLOCK 25, LOCK 26, STATE 27; statuses LOCKED 17, WRONG_PASSWORD 18 (ABI hash updated).
- Files (portal run, holding `@/`): Ctrl+K sets a password (typed twice), locks, or unlocks. A locked home shows as an empty folder; anything inside answers LOCKED. Files asks for the password at start when home is locked.
- sexfiles runs home work on its own 256 KiB stack: its main stack was too shallow for the key derivation (the first try faulted in blake2b_long).
- Shared-file touches: `sexos_build_spec.toml` build_sexfiles gets `--cfg aes_force_soft` (servers are soft-float); CI step added.
- Gate `scripts/files_home_gate.py`: no byte of the home file on the disk image, SEXCRYPT header present, `sexfs-fsck --deep` clean; second boot refuses a wrong password, unlocks with the right one. QEMU TCG: setup KDF ~0.7 s, unlock ~0.4 s.
- Password change: Ctrl+Shift+K in Files (current, new twice); kind CHANGE 28 rewraps the header with a new salt, no data block touched. Gate checks the old password is then refused.
- Next: the Settings lock/login screen unlocks home through the same UNLOCK request.
