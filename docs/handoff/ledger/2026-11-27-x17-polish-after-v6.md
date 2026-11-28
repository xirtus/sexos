# 2026-11-27: x17 polish after version 6 (Wi-Fi save, tether reconnect, resume)

Found on the x17 once version 6 ran (all from the Mac over the tether):
- **Wi-Fi never auto-joined**: `/data/wifi.conf` was a name whose inode was gone (a cut-short
  write). `ls` showed it at 0 bytes (Files lists unstatable entries as size 0), `rm` said
  "not found", and `wifi save` failed ("could not create": CREATE saw the name). sexfs now
  treats such a name as absent: `create` replaces it and `remove` drops it. Host test
  `dangling_tests` in crates/sexfs.
- **Tether needed several rounds to connect after a restart**: `tether_host.py serve` held one
  session at a time, and a machine that restarted never closed its old flow, so new connects
  sat in the backlog. A newer connection now replaces the open session.
- **Download resume** also retries a failed reconnect (Wi-Fi coming back), up to 10 tries with
  growing pauses; before, only a cut stream was resumed.
- `update` wording: "the boot menu records version N as accepted" instead of "this is version N"
  (the running image can be newer than the recorded version after a trial that never ran).
