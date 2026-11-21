# Protected recording Stop precedes Atlas and capture

The visible Chip0 recording target now reaches Stop before Atlas consumes clicks
and before a program captures a pointer-down event. Both inline and main HID
paths, plus the legacy raw USB path, use the same priority helper. The raw USB
path now requires the fixed authenticated Input PD/main task, matching HID.
A pending Display scale publication still fences pointer geometry; the helper
uses the same reflow/scale contract as Display's marker.

Stop is sent first. An Atlas drop intent is cancelled, while existing program
capture and button state stay intact so the normal button-up path can release
them. The helper does not apply drag/snap geometry or open ordinary SilkBar
controls through Atlas. This avoids swallowing an app's outstanding release.

Three source-integration tests passed with
`python3 -m unittest scripts.tests.test_recording_stop_routing` (0.005 s).
The existing actual marker/cursor pixel test now additionally checks every
marker pixel against the actual shared Chip0 hit action at scales 1–3; that new
assertion awaits the next compiler slot. No Rust compiler/VM ran for this change.
Native Stop while Atlas/capture is active remains an integration acceptance item.
