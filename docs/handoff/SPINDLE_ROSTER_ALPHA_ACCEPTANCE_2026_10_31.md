# Round 144: sustained acceptance of the recovery-roster image

The earlier Round 140 recovery-roster image completes both 1800-second mixed
desktop sessions. These are its two actual Spindles12→17 and eight Quil reserves;
they are separate evidence from the newer Round 142 startup and Round 143
three-Spindle images. The original runner command label 142 remains in raw data;
144 is the completed acceptance milestone.

| Profile | Actual soak seconds | Cycles | Verified captures |
|---|---:|---:|---:|
|1024×768 /e1000|1800.0029996660305|87|196|
|1280×800 /e1000e|1809.049743416952|86|194|
|Total|two complete sessions|173|390|

Both terminal runs perform real Quil input/save/load, Spindle commands and
durable history, Linen refresh, Browser/Quil/Spindle cleanup, all eight Quil
incarnations and the exhausted launcher. Each exact stopped artifact passes a
fresh source/image-bound read-only replay, including registered pixels, serial
windows and final durable document bytes. Terminal Quil Close completed its save;
these sessions do not relabel that result as pending device-I/O teardown.

Source `93a505983cfc71b253e5f7bb974bcfd86257a632` matches 1,165 distinct committed blobs
(576 production +589 runner). Its existing qualification supplies 43 host gates,
2,698 Rust executions, 989 Python tests and the separately documented same-image
short-workflow cold17→12 recovery. These long-session disks were not themselves
cold-reboot tested. Kernel/layout and BOOT authority are unchanged from Round 140.

ISO SHA256: `669ce912f1f43f93f543ba77484c0e825a0ee35012b60b8b3d1db015bbf6dcd6`  
Kernel SHA256: `b01a0464c6b0d1d3d3afa90684ab0954b2889c1dadb45613c11ab17ba9d41a26`  
Architecture SHA256: `4085f8fa65234205c5fdab070fe36765d767a46edc001410bf2ed5bfb27a98d7`

[Machine evidence](assets/sexos-spindle-roster-alpha-paired-2026-10-31-evidence.json)
and [raw archive](assets/sexos-spindle-roster-alpha-paired-2026-10-31-raw.tar.gz)
contain 1,640 members /12,975,310 compressed bytes. Every member size/hash was
verified after reopening; archive SHA256:
`831878ce294536311542099b51ce686bacf159c3e4d615e56c95d46848362071`.

This accepts the specified virtual-machine workflows. Newer images need their
own sustained evidence. SASOS authority rules remain unchanged. The unrestricted
daily-driver goal remains open: **2 Open / 28 Partial / System Proven false**.
