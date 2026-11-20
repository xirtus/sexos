# SexOS voice job contract

`sex-voice` is a small `no_std` frame codec shared by future native clients and
workers. It does not provide transport, authority or inference. A job ID and
asset ID identify data; a caller still needs an explicit service and asset grant.

The frame limit is 4096 bytes. Version 1 carries Submit, Cancel, GetStatus,
Status and Result messages. Result metadata includes WAV sample rate, channel
count, frame count, SHA-256 of the WAV bytes, seed and actual model name. A
client retrieves the immutable WAV asset separately.

## Fixture

```sh
cargo test --manifest-path crates/sex-voice/Cargo.toml
cargo run --manifest-path crates/sex-voice/Cargo.toml --example fixture_worker
python3 -m unittest discover -s crates/sex-voice/adapters -p 'test_*.py'
python3 crates/sex-voice/adapters/mosslanding.py --fixture
```

The Python adapter exposes `POST /v1/frame` and `GET /v1/assets/<id>` on
`127.0.0.1:8875`. Its fixture mode needs only Python's standard library.
Non-loopback binding requires `MOSS_VOICE_TOKEN`, passed as a bearer token.
For use across a LAN, put this HTTP bridge behind an authenticated TLS tunnel;
the bearer token alone does not encrypt audio or credentials.

## Existing MOSSlanding backend bridge

Run MOSSlanding's macOS backend normally on loopback port 8765, then run the
adapter without `--fixture`. It calls MOSSlanding's `/api/synthesize` and
`/api/status` endpoints, saves the returned WAV in memory, and returns metadata
through the versioned frame protocol. `MOSS_VOICE_MAP` maps numeric IDs to
reference voice filenames, for example `{"8":"my-voice.wav"}`. Filenames must be
bare names already present in MOSSlanding's voice library.

The existing HTTP backend does not accept a per-job seed or model selection,
and its `continue` mode is not implemented distinctly from clone. The adapter
rejects those requests instead of pretending to honor them. It accepts
`model="active"` and `seed=0` for real jobs. Cancellation stops publication of
the result, but the current MOSSlanding inference call cannot be interrupted.
The adapter's asset store is in memory; a production SexOS bridge must persist
WAV assets through SexFiles and bind access to the requesting capability.
