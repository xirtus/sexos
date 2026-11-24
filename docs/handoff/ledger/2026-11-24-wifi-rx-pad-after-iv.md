# 2026-11-24: Wi-Fi RX pad sits after the CCMP header (x17 unicast fix)

On the x17 (AX210), every protected unicast QoS data frame was dropped as
`Malformed` (`[wifi.link.rx] dropped=49 last_drop=Some(Malformed)`), while
broadcast frames worked. So ARP replies, DHCP unicast replies and TCP to the x17
never arrived; DHCP only worked once it asked for broadcast replies.

Cause: with the descriptor's PAD flag the firmware puts its 2 alignment bytes
after the IV of a decrypted frame, not right after the 802.11 header
(`iwl_mvm_create_skb`: "The padding is inserted after the IV"; `iwx_rx_mpdu_mq`
adds IEEE80211_CCMP_HDRLEN for CCM frames). `scan::parse_mpdu` removed two bytes
of QoS control/IV instead, so the ExtIV check failed. Broadcast frames from the
AP are non-QoS (24-byte header, no pad), so they were unaffected.

The host model (`tests/driver.rs` `mpdu`) placed the pad after 24 bytes, which
matched neither layout and hid the bug (its QoS control was zero); it now pads
where the firmware does.
