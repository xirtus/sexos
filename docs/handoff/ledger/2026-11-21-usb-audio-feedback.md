# USB sound cards follow their own clock (async feedback)

- `servers/sexusb/src/uac.rs`: playback descriptors now find an asynchronous
  card's feedback endpoint (iso IN with feedback usage, or bSynchAddress for
  UAC1). `feedback_q16` decodes 10.14 (full speed) and 16.16 (high speed)
  reports and rejects anything more than 1/8 off 48 kHz; `Pacer` turns the
  rate into whole frames per packet with a 16.16 carry.
- `servers/sexusb/src/audio.rs`: configures the feedback endpoint (type 5,
  CErr 0) next to the data endpoint, keeps 4 report requests in flight, and
  sizes each packet from the card's reported rate (within one frame of
  nominal). Logs `[sexusb.audio.feedback]` and `[sexusb.audio.rate]`.
  Adaptive/synchronous cards (QEMU's usb-audio) are unchanged.
- Tests: `tests/usb_audio.rs` (UAC2 and UAC1 feedback discovery, decode and
  pacing). No ABI change.
