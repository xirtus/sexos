# Wi-Fi: unsplit A-MSDUs, last malformed frame in `wifi`

Date: 2026-11-25 (x17, AX210, Midnight1). With PR #526 the tether still dropped with
"network error" after its first commands. Spindle's new `wifi` counters showed
sending was healthy (sent=219 ok=221 failed=0 queued=0 skew=0), but rx=1494 with
dropped=207 Malformed. sex-net's TCP flows give up after 30 s without ACKs, which
is the "network error".

- `link::amsdu_parts`: a QoS data frame with the A-MSDU bit set whose body is
  not one LLC/SNAP payload is taken as subframes (DA, SA, length, LLC payload,
  padded to 4 bytes from the first). Each subframe is passed up on its own (header
  with the bit cleared, the same CCMP header, its payload), with its own DA/SA,
  since address 3 is the BSSID in an A-MSDU. Before, every such frame was dropped
  as malformed. Frames the hardware already split (LLC/SNAP body) are unchanged.
- `Link::malformed` keeps the first 48 bytes, length, RX status and subframe of
  the last frame dropped as malformed. `[wifi.link.now3]` logs them, and plain
  `wifi` shows that line, so the next hardware run shows which check refused it.
- Test: `x17_carries_ethernet_over_wpa2` delivers an unsplit two-subframe A-MSDU.
