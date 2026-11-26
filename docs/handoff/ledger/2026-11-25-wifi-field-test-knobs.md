# Wi-Fi field-test knobs: `wifi ba off`, `wifi replay off`

Date: 2026-11-25. Each x17 Wi-Fi theory cost a CI image and a stick swap. Spindle can
now switch parts of the receive path for this boot, so one image tests several:

- `wifi ba off|on`: hand frames up as they arrive, without block-ack reordering
  (the firmware still acknowledges; TCP copes with order). Tests whether the
  reorder step loses the peer's frames.
- `wifi replay off|on`: skip the CCMP replay check on received frames. Tests whether
  that check refuses good frames. For testing only; it is on at every boot.
- New `sex_pdx::wifi_status::TUNE` (8), knobs `TUNE_BA` (1), `TUNE_REPLAY` (2), Spindle
  only. sexnet logs `[wifi.tune]`; `wifi` shows `[wifi.link.now5]` while a test is on.
- ABI pins re-run (scripts/update_abi_hashes.sh) for the sex-pdx change.
