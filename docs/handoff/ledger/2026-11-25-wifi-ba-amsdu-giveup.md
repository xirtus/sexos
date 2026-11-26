# 2026-11-25 Wi-Fi block-ack: A-MSDU subframes, stale frames, give-up

x17 tether stalls after 35-87 KB of unicast with an RX block-ack session up.
The audit of `crates/sex-wifi/src/ba.rs` and `link.rs` found several ways the
reorder window could lose or stall unicast frames. Changes:

- A-MSDU subframes share one sequence number: the window head moves only on
  the last subframe (`Received.last_subframe`, descriptor amsdu_info bit 0x80),
  and later subframes of a frame already passed or held go straight up.
- Held slots remember `(seq, subframe)` so the CCMP replay check sees the
  right subframe index on release (was always 0, dropping subframes 1..n).
- A frame below the head that the firmware did not mark old is dropped
  instead of aliasing into a slot (`sn % window`).
- `consecutive_drops` counts old/duplicate drops in a row. After
  `BA_GIVE_UP_DROPS` (16) the link sets `ba_bypass` and counts `ba_gave_up`:
  a firmware window out of step with the AP otherwise loses every frame.
- The stuck-window timer only treats an advanced head as movement, so steady
  traffic can no longer hold off the 250 ms flush.
- The DELBA-time flush honours the `replay_check` switch.

Logs: `[wifi.link.now4]` gains `in_a_row`, `[wifi.link.now5]` gains `ba_gave_up`.
