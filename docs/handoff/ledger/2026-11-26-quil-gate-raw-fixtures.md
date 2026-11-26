# Quil gate: copy the raw-string fixtures

The Quil editor gate served `raw-multiline.rs` and `raw-scroll.rs` from its
fixture server but never fetched them into `/data/quil`, so quick-open had no
match for "raw-multiline" and the gate timed out waiting for
`[quil.code.opened] path=raw-multiline.rs`. Both files are now fetched with
the other fixtures. No app change.
