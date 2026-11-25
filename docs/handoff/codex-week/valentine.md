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
  has no mail event source yet. The IMAP core now bounds any one announced
  literal and the accumulated response to 64 MiB, with 2 MiB of response
  framing text. Larger attachments need a streamed Files path rather than an
  in-memory response.
- JMAP, offline cache and conflict handling, account-backed folders and flags, sent copies,
  attachment opening, and a safe HTML view through sex-html. A local `.eml` attachment can now be selected and saved through Loom's Save picker. Valentine sanitizes the sender's suggested name, uses the exact picked Files grant, refuses an existing destination, and reports success after sync. Files returns the created SexFS object ID in the create reply; each attachment write and failure cleanup checks that ID inside its serialized request, so a same-name replacement receives no bytes and cannot be removed by the old writer. Removable formats report unsupported before creating because they lack stable IDs. A sudden stop can still leave an incomplete file because the picker grants only the chosen destination, not a same-volume staging name; a server-owned staged commit remains a gap.
- Search across accounts, conversation view, filters, and signatures.
  Draft fields now support bounded Ctrl+Z/Ctrl+Y history, including changes after
  save. Local drafts save through staged replacement and recover from the `.eml`
  file. A new draft's first save now refuses to replace a name taken by another
  writer and retries under a new name; unrelated Files errors stop name selection.
  A failed close-time save leaves the composer open. A two-boot QEMU gate checks idle draft autosave after killing the
  guest with the composer open. Account-backed send and sync remain.

Files now has staging tickets on primary SexFS and unlocked encrypted home,
with a private namespace, sequential checked chunks, a complete-size/digest
gate, no-clobber publish, and mount or unlock cleanup. Valentine's local
attachment export now uses the ticket, so a power cut cannot expose a partial
destination. The 4 MiB local message cap still limits attachment size, and
removable filesystems fail closed until they can offer the same guarantee.

The native window and `docs/apps/valentine-shortcuts.md` now provide an offline
keyboard reference for the controls that currently work.

The local `.eml` viewer does not claim live mail sync or sending.
It can now list nested folders within its granted local directory, bounded to
four levels and 64 folders, and drafts created there stay there. Account-backed
folders still require the Multipass service route.

The existing Shell-bound Program-to-SexNet route now reserves mail broker
operations for account status, inbox, fetch, and submit. SexNet checks the
kernel-stamped live caller and the exact current Shell binding; unbound,
retired, forged-task, browser, and Spindle callers are denied. With no
configured account or verified mail dialer, all authorized mail operations
return `NO_MAIL_ACCOUNT`, including submit. Ctrl+M now queries SexNet on a
`run +net valentine` grant and reports that state. This is the first transport gate,
not live mail: account setup in Shell, Shibari custody, TLS dialing inside
SexNet, broker-owned IMAP/SMTP operations, and a Valentine client remain.
