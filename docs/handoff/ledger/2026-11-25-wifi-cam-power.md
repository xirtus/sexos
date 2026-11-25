# 2026-11-25 Wi-Fi: really stay awake (MAC_PM_POWER_TABLE flags 0)

The x17's plain `tether` died about 30 s in, twice (16 KB, then 105 KB of the boot
log reached the Mac), and after that the same boot never reached the Mac again:
a `tether sweep` on it sent ARP requests every second (arp out +21 a trial) but not
one IPv4 packet (v4_out stuck at 142), while broadcast kept arriving. The Mac's
unicast ARP replies never came, so smoltcp never had a neighbour to send the SYN to.
The sweep's `idle45` shape (45 s quiet, then data) had failed 16 of 16 the same way.

Cause: `join::mac_power_awake` set POWER_MANAGEMENT_ENA (bit 1). In
`iwl_mvm_power_build_cmd` that bit is 802.11 power save ON ("use rest of
parameters"), and every other field was zero, so the firmware dozed after each frame
and the AP buffered all our unicast. Linux with power save disabled returns before
setting any flag; the command now carries flags 0 and only the keep-alive (CAM).

Also: Spindle's tether sends at most 4 KiB per network call (the size the sweep
proved), the link skips Null and QoS Null frames instead of counting them as not
decrypted, and `wifi` shows `unicast_in` (frames addressed to us) in `[wifi.link.now2]`.
