# Valentine mail broker route, 2026-11-25

## Gap

Thunderbird and Apple Mail connect to accounts and send and receive mail.
Valentine currently handles local `.eml` and drafts. Its Multipass core has
grant-checked IMAP/SMTP logic but no boot service, verified dialer, or app route.

## Change

Reserve four mail operations on the existing SexNet web route shared with
Shell-bound Program rows. The service uses the kernel-stamped live domain/task
identity and its Shell-owned per-run binding. Only Shell's boot task or the
current bound Program task enters the reserved operations. The interim reply
is `NO_MAIL_ACCOUNT`; `MAIL_SUBMIT` never reports a send. No credential bytes
are accepted from Valentine, no network flow is opened, and no kernel, Wi-Fi,
or tether code changes. Ctrl+M uses a credential-free broker status call in
`run +net valentine`; a run without the grant gets an honest denial.

## Verification

- Focused `sex-pdx` tests cover current, stale, unbound, forged-task, browser,
  Spindle, and Shell identities, malformed flow/sequence, and submit refusal.
- The `program_valentine_gate.py` QEMU gate runs Valentine with the person's
  network grant, requires SexNet's exact caller/task route log and a status 9
  reply in Valentine, then captures `valentine-account-status.ppm`.
  Full build and QEMU gate run on GitHub CI for this exact branch head.

## Next

Shell-owned account setup and Shibari custody, a SexNet verified TLS mail
dialer, broker-owned IMAP/SMTP operations with per-account grants, and a
Valentine client. Until those land, the UI continues to say account mail is
unavailable.
