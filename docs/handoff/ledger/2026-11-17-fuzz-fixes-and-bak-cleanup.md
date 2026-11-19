## 2026-11-17 Fuzz fixes and backup-file cleanup (WS3)

- sex-office `Pen::text` / `measure` / `mix` / `Canvas::blit`: 24.8 pen positions in i64, saturating clip math, alpha clamped to 255; huge or hostile coordinates no longer panic (#281).
- Spreadeagle `column_name` and cell names widen to u64 before `+ 1` (#282).
- Kaleidoscope extensions: ZIP inflate size hint capped by packed size and 256 KiB, folder entries with trailing blanks skipped, `clean_path` refuses empty segments (#319); `default_locale` must be letters, digits, `_` or `-` (#320); DNR `urlFilter` refuses `#`, `$`, blanks and control characters and every domain list entry must be a host name, so rules can't smuggle cosmetic exceptions or Shields options (#321).
- Removed 92 tracked `*.bak*` files; `scripts/tests/test_kernel_design_guardrails.py` now fails on backups under kernel/, servers/, apps/, crates/ and scripts/.
