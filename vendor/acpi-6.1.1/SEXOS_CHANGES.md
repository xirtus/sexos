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
- `aml/mod.rs`: `DerefOf` recognises references by variant. Upstream tested
  `typ() == Reference`, but `typ()` reports what a reference points at, so every
  `DerefOf (PKG [i])` failed with "expected Reference, got Integer" (common in real DSDTs:
  table lookups in `_PRT` helpers, `_BCL` users, vendor WMI methods). A dereferenced buffer
  element reads as an Integer.
- `aml/mod.rs`: a buffer field used as a value (`If (FLD0 == 2)`, `FLD0 + 1`) is read: an
  Integer when it fits the integer width, else a Buffer. Upstream passed the field object
  on and comparisons failed. `CreateXxxField (ArgN, ...)` now points the field at the
  buffer itself rather than the argument slot (reading it panicked). Every WMI method that
  takes its input buffer apart with `CreateByteField` needs both, e.g. Alienware's `WMAX`.
- Removed the upstream `Cargo.lock` and `.cargo_vcs_info.json`.
- `aml/mod.rs`: field lists accept `AccessAs` (changes the access type of the fields
  after it), `Connection (...)` (skipped: a resource template or its name) and extended
  `AccessAs`. Upstream hit `todo!()` on all three, which killed the whole AML namespace
  on the Alienware x17 R1 (its DSDT declares GPIO/I2C fields with them): no touchpad,
  battery, lid or brightness.
- `aml/mod.rs`: every remaining `todo!()` (CopyObject, Match, Load, LoadTable, some
  reference and store forms) returns `AmlError::LibUnimplemented` instead of panicking.
  In `sexacpi` a panic sets all AML aside for the boot; an error fails only the method or
  table that used the construct.
- `aml/mod.rs`: `Not`, `FindSetLeftBit` and `FindSetRightBit` take their target and store
  the result. Upstream started them with an operand and a target but matched one argument
  and panicked, which set all AML aside on the Alienware x17 R1 (no touchpad again, after
  the field list fix). `Divide`/`Mod` by zero return `AmlError::DivideByZero`, shifts by 64
  or more give 0, and every remaining argument mismatch (`else { panic!() }`) or unknown
  store target returns `AmlError::LibUnimplemented`: a bad construct fails its method, not
  the interpreter.
