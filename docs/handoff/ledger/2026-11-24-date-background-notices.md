# Date background notices

- Closing Date starts a hidden Date run using only its existing Files grant. It keeps the saved calendar, clock and notice cursor, posts due reminders, alarms and timers through Bell, and stages durable clock/cursor updates.
- The hidden run claims one worker file. Opening Date creates a handoff marker before its window and waits for the worker to exit; a stale claim from a dead run is cleared after the handoff wait. The visible run retains the marker until close, including during an early worker launch.
- The calendar QEMU gate now requires Bell delivery before Date reopens and captures `date-background-alarm.ppm`. It then reopens Date and checks the handoff and cursor load. Targeted `calendar` app check and gate syntax check ran locally; full build and QEMU run on GitHub CI.
- A worker that dies while Date stays closed is not restarted until a visible Date run closes again. Multiple visible Date runs and exact-once delivery across a power cut remain open gaps.
