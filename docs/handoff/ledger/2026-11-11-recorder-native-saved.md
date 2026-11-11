# Opt-in recorder: native saved movie

Frozen source `7c7e22496dbc2df5e4ad72c163e950364c52e4ad`, before the subsequent master merge, passed the real QEMU desktop recording gate. This is an explicit `SEXOS_RECORDER=1` build; the default profile remains unarmed.

Session 16777217, actual reborn task 1073741824, captured and encoded 15 FFV1/BGR0 640×360 frames into `/recordings/desktop.mkv`. Independent FFmpeg decode succeeded. Packet timestamps exactly match actual Display publication-time deltas: 0, 430, 914, 1365, 2125, 2594, 3052, 3499, 3949, 4407, 4882, 5334, 5817, 6440, 6878 ms. The sixteenth publication (generation 72, publication_ms 14728) was explicitly dropped at Stop. Actual Display revoke/scrub precedes the matching Saved receipt with sync/release confirmed; no subsequent Display frame occurs in the retained trace.

Evidence directory: `/Users/xmacbookm5/Projects/sexos-evidence/desktop-recorder/native-capacity256/`. `completed-build.json`, `guest/result.json`, raw serial, movie, decoded pixels, probe output, and root independent output verification are retained.

- ISO SHA256: `43068aa811eab370df050c476b3f4e33914ec75dd0420d0dad3eab3825315b9c`
- Canonical recorder ELF SHA256: `0026d27f774221dfc20e062ac4140bca2ee8cd8f80c891530d8a5927c0f0c414`
- Movie SHA256: `af12e67b5ebc62784bac10750ab25c096e0b0d47bf27714853242893e1246860`
- Decoded RGB SHA256: `875415706963a80e63270a5e3c727257214fd81e824290fd521050f1fbc84d01`
- Serial SHA256: `542e0a5851da2e49e8949efa35371cb7be7bd26a54bea09ac580810752cf193c`

The complete 308-byte prepared Spindle view includes “Recording saved and synced in Recordings.” Its final Display commit was not observed before guest exit; this proves prepared renderer text, not final Saved pixels. The active screenshot is retained. Exact captured-pixel correspondence, trusted Chip0 Stop, hostile-domain memory denial, restart/KEEP cleanup, and hardware qualification remain separate pending gates.

Prior failed boot at 128 frame reservations is retained under `native-f44d42`; the bounded 256-entry table preserves exhaustion and ownership checks and boots successfully here. The real 32 MiB FAT retains firmware and the exact canonical recorder bytes. Staged ELF compaction excludes the kernel and validates every file-backed program-header range; unstripped build outputs remain retained. This is a usable happy-path feature result, not complete recorder safety or daily-driver qualification.
