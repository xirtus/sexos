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

First x17 result (image 4141b489): `dhcp_sent=6 dhcp_answers=8 arp_in=60 v4_in=330`,
`txdone ok=12 failed=0`. Wi-Fi TX and RX both work and the router answers; the
lease is lost inside SexOS. So each DHCP frame (first 24 each way) is now logged:
- `[wifi.link.dhcp] in|out type= xid= client= to=` (type: 1 discover, 2 offer,
  3 request, 5 ack, 6 nak; client = chaddr; to = Ethernet destination)
- `[wifi.link.dhcp2] in|out SRC->DST SPORT->DPORT you= fl= ip_len= frame= sum=`

Second x17 result (image f555571f): our DISCOVER left (`out type=1`, our MAC,
fl=0000) and the firmware reported it sent, but the only answers seen were for
other clients: another client's DISCOVER and an OFFER with the broadcast flag
(fl=8000) sent to ff:ff:ff:ff:ff:ff. No unicast frame to us shows in any count.
So either our protected TX doesn't reach the router or unicast RX is lost.

- DHCP client messages now go out with the BOOTP broadcast flag (0x8000; UDP
  checksum cleared) so the router answers by broadcast, under the group key that
  is known to work. If the lease comes now, TX works and unicast RX is the bug.
- `[wifi.link.unicast] in= type= len=` counts frames addressed to us alone.
