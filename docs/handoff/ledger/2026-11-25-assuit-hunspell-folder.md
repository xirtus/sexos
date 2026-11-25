# Assuit app-owned Hunspell dictionaries

- Gap: Assuit exposed only embedded English proofing despite WordCraft's
  multilingual Hunspell engine. Other language tags were refused even when a
  person supplied a dictionary.
- Change: Assuit creates its own dictionaries folder, points WordCraft there,
  and accepts a language-region tag only when a matching `.aff`/`.dic` pair
  loads. A missing or unreadable pair leaves the document unchanged. Assuit
  does not redistribute third-party dictionaries; the person supplies a pair
  with a licence appropriate to their use through Loom.
- Proof: a synthetic German pair checks a correct and misspelled word, then
  DOCX and ODT save/reopen retain the language. The QEMU Assuit gate writes a
  pair into the app folder through Spindle, selects `de-DE`, checks the spelling
  issue, and captures `assuit-german-dictionary.ppm`.
- Local checks: focused Assuit host test, gate Python syntax, and diff check
  passed. Full build and QEMU guest proof are delegated to GitHub CI.
- Guest path correction: Hermit's `std::fs::canonicalize` is unsupported; use
  SexOS's grant-aware `sex_std::fs::canonicalize` when configuring the folder.
  The focused host dictionary test passes. A local target check stopped in
  `ring` because this machine lacks the target C `assert.h`; GitHub CI's
  Assuit build and QEMU gate will exercise the Hermit branch.
