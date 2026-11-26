# 2026-11-26 Wi-Fi: no protected management frames on WPA2

With the power fix in, the x17's tether still died: `wifi` showed only 19 unicast
frames ever received (unicast_in=19) while broadcasts kept coming, and a session
"ba tid=8" (no such TID). Our RSN element offered MFPC on WPA2, so the WPA2/WPA3
router turned PMF on. Its ADDBA requests then came CCMP-protected and were parsed as
garbage, our ADDBA response went out unprotected (a robust action frame the router
must drop, and may answer with SA Query we never handle), and a protected deauth or
disassociation would have been ignored too. Unicast to the x17 stopped.

WPA2 now leaves PMF off (`own_rsn` caps 0), as wpa_supplicant does by default; WPA3
(SAE) keeps MFPC|MFPR. Protected management frames are not parsed as plain ones.
