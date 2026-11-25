# Valentine's Multipass mail broker prerequisite

- Multipass core now checks a mail account's configured IMAP and SMTP servers through a trusted certificate-verified TLS dialer and authenticates both with the app password held in Shibari. Failed setup creates no stamp. The caller's setup buffer is wiped on every return path.
- A granted app can ask for an IMAP read flow or SMTP write flow. The broker dials only saved account endpoints and returns a SASL-authenticated stream; denied apps do not trigger a dial. It never returns the password or a Shibari knot. Mail stamps have a nonsecret serialized form.
- Host fixture tests cover send then receive, grant and endpoint enforcement, bad credentials, invalid endpoint, input erasure, and stamp serialization. `cargo test -p sex-multipass` passed (16 unit and 8 integration tests). Full builds and QEMU run only in GitHub CI.
- The native Valentine's existing account-status screenshot still honestly says Multipass is unavailable: this code is a service core, not a booted service. Boot grant routing, a TLS-capable SexNet app flow, flow ownership transfer, and the Valentine UI integration remain open.
