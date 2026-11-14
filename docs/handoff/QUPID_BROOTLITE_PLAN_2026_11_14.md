# Qupid: what to learn from brootlite (2026-11-14)

Andreas's brootlite (Android TV, macOS and Linux media suite: Kotlin + Swift + a
shared Rust core over libmpv) has many player features SexOS's player lacks. This
report lists them, says which ones Qupid takes and how they fit SexOS's own design,
and splits the work into slices.

Qupid is the media player. The rename thread (PR #211) makes Play into Qupid and
Music into Qupid's library; the old `apps/qupid` was a placeholder. Until that lands,
"Play" below means the program that becomes Qupid. Workstream G (PR #183) already
gives Play subtitles (SRT/ASS), sound track switching, speed, a playlist, resume,
controls and media keys, and gives Music gapless playback.

## brootlite inventory, against Qupid

| brootlite feature | Qupid today | Plan |
| --- | --- | --- |
| Keys: Space/K, J/L, arrows, F, M, Esc, Enter commits scrub | Space/K, J/L, arrows, Home, N/P, S, A, [ ] (#183) | Q2: M mute, F fullscreen ask, number keys jump to 10%..90% |
| Resume where left | Done (#183 `play-resume.txt`) | Q6: shown as "Continue watching" in the library |
| Audio, subtitle, video track lists with language, codec, size | A cycles audio tracks, unlabelled | Q4: labelled pickers, embedded text subtitle streams, subtitle and sound delay |
| Chapters list and jump | None | Q2: chapters from FFmpeg, list (C), PgUp/PgDn jump |
| Skip Intro / Skip Credits (Jellyfin media segments) | None | Q1 logic (done), Q2 card from chapter names, Enter skips |
| Series binge: "Keep watching, stream next" card with countdown, same release preferred | Playlist only | Q1 logic (done), Q3 card, next file from the folder or playlist |
| Credits "Next recommended" poster rail | None | Q6: same show / same folder unwatched items, from the library |
| Stream telemetry overlay (codec, hwdec, tracks, buffer) | Serial log lines only | Q5: I shows stats (codec, size, fps, late frames, sound buffer) |
| Zoom / aspect (Z on macOS) | Play fits, never scales up | Q5: Z cycles fit, fill, 1:1 (needs scaling in sex-media) |
| Poster LRU cache, dark-glass UI, Geist fonts | Geist already used by captions | Q6: library grid with poster thumbnails (first keyframe) |
| HTTP/HLS/DASH playback | Files only (Web streams separately) | Q7: URLs through sex_fetch with Range seek (reuse `Growing`), HLS playlists |
| Jellyfin multi-server: LAN discovery (UDP 7359), QuickConnect, direct play, progress reports, segments, Similar | None | Q8 |
| Stremio v3 addon protocol (catalog, meta, stream, subtitles), Continue Watching sync | None | Q9: addons the user adds by URL; nothing bundled |
| YouTube: SponsorBlock, DeArrow, ReturnYouTubeDislike | YouTube plays in Kaleidoscope (Web) | Q10, with the YouTube/browser thread |
| Torrent sliding-window streaming (header/footer pinned, deadline window ahead of the playhead) | None | Q11: report first; a `Source` fed by a confined BitTorrent client program |
| Swarm search across indexers (Nyaa, Torznab/Jackett/Prowlarr, others), seeders shown, results ranked | None | Q11b: indexer search built in. Torznab/Newznab and any indexer the user adds by URL; open catalogues (Internet Archive, Academic Torrents, distribution trackers) listed out of the box; results ranked by seeders and `episodes::similarity` |
| Multipass end-to-end encrypted sync of library and progress | None | Q12, later, on sex-crypt |
| Disposable Kodi Python addon helper | None | Not ported: Linux/Kodi-shaped and heavy. SexOS has Python as a program if wanted later |
| Twitch ad stripping, ad-free logged-in YouTube via InnerTube | None | Not ported: against those services' terms |

## How it fits SexOS

- Logic that brootlite keeps in `brootlite-core` lives in `crates/sex-media` as plain
  `no_std` modules with host tests (`episodes.rs`, `segments.rs`), so Qupid, the library
  and Kaleidoscope's `<video>` can share it.
- A program only reads files it was given (`@path` grants). Finding the next episode in
  a folder needs the folder granted, or the shell's open-with request (thread C) to hand
  the folder over; until then the playlist given on the command line is the list.
- Network sources are `sex_media::Source`s over `sex_fetch`, the same way Web streams.
  No kernel or POSIX additions: a Jellyfin or Stremio client is code inside Qupid (or a
  helper program it talks to over PDX), never a system service the kernel knows.
- Torrent streaming, when it comes, is its own confined program that serves byte ranges
  to Qupid through a lent region, as sexdrive serves blocks.

## Slices

- **Q1 (this PR): keep-watching logic.** `sex_media::episodes`: show/season/episode from
  names (`S01E02`, `1x02`, `Show - 07 [Group]`, `Episode 4`), release group, resolution,
  codec and flavour; `next_episode` picks the same show's next episode, specials
  skipped, the copy most like the one playing preferred (brootlite's scoring).
  `sex_media::segments`: skip segments from chapter names, `skip_at`, and the "Up next"
  card (at end credits or 30 s before the end, 10 s countdown, dismiss, re-arms on seek
  back). Host gate `scripts/qupid_host_gate.sh` in CI.
- **Q2: chapters and skip cards.** Bridge `sx_chapters` (AVChapter titles and starts),
  `Player::chapters()`; overlay card "Skip intro ⏎"; C lists chapters, PgUp/PgDn jump.
  Gate: a clip made with chapter metadata by host ffmpeg; Enter during "Opening" lands
  at its end (`[qupid.skip] kind=intro to_ms=`).
- **Q3: Up next.** When the card says Go, the next item plays: the playlist's next item,
  or `episodes::next_episode` over the folder listing when the folder is granted. Card
  shows "Up next · S01E04 · 7 s", Enter plays now, Esc watches the credits. Gate with
  three short episodes.
- **Q4: tracks.** Track list with language (ISO 639 names), codec, channels; picker
  overlay (A and S open it); embedded SRT/ASS/mov_text subtitle streams decoded into
  `subtitles::Track`; subtitle delay (Z/X) and sound delay.
- **Q5: stats and zoom.** I toggles a stats panel; Z cycles fit, fill and 1:1.
- **Q6: library.** With the rename thread: Music becomes Qupid's library. Continue
  watching row from the resume file, series grouped by `episodes::parse`, watched marks,
  the credits rail of unwatched items from the same show or folder.
- **Q7: URLs.** `run qupid https://...` streams with Range seek; HLS media playlists.
- **Q8: Jellyfin.** Add server (URL, user/password, QuickConnect), LAN discovery,
  libraries, direct play, progress reports, media segments into `segments`.
- **Q11b: indexer search.** A search box in Qupid's library that queries every indexer
  configured (Torznab/Newznab APIs, plus indexers added by URL), merges and dedupes by
  info-hash, ranks by seeders, size and `episodes::similarity`, and hands a result's
  magnet to Q11's client.
- **Q9-Q12** as in the table, each after a short report where it is large (Q11).

Order: Q2 and Q3 wait for #183 to merge (Play's main loop changes there) and for the
Play-to-Qupid rename; Q1 does not touch Play and goes now.
