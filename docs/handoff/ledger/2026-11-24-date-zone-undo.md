# Date world-zone undo

- Added a bounded, in-session Ctrl+Z/Ctrl+Y history for Clock's world-zone additions, removals, and home-zone changes. It saves each restored state immediately. The history snapshots only zone fields so a timer or alarm that fires between edits cannot be resurrected by undo.
- A focused host test checks undo/redo, timer preservation, and refusal after an external zone change. The Calendar QEMU gate adds Chicago, undoes and redoes it, and captures `date-zone-undone.ppm` and `date-zone-redone.ppm`.
- Alarm/timer edits and invitation replies remain outside this undo history. Full image build and QEMU gate run only in GitHub CI; no target or disk image is kept locally.
