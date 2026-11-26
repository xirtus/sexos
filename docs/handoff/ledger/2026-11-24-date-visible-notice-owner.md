# Date visible notice ownership

- Each visible Date run writes a Date-private Files lease and elects the lowest live task as Bell notice owner. A lease expires after ten seconds without a refresh, allowing another visible run to take over after a crash.
- The elected run reloads changed saved events and clock data, plus the notice cursor, before each delivery check. A hidden worker keeps a fresh Files claim while a window is visible. A crashed window's lease and opening marker expire after ten seconds; the worker then rereads saved state and resumes notices without a Date relaunch.
- The calendar QEMU gate opens two Date windows, checks a timer posts once, closes the owner, checks the remaining window posts its timer, and captures `date-two-windows.ppm`.
- Targeted calendar compile and gate syntax check ran locally. Full build and QEMU run on GitHub CI. A power cut between a Bell post and cursor save can still replay a notice. A worker that crashes while Date stays closed has no system supervisor and is restarted at the next visible launch.
