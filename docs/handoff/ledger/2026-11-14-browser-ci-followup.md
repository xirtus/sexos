# Browser PR 274 CI follow-up

Head cda3adfd's CI run 43631935393 passed host browser regressions, media, and
single-core JIT/W^X. Runtime IPv6 connect and terminal unprivileged-launch gates
failed; original job logs and artifacts are retained under target/browser-evidence.

The terminal native gate reached `files.ticket.denied` after Term had announced
`term.native.start`. General launch-ticket preparation introduced on master no
longer implies a terminal console grant. Term discarded the DENIED result from
relay release. Propagate that error before preparing a ticket; the existing native
negative gate still requires status 11 and no attached console, and now also
requires no native-start announcement. Targeted x86 Term build passed.

The IPv6 failure did not reproduce locally with the frozen ISO: TCP/UDP echo,
inbound TCP, HTTP by address, DHCPv6 and AAAA/DNS HTTP all passed, peer errors 0.
This does not clear the failed CI image. Keep all assertions and retain a bounded
256-frame peer trace plus peer counters/state on failure in the next CI run.
Native evidence: target/browser-evidence/ipv6-ci-repro, VM exited/reaped.

No merge until all current-head checks, including aggregate build, pass.
