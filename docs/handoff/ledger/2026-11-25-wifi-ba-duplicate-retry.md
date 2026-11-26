# Wi-Fi: block-ack duplicates need the retry bit; `wifi` shows the session

Date: 2026-11-25. On the x17 with b47d8fa4 the tether still closed after about 35 KB.
`[wifi.link.now3]` showed the frames dropped as malformed are the AP's spanning-tree
BPDUs (DA 01:80:c2:00:00:00, LLC 42 42 03, not SNAP), so they are harmless, and the
loss is elsewhere. Frames the block-ack session drops are counted nowhere visible.

- The reorder step dropped a frame whenever bit 22 of the RX descriptor's status
  word was set (`IWX_RX_MPDU_RES_STATUS_DUPLICATE`). On newer firmware that word is
  a 16-bit status followed by the A1 hash and filter status, so bit 22 can be set on
  good frames. A frame now counts as a duplicate only when it is also a retry (frame
  control retry bit), which every real duplicate is.
- `[wifi.link.now4]` (shown by plain `wifi`): the block-ack session's TID, window,
  head, frames held, passed, released and dropped, frames flagged duplicate, and the
  last frame to us with its RX status and reorder word.
