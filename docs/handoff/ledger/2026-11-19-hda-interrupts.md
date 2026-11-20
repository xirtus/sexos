# Sound on Intel HD Audio by interrupt (2026-11-19, native drivers)

- **What changed:** Bell no longer polls the Intel HD Audio controller every 1 ms
  tick while sound plays. The `sex-hwdb` BELL manifest now asks for one vector, so
  BOOT programs the controller's MSI (or MSI-X) at boot, masked. Bell unmasks it
  through syscall 86, flags every period of the playback ring and the microphone
  ring with interrupt-on-completion (BDL IOC + `SD_CTL.IOCE`), and enables the
  streams in `INTCTL` only while sound plays. Each wake clears the stream status
  bits so the next period end interrupts again (MSI is edge-signalled).
- **Wake rate:** while sound plays, Bell sleeps until the next period ends
  (8 KiB = ~43 ms) or a message comes, instead of 1,000 wakes a second. While an
  open program stream could still hand Bell sound the card has room for, Bell
  looks at the program rings every 4 ms (programs write their rings without a
  message). Idle Bell is never woken: the silent stream's interrupt is disarmed
  before Bell blocks for a message.
- **Fallback:** no vector (behind Intel VMD, no MSI capability, unmask refused)
  keeps the old 1-tick polling. Virtio sound gets a vector too but keeps it masked
  and polls its queues as before.
- **Boot log:** `[bell.hda.irq] mode=msi|msix|none vectors=1 bit=N polling=0|1`;
  after each drain `[bell.sound.wakes] irq=1 periods=P waits=W interrupts=I`.
- **Gates:** `scripts/sound_gate.py --card hda` is now a CI media-job step and
  asserts interrupt mode and at most `3*periods+8` waits per drained note (polling
  needs ~43 per period). `sound_listen_gate.py --card hda` asserts interrupt mode.
- **Not yet:** codec unsolicited responses (CIE + RIRB interrupt) for jack
  detection, which is still looked at twice a second while sound plays; a
  program-ring doorbell so live streams need no 4 ms look.
