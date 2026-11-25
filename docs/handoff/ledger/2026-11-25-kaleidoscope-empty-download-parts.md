# Kaleidoscope empty download part cleanup

For public downloads, Kaleidoscope durably writes an exact-name ticket in its
own `Kaleidoscope/empty-parts` state before creating
`Downloads/.kaleidoscope-NAME.part`. A hard stop before the first write can
leave a zero-byte part. On startup it deletes that part only when a valid
ticket names the exact file and no validated resume record protects it.
Unknown files survive, including a person's `.notes.part` and an unticketed
`.kaleidoscope-NAME.part`. Nonempty parts remain for manual recovery. Tickets
are retired after a transfer and stale tickets for absent or nonempty parts
are pruned on startup. Private downloads do not persist tickets or history.
Validated older resumes still read their generic `.NAME.part` files.
The downloads panel lists retained hidden `.part` files for review in Loom.

The `sex-web` host test covers the naming, size, directory, and protected-record
boundaries. The GitHub CI downloads QEMU gate stages an empty and a nonempty
ticketed hidden part, checks that an unticketed same-prefix file and a generic
`.notes.part` survive, and checks the cleanup receipt and Files listing. It
saves `browser-download-empty-parts-cleaned.ppm` and
`browser-download-hidden-parts-review.ppm` beside its other panel screenshots.
The same-disk hard reboot repeats cleanup with a ticketed zero-byte part and
checks pruning of stale tickets for an absent and a nonempty part.

No kernel, network driver, tether, or filesystem service behavior changes.
