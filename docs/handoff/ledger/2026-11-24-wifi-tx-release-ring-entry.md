# Wi-Fi TX release by ring entry, `wifi` shows live link counters

Date: 2026-11-24. Found on the x17 (AX210): the first Wi-Fi tether streamed about
87 KB of the boot log, stalled ("tether: network error"), and every reconnect
after that failed with "could not reach the other computer" while the Mac listener waited.

- `sex-wifi` `TxQueue::release` freed frames until its 16-bit write pointer
  equalled the answer's SSN exactly. When the firmware's SSN does not count the
  same way, that compare never matches, and the loop freed every queued frame,
  zeroing the byte counts of frames still waiting to be sent. That can leave the
  firmware's queue stuck. It now frees up to the SSN's ring entry (SSN mod 256),
  as `iwl_txq_reclaim` and iwx's `iwx_txq_advance` do. `ssn_skew` counts answers
  whose SSN was not our write pointer, to show on hardware whether this happened.
- Typing in Spindle logs about 25 lines per key, which pushed every Wi-Fi line out of
  the log's 64 KiB tail (`hw wifi.link`: 0 lines). sexnet now logs
  `[wifi.link.now]` / `[wifi.link.now2]` (TX queue, answers, SSN, skew, sends
  refused as full, RX drops, overflow, IPv4/ARP counts, missed beacons, block ack)
  when Spindle asks for the Wi-Fi state, and Spindle's plain `wifi` prints them
  under its state line while on a network.
- `[wifi.link.tx.failed]` now counts and logs only the first 8 and powers of two.
- Test: `tx_release_frees_up_to_the_answered_ring_entry`.
