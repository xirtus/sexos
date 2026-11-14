# SexOS bounded expansion and KDBX4 staging

Upstream: `keepass` 0.17.0, https://github.com/sseemayer/keepass-rs.
Copied from the crates.io release archive, SHA-256
`8025344cf174f72cccbac2a99640a8d1e82cf46922d57e06a45a32e49edfed16`.
Its packaged VCS metadata reports commit
`26cb076aa5cbbe36e175326dfdaf5a19799c4085`, **dirty: true**; the archive
checksum, rather than a claim of pristine git equivalence, identifies this source.
Original MIT LICENSE and copyright notices are retained; local changes are MIT.

## Scoped changes

- `compression.rs`: 16MiB maximum expanded bytes per gzip stream, including
  the shared compressed-attachment path, and maximum uncompressed copy input.
  Read at most 8192 bytes per iteration and probe exactly one byte beyond the
  limit. Rejection happens before output growth. One 16MiB zeroizing allocation
  prevents reallocations from releasing unwiped output prefixes; only the used
  prefix is returned, retaining capacity until its caller releases it.
- Decompression output is wiped on read/CRC/length failure. Successful output
  transfers ownership to the existing API caller. Compression's output owner is
  also zeroizing on error, with 16MiB input admission before compression.
- `format/kdbx4/parse.rs`: wrap decrypted compressed bytes, decompressed inner
  payload, and the XML passed to the parser in `Zeroizing` owners, covering
  ordinary success and Rust error-return paths.
- `format/kdbx4/dump.rs`: wrap XML, inner payload and compressed plaintext
  staging. Borrow XML during payload assembly so its owner wipes after copying.
- Unit tests cover exact 16MiB acceptance, limit+1 gzip bomb rejection, raw
  no-compression boundary, malformed/truncated gzip and an endless reader proving
  the decoder consumes at most limit+1 bytes.

Integration requires root workspace exclusion and:

```toml
[patch.crates-io]
keepass = { path = "vendor/keepass-0.17.0" }
rust-argon2 = { path = "vendor/rust-argon2-3.0.0" }
```

Preserve `sha2@0.11.1` opt-level `s` and the separate Argon2 erasure patch.
Tests: standalone `cargo +stable test --manifest-path
vendor/keepass-0.17.0/Cargo.toml --lib --features save_kdbx4`; root
`cargo +stable test -p chastity-core --target aarch64-apple-darwin` exercises
create/save/open through both path-patched dependencies.

## Accounting and limits

This is a **per-stream expansion ceiling**, not a whole-operation or retained
Database allocation budget. Main KDBX inner payload expansion is at most 16MiB;
XML compressed attachments each have the same ceiling, but multiple attachments
can cumulatively exceed it. XML parsing objects, decoded/base64 data, metadata,
entries and histories add allocations. Compressed ciphertext, decompressed
payload and XML copies coexist. In the KDBX4 parse path this includes one 16MiB
capacity output plus its copied XML and decrypted compressed input; Argon2's
separately admitted workspace also contributes to peak memory. Admitting
arbitrary imported databases still requires cumulative model/attachment and
parser-work limits, in addition to ciphertext-size and KDF cost checks.

This patch does **not** claim total plaintext erasure. `Value::Unprotected`
still lacks an erasing destructor; metadata/attachments can remain unprotected.
The XML library, cipher/KDF/compression internals, keys/inner-header objects,
KDBX3 parsing, saved-buffer reallocation history, CPU registers and compiler
copies are not retroactively erased. The existing `Database::get_xml` result
is a returned ordinary Vec whose caller owns cleanup; `parse_kdbx4` wraps its
own XML owner, without changing that API. Success ownership transfers themselves
may leave compiler-generated copies. Abort, OOM abort or forced termination do
not run destructors. Chastity-owned cleanup and these selected staging owners
must not be described as whole-vault memory wiping.

Validation result: canonical chastity-core17 tests pass through both path
patches. KeePass library tests:61 pass, including the4 new compression tests;
`db::database_tests::test_xml` cannot run from the published crate archive because
`tests/resources/test_db_with_password.kdbx` is absent. This missing-fixture
limitation is not treated as passing evidence.
