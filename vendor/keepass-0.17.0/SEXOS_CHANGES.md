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

Each expanded stream is capped at 16MiB. In addition, XML conversion shares a
single 16MiB logical attachment budget: all KDBX4 header attachment lengths are
checked before cloning, then each XML binary consumes remaining bytes. Duplicate
IDs do not refund budget even when replacing a previous pool entry. Compressed
XML binaries pass remaining bytes into the gzip decoder before its allocation;
unprotected/protected raw binaries are checked before database insertion.
Zeroizing binary decode/decrypt owners wipe temporary bytes on error and when
replaced during expansion. Success transfers the retained Value to the Database.

This is **not a strict global heap budget**. XML parsing objects, base64 staging,
metadata, entries and histories add allocations. Compressed ciphertext,
decompressed payload and XML copies coexist. Decoder allocation capacity may
exceed the logical retained attachment length, though it never exceeds admitted
remaining bytes at that decoder's start; later copies do not retroactively
shrink earlier capacity. Main KDBX4 parse staging includes one 16MiB-capacity
output plus copied XML and decrypted compressed input, in addition to retained
attachments and Argon2's separately admitted workspace. Parser/object work and
whole-operation allocation limits still require separate accounting.

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
patches. KeePass library tests:61 pass, including the 4 new compression tests;
`db::database_tests::test_xml` cannot run from the published crate archive because
`tests/resources/test_db_with_password.kdbx` is absent. This missing-fixture
limitation is not treated as passing evidence.

Cumulative follow-up tests: two individually admissible compressed attachments
that exceed the aggregate ceiling are rejected (including duplicate IDs); two
attachments exactly at the budget succeed. Mixed protected header/XML bytes
share the same counter and excessive headers reject before XML decompression.
A test-only decoder allocation receipt verifies the second decoder receives
only remaining budget, rather than allocating another full 16MiB. These 3 tests
and the 4 stream-admission tests pass, as do 17 canonical Chastity tests.

### Pinned CI regression entry point

`scripts/chastity_host_gate.sh` calls `scripts/vault_dependency_host_gate.sh`.
The dependency gate uses standalone manifests with checked-in vendor Cargo.lock
files and `--locked`, since workspace package selection cannot resolve excluded
crates' test-only dependencies from the root lock. Its explicit Cargo config
patches standalone KeePass's rust-argon2 dependency to the repository's local
vendored Argon2. No registry Argon2 substitutes for the erasure patch. It runs
Argon2's unit/integration suites and all seven `sexos` compression/attachment
regressions, omitting the known unpublished XML fixture rather than counting it
as passing evidence. It builds host tests only and does not modify guest media.

The published manifest's unused native dev-only `rustfmt0.10` dependency is
removed from both manifests: it pulls extprim1.7 whose nightly-channel cfg
requests removed `llvm_asm`/`const_fn` features. No packaged library test uses
this old formatter. Removing this test-tool edge makes the gate compile on the
project's pinned nightly without changing application dependencies or features.
