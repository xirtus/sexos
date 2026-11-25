# Wi-Fi: reasons for every retry (2026-11-24)

sexnet's Wi-Fi `why` line (op 0x2E8 action WHY, shown by Spindle's `wifi` as
"the last try failed: ...") now also says why a retry happened when:

- a connected link dropped: the AP's deauth/disassoc reason, beacons missed, or the link failure;
- the scan after a restart failed;
- the join could not be started after a restart;
- the chip restart itself failed (firmware or layout).

Before this, these retries left the line empty, so a join that kept restarting
looked unexplained on the x17.
