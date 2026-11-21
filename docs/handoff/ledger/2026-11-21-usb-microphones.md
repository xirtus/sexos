# USB microphones (headsets and microphones on their own)

- `servers/sexusb/src/uac.rs`: `capture()` finds a microphone setting (isochronous
  IN data endpoint, PCM at 48 kHz, 1-2 channels of 16/24/32-bit); for UAC2 the rate
  goes to the clock of the terminal the setting streams from. `to_stereo()` turns a
  packet into 48 kHz stereo S16 frames.
- `crates/sex-pdx/src/usb_audio.rs`: a capture ring next to the output ring, with a
  microphone announcement separate from playback (a USB microphone works next to
  the laptop's speakers) and Bell's "may listen" word. Falling behind loses the
  oldest frames, never the newest.
- `servers/sexusb/src/audio.rs`: one Configure Endpoint for the card's playback,
  feedback and microphone endpoints; the microphone's streaming setting is chosen
  only while Bell says someone may listen (a headset's mic light follows), and each
  packet goes into the capture ring. Logs `[sexusb.mic]`, `.on`, `.off`, `.first`,
  `.gone`.
- `servers/sexbell/src/sound.rs`: a USB microphone wins over the card's own
  (`[bell.mic.source]`); a USB card with no microphone records from the card it took
  over from. The per-program permission to hear the microphone is unchanged.
- Fuzz: `drv_xhci_usb` gives sexusb Bell's region and models isochronous IN data.
- Tests: `tests/usb_audio.rs` (headset, UAC2 terminal clock, sample conversion, the
  capture ring). No ABI change. QEMU's usb-audio has no microphone, so the QEMU gate
  covers playback only.
