# Date: retain reminders and alarms when Bell transport fails

Date's visible owner and hidden worker previously advanced the durable notice
cursor after a failed Bell PDX post. A one-time alarm was also disabled before
posting. Both paths now use one due-minute delivery routine. It stops at the
first failed transport, leaves that minute pending, and retries after five
seconds. The one-time alarm is disabled only when its post succeeds.

Delivery remains at least once. Earlier successful posts in the same failed
minute can repeat on retry, and a power cut after Bell accepts a post but before
the cursor save can also repeat it. Bell does not acknowledge its mute, spam or
queue policy through this API.

Focused host test: `cargo test --locked -p sex-calendar
failed_bell_post_retains_reminder_and_one_time_alarm_until_retry` injects a
failed alarm post after a successful same-minute reminder, checks the retained
minute and enabled alarm, then checks successful retry and bounded delay.
`cargo check --locked -p calendar --bin calendar --target x86_64-unknown-none`
checks the guest app. The exact-head GitHub CI runs the full build and Date QEMU
gate, including `date-background-alarm.ppm`. That gate proves healthy background
delivery and a screenshot; it does not inject Bell transport failure in QEMU.
