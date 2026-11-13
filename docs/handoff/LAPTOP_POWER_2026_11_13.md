# Laptop power: battery, lid, brightness, profiles, thermal (workstream H, 2026-11-13)

Slices H1, H2 (ACPI path), H3, H7 and H9 of `DAILY_DRIVER_NEXT_100_SLICES.md`, plus the
firmware plumbing every laptop needs first. Target: Alienware x17 R1.

## What changed

| Part | Where | What it does |
| --- | --- | --- |
| Embedded controller | `servers/sexacpi/src/laptop.rs`, `crates/sex-laptop/src/ec.rs` | `PNP0C09` found before `_INI`, ports from `_CRS` (default 0x62/0x66), `EmbeddedControl` region handler over the kernel's `FW_IO` window, `_REG(3,1)`, and EC events (`SCI_EVT` polled every pass) run `_Qxx`. Without this, Dell/Alienware batteries read absent. |
| AML interpreter | `vendor/acpi-6.1.1` (`SEXOS_CHANGES.md`) | Custom region handlers are actually called (upstream returned `LibUnimplemented`), and `Notify` reaches `Handler::handle_notify` (upstream panicked on every `Notify`, so any `_Qxx` or `_Lxx` method stopped sexacpi). |
| Battery and AC | `crates/sex-laptop/src/battery.rs`, panel in `servers/sexdisplay` | `_BST` + `_BIX`/`_BIF` summed over batteries: percent, minutes to empty or full, health (last-full over design), critical. Panel shows "87%  2:05 left" / "64%  charging" left of the status chips. |
| Lid | sexacpi `_LID`, kernel `OP_EVENT`, Shell `lid_changed` | Lid change wakes the Shell; it closes the launcher and logs. Locking waits for the lock screen (B6). |
| Power button | kernel `sci()`, Shell `power_button_dialog` | The Shell claims the button (`CTL_CLAIM_BUTTON`): a press opens the launcher on Shut Down (Restart next row). A press the Shell never took still shuts down. |
| Brightness | sexacpi `Backlight`, `crates/sex-laptop/src/backlight.rs` | `_BCL` levels, `_BCM`/`_BQC`. Fn keys arrive as video `Notify` 0x86/0x87. Launcher: Brightness Up/Down (`CTL_BRIGHTNESS_STEP`, queued by the kernel, drained with `FW_CONTROL`). |
| Power profiles | kernel `set_profile`, launcher | Power Saver / Balanced Power / Performance Power write the energy-performance preference (0xC0/0x80/0x00) into `IA32_HWP_REQUEST` (or AMD `CPPC_REQUEST`) on every core, each core at its next idle. |
| Thermal and critical battery | sexacpi + kernel `sensors_reported` | A zone at or above `_CRT`, or a critical battery on battery power, starts a clean shut down (Files flushes first). |
| Touchpad model | `crates/sex-hid-touch` | HID-over-I2C protocol, Precision Touchpad report descriptor parser, contact decoding, gestures (pointer, two-finger scroll, pinch, taps, clickpad, three-finger swipes). Host-tested; the I2C bus server and input wiring are next. |

## ABI changes

- `sex_pdx::power`: new ops `CONTROL` (6, `REQUEST`) and `FW_CONTROL` (24, `FIRMWARE`); control
  words `CTL_BRIGHTNESS_STEP`, `CTL_BRIGHTNESS_SET`, `CTL_PROFILE`, `CTL_CLAIM_BUTTON`; message
  `OP_EVENT` (0x50F1, kernel to the desktop).
- `Sensors` grew `battery_minutes`, `battery_health`, `battery_critical`, `brightness`,
  `critical_decikelvin`, `thermal_critical`, `profile`, `button_pressed`, in bits that were zero
  before, so old words decode unchanged.
- Display (pd 1) gets a `Power` grant with `SENSE` only, to draw the battery.
- No new syscall numbers.

## Tests

- `cargo test -p sex-laptop` (19) and `cargo test -p sex-hid-touch` (15), both in CI.
- `scripts/laptop_power_gate.py` (CI, devices job): laptop firmware SSDT
  (`tests/acpi/laptop_gate_ssdt.asl`) in QEMU. Checks inventory, 50% / 120 min / 96% health / on
  battery, Notify-driven brightness up, panel text, lid close/open in the Shell, Power Saver
  reaching the kernel, Brightness Down reaching `_BCM`, the power dialog and Enter shutting
  down, then a second boot where the thermal zone passes `_CRT` and the machine shuts down.
- `scripts/power_acpi_gate.py` now expects the power dialog before S5.

## On the x17 R1 (what to look for in the serial or boot log)

1. `[sexacpi.ec] path=... data=0x62 command=0x66 from_crs=1` and `[sexacpi.ec.reg] ran=1`.
2. `[sexacpi.ready] ... batteries=1 ac=1 lids=1 ... ec=1 backlight=?`.
3. `[sexacpi.sensors] battery=NN ...` with real numbers; unplugging the charger should log
   `[sexacpi.ec.event] query=0x.. method=_Q.. ran=1` and a `[sexacpi.notify] ... kind=ac` line.
4. Fn brightness keys: `[sexacpi.notify] ... value=0x86 kind=backlight`. If the panel does not
   change while `bqc` does, the firmware's `_BCM` needs the Intel graphics OpRegion (ASLE)
   answered: that is the PWM register path (H2 part two), owned with the native Intel graphics
   work (workstream L).
5. `[acpi.profile] ... msr=0x774 applied=1` after picking Power Saver.

## Not done here

- H4–H6 bus side: DesignWare I2C server (`servers/sexi2c`) with the LPSS lease, the touchpad's
  `_CRS`/`_DSM` from sexacpi, and pointer events into the Shell.
- H8 suspend to idle.
- GPE-driven EC events: events are polled from the EC status register every pass (64 ms);
  the GPE route can replace the poll once the kernel arms GPEs.
