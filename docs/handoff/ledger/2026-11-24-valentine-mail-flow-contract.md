# Valentine mail flow request contract

- Added a strict versioned `OPEN_MAIL_FLOW` region encoding for IMAP and SMTP. It is exactly four bytes, has no password, token or caller-selected endpoint, and derives read versus write scope from the protocol. A reply is usable only when it is successful, matches the request sequence, and names a nonzero flow.
- Focused host tests cover valid and malformed requests, protocol scopes, stale replies, and refused or empty flow IDs. The existing Valentine QEMU gate still captures `valentine-account-status.ppm` to prove the guest does not imply mail works before Multipass and TLS flow handoff are booted.
- This is a protocol prerequisite only. The Multipass service, account-link UI, Shibari-backed SASL, app TLS flows and capability-safe transfer remain unimplemented. The native mail window continues to refuse live accounts.
- Validation: focused `rustc --test` run locally; full image build and QEMU gate only in GitHub CI. No target or disk image is kept locally.
