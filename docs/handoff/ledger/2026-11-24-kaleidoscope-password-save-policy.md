# Kaleidoscope password save policy foundation

The read-only autofill route cannot safely carry writes in reverse. This change
adds the two local decisions needed before a separate Shell-brokered write route:

- Kaleidoscope's policy captures only current values from one eligible
  top-level HTTPS same-origin POST login pair. Both fields must be present,
  nonempty and at most 4096 bytes. The capture owns zeroizing copies; it does
  not enter the browser profile or a serial record.
- Chastity accepts only a canonical exact HTTPS origin and bounded fields.
  An update can target only an entry with the same origin and username, and
  changes only its protected password. Adding a separate entry remains an
  explicit choice. The proposal owns a zeroizing password.

This is a policy foundation, **not a user-visible save feature**. A follow-up
must add an authenticated Shell transfer with cancellation and liveness checks,
then a Chastity approval screen, durable vault save, and a browser command.
The existing autofill route remains read-only. No kernel, device, network or
filesystem authority changes.

Host tests: `sex-web` autofill policy test and `chastity-core` save-login unit
tests, run in GitHub CI. The existing QEMU browser and Chastity suites remain
CI gates; they do not yet exercise a save action. The CI QEMU screenshot is
evidence of the unaffected native browser/Chastity baseline, not of saving.
