# Wi-Fi stays connected (W6a)

Intel Wi-Fi now recovers by itself: a lost link (deauthentication, beacons gone,
firmware crash) or a failed join stops and restarts the chip (`csr::stop`, fresh
bring-up), rescans and rejoins the same network with backoff (1 s doubling to 30 s,
10 tries). The network name and passphrase live only in sexnet's memory for this
boot. A refused handshake forgets them. `wifi join` while busy switches networks;
`wifi leave` (new `wifi_status::LEAVE`) forgets and rescans. The link also sends
MCAST_FILTER_CMD (pass all) so IPv6/mDNS multicast arrives. Evidence: two new
sex-wifi driver tests in `scripts/wifi_host_gate.sh`; sexnet and Spindle build
for x86_64-sex. Radio behaviour is unproven until the x17. Doc: INTEL_WIFI section 18.
