# 2026-11-25 Wi-Fi: take MIC-OK frames, decrypt the rest in software

Second x17 `tether sweep` (with the power-save fix): downloads of 512 KiB from the
Mac pass in ~2 s, but every upload stops at exactly 32 KiB. The Mac got all of
it, so its TCP ACKs (small, sent by macOS at a high priority, TID 6/7) never
reached sex-net, while its data (TID 0) did. About 40% of received frames were
dropped, most not as Malformed.

- `link::decap` took a protected frame only with MIC OK *and* bit 11 (decryption
  done). iwlwifi (`iwl_mvm_rx_crypto`) and iwx (`iwx_rx_hwdecrypt`) require only
  CCM + MIC OK. Now the same.
- A protected unicast frame the firmware did not decrypt is decrypted in software
  with the pairwise key (`Supplicant::pairwise_tk`, `ccmp::decrypt_frame`), as
  mac80211 does; a padded frame is tried with the pad after the header and after
  the IV (`scan::Received::raw`).
- `Link::rx_stats`: frames taken per TID, not-decrypted per TID, replays per TID,
  software decrypts ok/failed, and the last such drop (kind, TID, status, bytes):
  `[wifi.link.now6]` and `[wifi.link.now7]`, shown by plain `wifi`.
- Test: `x17_takes_frames_with_mic_ok_and_decrypts_what_the_firmware_left`.
