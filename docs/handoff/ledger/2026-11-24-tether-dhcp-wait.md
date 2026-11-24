# Tether waits for DHCP; filtered `hw` stops flooding the log (2026-11-24)

- Spindle `tether A.B.C.D`: after the network is up, it now waits (fresh 20 s) for an
  IPv4 address from DHCP before connecting. sexnet calls the network up with only an
  IPv6 (SLAAC) address, and an IPv4 connect then failed at once as "network error".
  It now says "waiting for an IPv4 address from the router (DHCP)", and if none comes,
  "no IPv4 address: the router never answered DHCP".
- `hw TEXT` / `hw all` no longer echo their lines as `[spindle.hw.line]` into the
  boot log: four filtered `hw` calls filled the 64 KiB newest part with copies, so the
  next `hw TAG` found nothing (x17, 2026-11-24). Plain `hw` still echoes (gates read it).
- Kernel console: per-keystroke lines (`[spindle.text.*]`, `[spindle.view.text]`,
  `[spindle.line.edit*]`, `[display.text.prepared]`, `[display.text.committed]`,
  `[display.typed.released]`) still go to the serial port but no longer into the
  64 KiB log ring or the 512 KiB head: five a keystroke pushed every Wi-Fi line out
  before `hw` could read it.
- Tether over IPv6: `tether [fdxx::y]:7177` (or a bare IPv6 address) connects with
  CONNECT_TO and needs no DHCP. tether_host.py now listens on IPv6 and IPv4 on one
  socket and prints its IPv6 addresses too (the x17 gets IPv6 from the router over
  Wi-Fi while DHCPv4 never finishes).
