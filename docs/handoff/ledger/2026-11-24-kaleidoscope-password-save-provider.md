# Chastity save proposal receiver

- Extended the existing Chastity provider SDK to receive a bounded browser save proposal over Shell's run-attested route. Origin and credential chunks are retained in zeroizing buffers; the SDK exposes a borrowed proposal only after ordered delivery of the full header and data, valid UTF-8, and nonempty fields.
- Added `approve_save`, explicitly reserved for after a person's consent and a durable encrypted vault commit. Its broker packet does not contain the password. Deny, cancellation, lock and timeout clear the proposal.
- A production-code host transport test exercises browser-to-Chastity transfer and provider-only final approval; 21 autofill host tests passed in a temporary target, removed after use. GitHub CI runs the full image and QEMU gate and captures the existing browser save request screenshot (`browser-password-save-request.ppm`).
- This is a receiver API. Chastity still needs the visible save consent and commit UI before any password save is functional.
