# Date notice catch-up cursor

- Date keeps its last checked reminder/alarm minute in a staged, synced file inside its own Files grant. Reopening the window checks a missed interval capped at seven days, then advances the cursor after posting notices.
- Invalid or future cursors start at the current minute. A machine without a civil clock does not replay stale years of notices. An overdue timer already comes from `clock.txt`.
- Host tests cover cursor parsing, bounds and invalid state. The calendar QEMU gate closes Date before a one-time alarm, waits for it to become due, reopens Date, checks Bell delivery and captures `date-overdue-alarm.ppm`.
- This is catch-up on reopen. Bell delivery while Date remains closed still requires a singleton background worker. A crash between Bell posting and cursor commit may duplicate a notice.
- Full image build and QEMU test run on GitHub CI; only focused host tests and targeted app checks run locally.
