# SexOS Argon2 workspace erasure

Upstream: `rust-argon2` 3.0.0, https://github.com/sru-systems/rust-argon2,
release source commit `ed81866f163f0c7026aa6fd8388adf37242eb32a`.
Copied from the crates.io release archive, SHA-256
`8ae76b7506744d254fd0eb2c0ff5c5d108201ccbb083111ac04a44eeda105680`.
Original MIT and Apache-2.0 licenses, copyright headers, API, and test vectors
are retained. Local modifications are under the same dual license.

## Changes and integration

- Add `zeroize = "1"` in normalized and original Cargo manifests.
- Every `Block` implements `Zeroize` and `Drop`. Dropping the workspace's
  `Box<[Block]>` erases every 1KiB block before releasing its allocation. The
  same destructor erases compression clones, final accumulator and address
  generation blocks at their respective lifetimes' ends.
- Wrap the H0 seed and H-prime expansion arrays in `Zeroizing`; erase the 16
  explicit compression scalar locals after each column/row writeback.
- Update upstream's work-memory limitation comment to describe this scope.
- Add destructor-path tests that observe each live block immediately after
  erasure, before allocation release; retain upstream compatibility vectors.

The workspace root must contain the following under `[patch.crates-io]`:

```toml
rust-argon2 = { path = "vendor/rust-argon2-3.0.0" }
```

Exclude this directory from workspace membership. The root lockfile selects
this path package (no registry source/checksum) and adds its zeroize dependency.
Preserve the existing `sha2@0.11.1` release `opt-level = "s"` workaround: this
patch does not change that package or its code generation.

Host verification:

```sh
cargo +stable test --manifest-path vendor/rust-argon2-3.0.0/Cargo.toml --no-default-features
cargo +stable test --manifest-path vendor/rust-argon2-3.0.0/Cargo.toml
cargo +stable test -p chastity-core --target aarch64-apple-darwin
```

## Explicit limits

This is best-effort erasure of the named owned storage, not an assurance that
all password material disappears. `zeroize` uses volatile writes and compiler
barriers. It cannot erase earlier moves/copies, CPU registers, optimizer-created
spills, the internal state/copies of `blake2b_simd`, caller-owned borrowed password,
salt/secret/additional-data inputs, or the returned derived key. Drop does not
run after process abort, OOM abort, force termination, or machine reset. Wiping
scalar staging adds work in Argon2's compression loop; measure guest latency
with the admitted KDF limits rather than weakening these limits to compensate.

The unchanged KeePass 0.17.0 dependency has additional material limits:

- `src/format/kdbx4/parse.rs:113-147` holds decrypted compressed bytes, the
  decompressed inner header/XML payload, and an additional `xml.to_vec()` copy
  in ordinary `Vec<u8>` allocations. Their normal/error-path destruction does
  not zeroize them.
- `src/format/kdbx4/dump.rs:96-119` creates XML, inner payload and compressed
  plaintext staging in ordinary vectors before outer encryption.
- `src/db/types/value.rs` protects `Value::Protected` with `SecretBox`, but
  `Value::Unprotected(T)` has no erasing destructor. Unprotected imported fields,
  metadata and attachments therefore do not gain erasure from the trait bound.
- An outer encrypted-file size cap does not bound decompressed XML or
  attachment memory. KeePass's gzip decompressor uses unbounded `read_to_end`;
  authenticated imported vaults require an explicit decompressed-size ceiling
  before general import can be claimed safe against memory exhaustion.
- XML parsing/serialization and cipher/compression implementation internals can
  create further temporary strings, byte buffers or state. This scoped patch
  does not audit or retrofit them, nor does wiping Chastity's own rendered text
  or dropping its Database retroactively erase released parser/save buffers.

Accordingly describe Chastity-owned secret/render buffer cleanup separately
from dependency memory hygiene; do not claim whole-vault plaintext wiping.
