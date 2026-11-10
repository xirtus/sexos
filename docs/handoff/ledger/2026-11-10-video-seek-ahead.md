## Seeking ahead in a video still downloading (2026-11-10)
- `sex_media::Growing` holds pieces: the one coming in (`start`, `bytes`) and the ones
  before it (`kept`). When the server takes Range requests (`Accept-Ranges: bytes` or a
  206), a read more than 1 MiB past the coming piece, before it, or into a gap sets
  `want`. `Growing::begin` keeps what came (dropping the pieces furthest away over
  `MEDIA_MAX`) and starts a new piece there.
- Web (`media_downloads`) sees `want` once the video is open, lets the download go and
  asks again with `Range: bytes=WANT-` (`[web.media.range] node= from=`). A reply that is
  not a 206 from that byte turns pieces off (`[web.media.range.refused]`); the stream
  then ends where it got. A finished piece with gaps elsewhere stays as an idle load
  until the next seek.
- Decoder: a read whose bytes are not here yet (seekable stream) no longer ends the
  video: `sx_next` answers 3, the player waits until the bytes come, then repeats the
  last seek (the MP4 demuxer lost its place on the failed read). Media Source feeds keep
  their old behaviour.
- Gate: `scripts/program_web_seek_gate.py` (page `web_site/seek.html`): 100 KB/s site,
  the page jumps to 8 s a second after playing; the piece must be asked for and 8 s
  must show within 6 s, before the whole download. The stream gate's site now takes
  Range requests too.
