# Tether waits for DHCP; filtered `hw` stops flooding the log (2026-11-24)

- Spindle `tether A.B.C.D`: after the network is up, it now waits (fresh 20 s) for an
  IPv4 address from DHCP before connecting. sexnet calls the network up with only an
  IPv6 (SLAAC) address, and an IPv4 connect then failed at once as "network error".
  It now says "waiting for an IPv4 address from the router (DHCP)", and if none comes,
  "no IPv4 address: the router never answered DHCP".
- `hw TEXT` / `hw all` no longer echo their lines as `[spindle.hw.line]` into the
  boot log: four filtered `hw` calls filled the 64 KiB newest part with copies, so the
  next `hw TAG` found nothing (x17, 2026-11-24). Plain `hw` still echoes (gates read it).
