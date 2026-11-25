# 2026-11-25 Wi-Fi: keep the radio awake (MAC_PM_POWER_TABLE)

The first x17 `tether sweep` (124 trials) showed the block-ack and replay switches
change nothing (the BA session never saw a frame: BAID 0x7f). Only 13 trials
connected; the Mac accepted 17, so its SYN-ACKs were lost; data to the Mac stalled
after 20-80 KB, while broadcast kept arriving and the slow trickle passed. That is
the signature of an AP buffering unicast for a station it thinks is dozing.

The driver never sent MAC_PM_POWER_TABLE (0xa9). After association
(`assoc::Step::Power`) it now sends `join::mac_power_awake`: power management on,
power save off, keep-alive max(3 DTIM, 25 s), as Linux does with power save off.
Test: `x17_authenticates_and_associates_like_net80211` checks the command.

`tether sweep` keeps only the link counters after each trial (not the network
list), so all trials fit the results file.
