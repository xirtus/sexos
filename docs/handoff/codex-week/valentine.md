# Valentine daily-driver gaps

Compared with Thunderbird and Apple Mail, Valentine has a strong protocol core
(`sex-mail`): IMAP folders, UID operations and IDLE, SMTP submission, MIME parsing,
plain-text composition, attachments and conversation threading, with a host
send-then-receive fixture. It had no native window.

This first native slice lists and searches `.eml` messages inside Valentine's Files
grant, opens decoded plain-text mail and shows attachment counts. HTML is kept
closed, so embedded remote trackers do not load. It follows the SexOS appearance
theme. A QEMU gate checks the real window and preserves screenshots.

Remaining gaps:

- Link accounts through Multipass, connect Valentine's native inbox and
  composer, and deliver IMAP IDLE notifications through Bell. Multipass's core
  validates configured IMAP and SMTP servers, checks both sign-ins, stores the
  app password in Shibari, and enforces read/write grants. Its new broker-owned
  operations page inbox UIDs, fetch an exact message, and submit plain-text
  mail from the saved account over authenticated flows; Valentine need not
  receive a network flow or credential. The earlier `OPEN_MAIL_FLOW` PDX
  contract remains unimplemented and would need a flow ownership transfer.
  The broker-owned path still needs a boot-routed Multipass service, a verified
  TLS SexNet dialer, account setup and grant UI, and a Valentine client. Bell
  has no mail event source yet.
- JMAP, offline cache and conflict handling, account-backed folders and flags, sent copies,
  real attachment open/save via Loom, and a safe HTML view through sex-html.
- Search across accounts, conversation view, filters, and signatures.
  Draft fields now support bounded Ctrl+Z/Ctrl+Y history, including changes after
  save. Local drafts save through staged replacement and recover from the `.eml`
  file. A new draft's first save now refuses to replace a name taken by another
  writer and retries under a new name; unrelated Files errors stop name selection.
  A failed close-time save leaves the composer open. A two-boot QEMU gate checks idle draft autosave after killing the
  guest with the composer open. Account-backed send and sync remain.

The native window and `docs/apps/valentine-shortcuts.md` now provide an offline
keyboard reference for the controls that currently work.

The local `.eml` viewer does not claim live mail sync or sending.
It can now list nested folders within its granted local directory, bounded to
four levels and 64 folders, and drafts created there stay there. Account-backed
folders still require the Multipass service route.
