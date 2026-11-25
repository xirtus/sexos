# Assuit proofing languages

- Gap: Assuit could assign only US or British English even though WordCraft
  includes Australian and Canadian English proofing policies. Authors also
  could not mark DOCX text as intentionally unproofed.
- Change: Ctrl+Shift+K accepts `en-AU`, `en-CA`, and `none` (`zxx`) alongside
  the existing English choices. ODT refuses `none` before mutation because
  WordCraft loses that tag on ODT save.
- Proof: focused host test checks language routing, DOCX/ODT save and reopen,
  and refusal without mutation. The Assuit guest gate chooses `en-AU` and
  captures `assuit-proofing-en-au.ppm`.
- Checks: `cargo test --locked -p assuit english_dialects_and_no_proofing_round_trip`
  passed locally. Full build and QEMU guest proof run in GitHub CI.
