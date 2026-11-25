# Date CalDAV conflict review

Date already retained losing local event copies in a local Conflicts calendar.
The `V` key now opens a read-only comparison of each retained copy and the
current server event. Repeated `V` cycles through conflicts; Escape closes it.
The view never pushes either copy or silently discards the local version.

The `sex-calendar` host test checks ordering, wraparound and server pairing.
The CalDAV QEMU gate creates a real reply/server-update race, checks that both
versions reach the review, and captures `date-conflict-review.ppm` in the
productivity proof artifact. Full builds and QEMU checks run only on GitHub CI.
