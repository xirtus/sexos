# Valentine new draft collision guard

- Gap: a new local draft treated every Files stat error as an unused name, then
  saved with a replacing rename. A late name collision could erase another
  `.eml` file.
- Change: inspect names with an uncached Files ID check, accept only `NOT_FOUND`
  as available, and use Files' no-replace `move_if_id` for the first save.
  On `EXISTS`, choose another name and retry; an existing opened draft still
  uses staged replacement. A close request keeps the composer open if saving
  fails.
- Proof: the Valentine guest gate gives another writer the selected draft name
  between Ctrl+N and Ctrl+S, verifies that the saved draft uses a different
  name and that the original mail remains, and captures
  `valentine-draft-collision-saved.ppm`.
- Checks: `cargo check --locked -p valentine --target x86_64-unknown-none`,
  Python gate syntax and `git diff --check` passed locally. Full image build
  and QEMU proof run only in GitHub CI.
