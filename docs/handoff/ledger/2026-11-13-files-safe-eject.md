# Files: safe eject, grants end with the stick, pulled-out sticks reported

Workstream E, slice E8 of `docs/handoff/DAILY_DRIVER_NEXT_100_SLICES.md`.

- Ctrl+E on a stick or disk (or on it in Ctrl+D disks) ejects it: every volume on that USB disk is
  written out and the stick flushed (`sexfiles` `volumes::unmount`); only when all of that succeeds
  are they forgotten and "can be pulled out safely" shown. In a trash folder Ctrl+E still empties it.
- Grants end with the volume: on eject and on pull-out, `fs_service::revoke_under("/media/NAME")`
  removes every run's `@/media/NAME...` grants (start arguments and picker answers) and clears what
  runs were told about those paths, so a different stick later mounted under the same name is not
  reachable through an old grant.
- ABI/policy change: programs may now send UNMOUNT for a `/media/NAME` their grant covers whole
  (it was Spindle-only). MOUNT stays refused to programs. New `sex_app::files::eject`.
- Pulled out without Eject: Files notices within a second (or when a copy fails) and says a file
  being written may be cut short. Half-copied `.NAME.part` files are hidden from the listing.
- Gate `scripts/files_eject_gate.py` (CI devices job): stick pulled mid-copy is reported, its
  existing files intact on the host, the old run's grant is gone after re-plug; second stick copied,
  checksummed, ejected, pulled: host `fsck.vfat -n` clean and the copy's SHA-256 equal.
