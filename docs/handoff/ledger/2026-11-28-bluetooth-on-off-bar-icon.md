# 2026-11-28: Bluetooth on/off, the Bluetooth key and a Bluetooth icon in the bar

Bluetooth (Cufflinks) can now be turned off and on: from the Bluetooth panel's new first
row, its B key, the Bluetooth key (Shell code 0x74, Input usage 0xEF; F9 on the x17 once
its producer is known) or the `bluetooth.on` setting. Off drops every link, stops scans and
resets the controller; the choice lasts across boots. The bar shows a Bluetooth icon left of
the network icon while a controller is there: dim when off, lit when on, green while a
device is in use; a click opens the panel. Details: CUFFLINKS_BLUETOOTH_2026_11_14.md
section 22. No syscall, kernel or ABI-pinned file changed.
