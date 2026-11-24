# 2026-11-24 Wi-Fi association asks for short preamble on 2.4 GHz

- `sex_wifi::ieee80211::assoc_request` sets capability bit 0x0020 (short preamble) on
  2.4 GHz channels. The x17 R1's home AP (Midnight1, channel 10, WPA2/WPA3) refused every
  association with status 19 ("station does not support short preamble"); `wifi` showed
  `the last try failed: assoc Refused(Status { frame: "assoc", status: 19 })`. The 5 GHz
  BSS of the same network may not check the bit (likely why an earlier join worked; unconfirmed).
- Test: crates/sex-wifi/tests/driver.rs authentication_and_association_frames.
