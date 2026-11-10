# Optional lossless recording encoder

This slice adds an FFV1/BGR0 Matroska encoder inside the calling program's
existing PKU domain. It adds no capture grant, framebuffer access, kernel code,
networking, audio capture, or desktop recording UI. Spindle has no allocator and
must not acquire this encoder implicitly. A recorder needs an explicitly bounded
heap and narrowly held capture/file routes.

Build `scripts/build_ffmpeg.sh` with `FFMPEG_RECORDING=1` to enable FFV1
encode/decode and Matroska muxing. Default builds retain the decoder profile and
return `recording::Error::Unavailable`. A profile marker and separate CI cache
key prevent the decoder archive being mistaken for an encoder build. The new CI
job is included in the aggregate `build` requirement.

`Encoder` accepts owned private XRGB pictures and increasing millisecond PTS.
It copies into an AVFrame, preserves input timestamp gaps, drains delayed packets
at finish and emits ordered offset writes, including Matroska header patches.
C callbacks only enqueue bytes or move the prospective output cursor; they never
perform Files IPC. Before accepting another frame or finish the owner drains all
output, authenticates each actual Files reply and acknowledges its exact local
token. Each advertised turn is at most 32 KiB; partial acknowledgements rotate
tokens. Queue limits: 32 MiB payload, 4,096 chunks, 4 GiB file, 4,194,304 pixels,
36,000 frames and PTS no greater than 600,000 ms. Codec/frame state is additional
memory and remains bounded by the program row's heap, not the queue limit.
Allocation or codec failures poison the encoder; dropping it performs no IPC or
trailer writes. Durable sync remains the file owner's responsibility.

`run play --encode-proof proof.mkv` is an explicitly synthetic diagnostic in
Play's existing private Files folder. It atomically reserves the final name
with exclusive CREATE, writes four 65x49 pictures at PTS 0/200/600/800 ms, mutates
each submitted input buffer, drains/finishes and syncs. Partial files remain named
on failure/cancellation. There is no replacement rename or atomic-save claim. It is not a desktop recording.
The guest gate recovers `/data/play/proof.mkv` read-only after stopping QEMU,
then independently verifies codec, dimensions, every RGB byte and packet PTS.

Validation:

- Three production output-queue host tests pass: ordered backward patches/holes,
  32 KiB turns, partial/replayed acknowledgements and failure without mutation.
- Actual Rust/C encoder host smoke passes all four pictures byte for byte and
  packet timestamps. Boundary: host FFmpeg 8.1.1 and host libc, not guest 7.1.2.
  Evidence: `sexos-evidence/recording-encoder-host-gate/result.json` outside Git.
- First host CLI extraction failed because default constant-rate conversion
  duplicated one picture across the deliberate timestamp gap. This was diagnosed
  by comparing all five decoded frames and packet PTS; extraction now explicitly
  uses passthrough timing. No encoder assertion was weakened.
- SexOS target check for Play and sex-media passed with encoding unavailable.
  A native ARM host cargo check fails on the repository's x86 syscall registers;
  it is not a supported target check and is not reported as passing.
- Private FFmpeg 7.1.2 recording profile built with Clang 23.1.5, musl headers,
  FFV1 encoder/decoder and Matroska muxer enabled. Existing forbidden-instruction
  scanning passed, with the existing sha.o frame-pointer recompilation.
- Native guest round trip passed on runtime commit `d14168f0`, clean worktree.
  ISO SHA-256 `a31aa42df848c3eb928924408023050d0c8bc93aba286a3b78792e2ab2bc351f`;
  Play `212f2173e4d015076ce8aef8ba658699ff80c216a432cbe476156411492c261d`.
  Guest wrote and synced 10,503 bytes; independent host decode matched all
  38,220 RGB bytes and all four packet PTS/durations. Evidence:
  `sexos-evidence/recording-encoder-guest-d141/result.json`, with the disk, raw
  serial and recovered Matroska preserved outside Git; build/archive hashes in
  `sexos-evidence/recording-encoder-inputs/provenance.json`. The guest was stopped
  and reaped by the harness after sync; no orderly OS-shutdown claim.
- Subsequent review fixed consumed Close leaving the diagnostic window alive:
  between file turns it now removes the temporary best-effort and finishes the
  run. The corrected Play was rebuilt with `scripts/build_play.sh` at clean
  commit `436d6cad` and rerun on the same pinned ISO; Play SHA-256
  `ca78f2ef81e59aee340778c4ce8b437a7fcf83623791b62893e184abece79bd8`.
  The recovered file and all pixels/timestamps matched again; evidence:
  `sexos-evidence/recording-encoder-guest-436/result.json`. This rerun exercises
  successful completion, not an injected mid-write cancellation. Final candidate
  CI remains required. No recording performance, desktop capture integration,
  physical hardware or daily-driver claim yet.

Review follow-up (after both native proofs above):

- Removed the diagnostic's stale STAT → replacement RENAME race. Exclusive final
  CREATE now fences concurrent creation; failed/closed runs retain their named
  partial instead of removing paths that could have changed. Existing output is
  never opened for writing when CREATE fails. The historical proof runtimes used
  the old temporary+rename policy and are not claimed to cover this change.
- Final codec draining must reach EOF before a trailer is accepted; EAGAIN after
  a flush is an error. Seek callback errors are sticky because FFmpeg's AVIO seek
  path and some Matroska trailer calls can ignore the callback's negative return.
  A real-encoder host test injects a failing seek and a successful C-operation
  result, requiring terminal failure across output, finish and frame calls.
- Host round trip still passes after these review changes. Current guest round
  trip/CI must pass before this draft can merge.

- Review-corrected native Play at clean `f82ccbe5` passed the complete round trip
  on the same pinned ISO, using Play SHA-256
  `377a3d2d409acaa24e19e44d23f1239f4bb7d600aee566f8b46d2f27c2fdddfc`.
  Exact pixels, timestamps and recovered Matroska bytes match both earlier runs;
  evidence: `sexos-evidence/recording-encoder-guest-f82/result.json`.
- Required CI also executes the production Rust callback test against an explicitly
  scripted C ABI and rejects a compiled mutant that drops sticky seek failure.
  This boundary test complements, and does not replace, actual guest encoding.
