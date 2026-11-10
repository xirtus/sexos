# Feature ledger fragments

New feature entries go here as one file per change, named
`YYYY-MM-DD-short-topic.md`, in the same shape as the entries in
`../SEXOS_FEATURE_LEDGER_2026_11_02.md`. Separate files never conflict when
several branches merge on the same day. The main ledger also has
`merge=union` in `.gitattributes`, so local merges and rebases keep both
sides' appended lines.
