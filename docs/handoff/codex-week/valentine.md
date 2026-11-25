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

- Link accounts through Multipass and obtain authenticated TLS IMAP/SMTP flows
  without giving Valentine a password. Then connect the native inbox and composer
  to those flows, with IDLE notifications through Bell. The mail clients can now
  resume a flow after Multipass authenticates it, but the boot-routed Multipass
  service and a capability-safe flow handoff do not exist yet.
- JMAP, offline cache and conflict handling, account-backed folders and flags, sent copies,
  real attachment open/save via Loom, and a safe HTML view through sex-html.
- Search across accounts, conversation view, filters, signatures, keyboard
  undo. Local drafts now save through staged replacement
  and recover from the `.eml` file, but account-backed send and sync remain.

The native window and `docs/apps/valentine-shortcuts.md` now provide an offline
keyboard reference for the controls that currently work.

The local `.eml` viewer does not claim live mail sync or sending.
It can now list immediate child folders within its granted local directory, and
drafts created there stay there. Nested folder trees and account folders remain.
