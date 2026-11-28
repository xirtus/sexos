# 2026-11-28: x17 trackpad controller address, plus hotkey and microphone diagnostics

From the x17's version 6 tether log (`sexwork/tether/logs/session-20261127-205718.log`):

1. **Trackpad controller has no address.** `00:15.0` (Tiger Lake-H Serial IO I2C0,
   `8086:43e8`, where `\_SB_.PC00.I2C0.TPAD` sits) reads `bar_lo=0x00000004 bar_hi=0`
   after it is woken to D0: firmware never assigned its 64-bit BAR (Linux and Windows
   assign one themselves). So the kernel skipped the lease, Input had no controller, and
   neither the ACPI route nor Input's fallback scan could reach the pad.
   Fix: `PciDevice::assign_boot_bar64` (BOOT only) gives an empty 64-bit memory BAR an
   address 1 MiB above the highest 64-bit window firmware placed on that bus (function
   BARs and bridge prefetch windows), naturally aligned, below 2^39. `class_lease` calls
   it for Serial IO before sizing. Logs `[pci.bar.assign]` / `[pci.bar.assign.skip]`.
   Nothing behind a bridge is probed. +~70 kernel lines (budget 24,600 holds).
2. **Trackpad `_CRS` fails.** `[sexacpi.touchpad.skip] path=\_SB_.PC00.I2C0.TPAD reason=no_crs`:
   the AML interpreter errors in the method (Dell builds it with `ConcatenateResTemplate`
   and helper methods). Now logs `[sexacpi.touchpad.crs.err]` with the error and dumps the
   AML of `_CRS` and the methods it names (`[sexacpi.aml.method]`/`[sexacpi.aml.hex]`,
   6 KiB cap), and still runs `_PS0`/power resources on the pad and controller so Input's
   5 s fallback scan (addresses 0x2c, 0x15, 0x20, 0x38, 0x10) can find a powered pad. That
   gives a polled trackpad; the interrupt line needs `_CRS` fixed from the dump.
3. **Hotkeys.** The internal keyboard is USB (`0d62:babc`, Darfon); only its boot-keyboard
   interface 0 binds, so its volume/mute/mic-mute keys (another interface) never arrive.
   PR #848 owns the key mapping (mic mute, composite pointer+media). Here only: a HID
   interface nothing understood logs `[sexusb.hid.unbound]` and its report descriptor
   (`[sexusb.hid.desc]`, 512 B cap); interfaces dropped by the 3-endpoint cap log
   `[sexusb.hid.skip] reason=max_intr`.
4. **Microphone.** `[bell.mic.card] input=-1`: no codec pin qualified. On the x17 R1 the
   built-in microphones are DMICs on the Tiger Lake DSP (Linux uses SOF with
   `sof-tgl-h.ri` and `sof-hda-generic-2ch.tplg`), not on the ALC289 codec, so Bell's HDA
   path cannot see them; only the headset jack is on the codec. Bell now logs every
   input-capable pin (`[bell.hda.pin.in]` config/caps) to confirm. A DSP (SOF) driver is
   a separate project.

No syscall ABI change; no ABI_INPUTS touched.
