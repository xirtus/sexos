# Kaleidoscope: clear download history

- Ctrl+Alt+J in `about:downloads` clears completed download history and the current failed-transfer list after a durable state replacement. It removes the old history recovery copy too, so a later damaged live file cannot resurrect the cleared list. It leaves downloaded files and resumable transfers in place. A failed state write leaves the panel list available and reports the failure.
- The panel and offline shortcut reference explain the control. GitHub CI's downloads guest gate captures `browser-download-history-cleared.ppm` and verifies that the empty history survives a browser restart.
- A process killed mid-transfer may still leave an orphan hidden part. That requires a safe browser-owned file namespace before automatic cleanup.
