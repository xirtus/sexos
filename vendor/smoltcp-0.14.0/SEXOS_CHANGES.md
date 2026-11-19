# smoltcp 0.14.0: independent discovery clocks

Source: the official crates.io `smoltcp` 0.14.0 archive, exactly the version
already selected by SexOS's lockfile. Archive SHA-256:
`b6f8b28ad56c6e35524a37dd492af5d1a47e31e1a4d175cd12f89c075f01980f`.
The original 0BSD license is retained in `LICENSE-0BSD.txt`. No Cargo registry
files were modified. The workspace selects this copy with a local Cargo patch.

Upstream uses one neighbor-discovery silence timestamp for IPv4 and IPv6.
Socket egress uses insertion order. A fixed IPv4 socket waiting for an absent
gateway can send ARP at each one-second boundary before a newer IPv6 socket
gets to request its neighbor; the IPv6 connection then never sends its SYN.

The local patch changes only `src/iface/neighbor.rs` and its one production
caller in `src/iface/interface/mod.rs`. Two fixed timestamps retain the existing
one-second discovery limit independently for each IP family. Cached neighbors
still bypass discovery throttling. The additional timestamp uses eight bytes,
allocates nothing, changes no socket state or handles, and keeps unanswered
IPv4 work alive. At most one discovery per family per second is permitted.
This addresses cross-family progress, not arbitrary same-family scheduling.

The production Engine regression in `crates/sex-net/src/tests.rs` fails against
the unmodified locked crate and passes with this patch. Upstream neighbor-cache
tests additionally exercise the unchanged expiry/cache behavior and independent
family clocks. Actual guest validation uses the unchanged IPv6-only peer/gate.
