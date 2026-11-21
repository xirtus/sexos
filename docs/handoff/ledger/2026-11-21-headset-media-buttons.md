# Headset buttons and media keys on USB

- `servers/sexusb/src/hid.rs`: `parse_media()` finds Consumer (0x0C) and Telephony
  (0x0B) buttons in a report descriptor, as one-bit buttons or array slots, with or
  without report IDs. Volume Up/Down, Mute, Play/Pause (and Play, Pause, the hook
  switch), Stop, Previous and Next become the keyboard usages sexinput already maps
  (0x80, 0x81, 0x7F, 0xE8-0xEB), the same ones Cufflinks sends for a Bluetooth
  headset's buttons. Phone Mute is not a speaker mute and presses nothing.
- `servers/sexusb/src/usb.rs`: a HID interface with media buttons gets `Role::Media`
  and its held keys merge into the keyboard snapshot. A headset's HID interface is now
  driven next to its sound (a second Configure Endpoint that keeps the sound
  endpoints inside the slot's context entries, and no second SET_CONFIGURATION,
  which would stop the sound). Keyboards' separate media-key interfaces and USB
  remotes work the same way. Logs `[sexusb.hid.media]` and `[sexusb.hid.media.keys]`.
- Tests: `servers/sexusb/src/hid.rs` (headset with two report IDs, a 16-bit consumer
  array, hostile descriptors), run by `scripts/usb_wheel_host_gate.sh`. No ABI change.
