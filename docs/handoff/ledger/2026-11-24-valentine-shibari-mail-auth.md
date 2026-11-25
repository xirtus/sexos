# Valentine Shibari-backed mail authentication prerequisite

- Added a Multipass-side IMAP/SMTP SASL helper that reads an app-password knot only through Shibari's owner-checked closure and returns an already-authenticated stream. Its `VerifiedMailTls` bound requires the eventual trusted transport to verify the certificate and account hostname before SASL.
- Zeroized the raw SASL payload, encoded response, command and wire buffers in `sex-mail` after transmission. Host fixture tests prove IMAP read and SMTP submission work after this helper hands off the stream; wrong Shibari owner and wrong password return no stream.
- This does not create live Valentine accounts. SexNet has no app TLS flows or capability-safe flow transfer, Multipass is not booted, and the native window keeps its unavailable account state. The Productivity QEMU gate captures `valentine-account-status.ppm`.
- Validation: focused Multipass host tests locally, full image build and QEMU only in GitHub CI. Temporary target and disk images removed.
