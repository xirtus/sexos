# Qupid: workstream G lands, Music becomes the library view

PR #183 (subtitles, sound tracks, speed, playlist and resume, sound panel, HDMI/DP
and USB audio, A/V sync gate, gapless library) is merged onto master after the
Bluetooth work. The player is now the program `/bin/qupid` (crate `apps/play`):
the old 93-line `apps/qupid` stub and the separate `apps/music` program are gone,
and the music library is `run qupid --library @FOLDER` (`apps/play/src/library.rs`,
serial tags `[qupid.library.*]`). Files open media with `qupid` (sex_pdx::launch,
sex-config). ABI: `sex_pdx::usb_audio` moved to slot 1009 / region 0x4406 because
Bluetooth's `bt_hci` took 1003 / 0x4402. The CI media job now runs the subtitle and
gapless gates.
