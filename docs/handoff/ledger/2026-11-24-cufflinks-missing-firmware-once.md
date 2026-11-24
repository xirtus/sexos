# 2026-11-24: Cufflinks stops retrying a controller whose firmware is missing

The x17 report (hw save) showed 810 `[cufflinks.controller]`/`[cufflinks.firmware]
missing=ibt-0041-0041.sfi pack=1` lines. After a missing-firmware failure,
Cufflinks went straight back around its loop: the same controller (same link
generation) was still there, so it asked again with no wait. That filled the kept
boot log and pushed the Wi-Fi link lines out, so `hw wifi.link` showed nothing.

Now a missing file marks that generation, and Cufflinks waits until SexUSB reports
the controller again (replug or reset). The log line ends `retry=replug`. The
panel still shows the controller as failed.

Still open: the firmware pack on the x17 image has no ibt-0041-0041.sfi
(scripts/fetch_bt_firmware.sh lists it), so Bluetooth stays off there.
