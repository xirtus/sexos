## Release update trust key (2026-11-10)

- Generated the project Ed25519 key pair using `tools/sexos-release keygen` after
  reading `SIGNED_UPDATES_2026_11_10.md`. This change commits **only** `keys/release.pub`.
  Subsequent builds compile that public trust anchor into Files for signed A/B updates.
- Private material is outside every repository at `/Users/xmacbookm5/sexos-release-key/`.
  The directory is mode 0700 and `release.key` is mode 0600. No secret was printed,
  committed, uploaded, or placed in CI. The owner still needs an offline backup.
- Local checks: public derivation matches the committed public half; a temporary
  512-byte fixture image signed with the generated key verifies; changing the image
  makes verification fail. This was a local signing round trip, not an OS release.
- No release was published and no CI signing secret was configured. Builds from before
  this trust anchor still require reinstalling once, as documented by signed updates.
- No kernel, ABI, driver or runtime behavior change beyond build-time public trust data.
