# SexOS changes to acpi 6.1.1 (MIT OR Apache-2.0)

- `aml/mod.rs`: operation regions in spaces the crate does not handle natively
  (EmbeddedControl, SMBus, GenericSerialBus, ...) now go to the handler installed with
  `install_region_handler`. Upstream returned `LibUnimplemented` even when a handler was
  installed. The handler sees an `OpRegion` whose `base` is the accessed address (region
  base plus field offset) and whose `length` is the access width.
- `aml/mod.rs`, `lib.rs`: `Notify(object, value)` runs `Handler::handle_notify`. Upstream
  had no arm for it and panicked ("Unexpected operation has created in-flight op"), so any
  firmware event method (`_Qxx`, `_Lxx`) that notified a device stopped the interpreter.
- `aml/mod.rs`: `LEqual`/`LLess`/... on strings and buffers compare the whole byte
  sequences, as the ACPI specification says. Upstream cast the first four bytes to an
  integer and panicked (slice length mismatch) on any longer buffer, which every I2C HID
  touchpad's `_DSM` hits with its 16-byte UUID check.
- Removed the upstream `Cargo.lock` and `.cargo_vcs_info.json`.
