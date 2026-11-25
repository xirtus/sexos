# Date owner-gated Bell polling

- Gap: Date elected one visible Bell owner only on an idle event wait, yet ran
  due delivery after every handled key in every window. A second window could
  post a duplicate notice, while a continuous stream of ignored input could
  postpone the elected owner's due check.
- Change: one 500 ms cadence performs owner election, rereads shared calendar,
  clock and cursor state, and calls due delivery only for the elected owner.
  It runs independently of the window's input stream. The hidden worker handoff
  and at-least-once cursor behavior remain as before.
- Proof: a host cadence regression simulates uninterrupted input; the calendar
  QEMU gate keeps the non-owner busy over a shared timer deadline and then
  keeps the owner busy with pointer motion over another deadline. It checks
  exactly one Bell post and captures `date-owner-under-input.ppm`.
- Checks: focused `sex-calendar` test, Date x86 target check, Python syntax and
  `git diff --check` passed locally. Full image build and QEMU proof run only
  in GitHub CI.
