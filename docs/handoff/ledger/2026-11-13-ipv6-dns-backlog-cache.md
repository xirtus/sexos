# IPv6-only DNS, accept backlog, bigger UDP queues, resolver cache (workstream K1, K2, K4a)

- **K1** `crates/sex-net/src/v6config.rs`: router advertisements are read for RDNSS (RFC 8106, lifetimes honoured, 0 withdraws) and the M/O flags; O or M starts stateless DHCPv6 (Information-Request to ff02::1:2 from port 546, DUID-LL, RFC 8415 backoff, refresh time). `Engine::open_resolver()` picks the leased IPv4 resolver, else the first IPv6 one, so an IPv6-only network now resolves names. `open_resolvers6()` lists them for a future Network page.
- **K2** Accept backlog: `open_listen_group(flow, port, group)` lets many flows wait on one port; `open_accepted(group)`, `open_take(flow)`, `open_backlog(group)`. Host test: 64 peers connect at once, all accepted. UDP queues 16 -> 64 datagrams and 16 -> 64 KiB each way; 1000 datagrams arrive with none lost.
- **K4a** Resolver cache in the engine: 512 names, TTL clamped 5 s..1 day, 30 s for names with no address, A-only answers never stand in for A+AAAA, flushed when the DHCP lease changes. DNS replies are taken every turn, so stray datagrams can't crowd out the answer.
- Gate: `scripts/program_ipv6_gate.py` now advertises RDNSS + O flag, answers DHCPv6 and DNS, and loads `http://six.test/` by name on a network with no IPv4; added to CI with `cargo test -p sex-net`.
- sexnet is untouched (the x17 networking thread owns it); it gets all of this through the calls it already makes. Backlog needs a sexnet ACCEPT change to use, after that thread merges.
- No ABI change, no syscall.
