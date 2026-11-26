# x17 internal I2C-HID trackpad bring-up (Gemini, PR #763)

Lands Gemini's `hw/x17-internal-trackpad` on current master.

- Kernel `devmgr`: Serial IO and Wi-Fi parts are woken to D0 (with a 10 ms settle)
  before their BAR is sized, so a part firmware left in D3hot no longer reads as an
  invalid BAR. The skip line now prints the BDF and raw BAR words. +4 kernel lines.
- `sexacpi`: `_HID`/`_CID` matching ignores case (the x17's touchpad ids are not
  upper case).
- `sexinput` touchpad: probe retries, a read-only alternate address/register check on
  the same controller, a one-time fallback scan of leased Serial IO controllers 5 s
  after boot when ACPI found no pad, and a plain-mouse fallback when the pad has no
  precision-touchpad collection.
- `sex-hid-touch`, `sex-i2c-dw`, `sex-hwdb`, `sex-laptop`: descriptor and session
  handling for the mouse fallback, plus x17 sweep tests.

Self-wait: `scripts/check_self_wait.py` gains an `ACPI_AUX_COHORTS` entry for the new
`servers/sexacpi/src/main.rs` (its fixed 64-tick wait is unchanged). Without it the
keyboard proof and Host regressions refused ACPI's task 50 timed polls.

No syscall ABI change. No Wi-Fi or tether code touched.
