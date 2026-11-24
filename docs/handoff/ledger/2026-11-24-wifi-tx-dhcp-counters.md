# 2026-11-24: Wi-Fi TX answers and DHCP counters (x17 diagnosis)

On the x17, Wi-Fi joins Midnight1 (WPA2) and IPv6 SLAAC configures from router
advertisements, but DHCPv4 never gets a lease. SLAAC needs no transmit, DHCP
does, so the question is whether our protected data frames reach the AP.

- `crates/sex-wifi/src/link.rs`: `Link::tx_ok`, `tx_failed`, `tx_last_failure`
  count the firmware's TX responses for data frames (status 1/2 = sent).
- `servers/sexnet/src/wifi.rs`:
  - `[wifi.link.txdone] ok= failed= last_failure=` when those change (first 16,
    powers of two, new failures).
  - `[wifi.link.ip] dhcp_sent= dhcp_answers= arp_sent= arp_in= v4_sent= v4_in=`
    counts IPv4, ARP and DHCP (UDP 67/68) frames each way on the link.

On the x17: `hw wifi.link` shows both. Reading them:
- dhcp_sent=0: sexnet's DHCP client never started (open side not attached).
- dhcp_sent>0, failed>0: the firmware refuses or gets no ACK (look at last_failure).
- dhcp_sent>0, ok>0, dhcp_answers=0: frames leave but the AP/router drops them
  (encryption or addressing).
