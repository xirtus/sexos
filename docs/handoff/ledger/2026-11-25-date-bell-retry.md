# Date: retain reminders and alarms when Bell transport fails

Date's visible owner and hidden worker previously advanced the durable notice
cursor after a failed Bell PDX post. A one-time alarm was also disabled before
posting. Both paths now use one due-minute delivery routine. It stops at the
first failed transport, leaves that minute pending, and retries after five
seconds. The one-time alarm is disabled only when its post succeeds.
On a fresh profile, Date commits the initial cursor before checking due notices.
If that save fails, delivery pauses. On reopen with saved event or alarm state
but no valid cursor, Date commits a bounded seven-day replay baseline before
posting. Older notices in that window may repeat; a notice whose underlying
event or alarm was never saved cannot be recovered.

Delivery remains at least once. Earlier successful posts in the same failed
minute can repeat on retry, and a power cut after Bell accepts a post but before
the cursor save can also repeat it. Bell does not acknowledge its mute, spam or
queue policy through this API.

Focused host tests inject a failed Bell post and a failed initial cursor save,
then simulate close and reopen with saved state. The first checks retained
minute, enabled one-time alarm, successful retry and bounded delay. The second
checks that delivery pauses until the baseline is durable and the saved alarm
remains in bounded catch-up. Run with `cargo test --locked -p sex-calendar
notice_cursor::tests`.
`cargo check --locked -p calendar --bin calendar --target x86_64-unknown-none`
checks the guest app. The exact-head GitHub CI runs the full build and Date QEMU
gate, including its fresh-profile cursor initialization assertion and
`date-background-alarm.ppm`. That gate proves healthy background delivery and a
screenshot; it does not inject Bell transport failure in QEMU.
