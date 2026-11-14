# Lurk remote desktop (W6), 2026-11-14

Slice W6 of the app parity plan: **Lurk**, the remote desktop program. This first PR
covers the VNC (RFB) client. The VNC server half and RDP are described below as next
steps.

## Done

| Piece | Where | Test |
|---|---|---|
| RFB 3.8 protocol, `no_std` + alloc, no I/O (a state machine fed bytes, drained of bytes) | `crates/sex-rfb` | `cargo test -p sex-rfb` (19 tests, byte-exact goldens) |
| Handshake: versions 3.3/3.7/3.8, security None and VNC Authentication (DES in-crate, `des.rs`, checked against the FIPS example and an OpenSSL golden) | `client.rs`, `des.rs` | `handshake_*`, `des_*`, `vnc_response_*` |
| ServerInit, SetPixelFormat (RGB32, the window pixel layout), SetEncodings, incremental FramebufferUpdateRequest | `client.rs` | `handshake_none_38_bytes` |
| Decoders: Raw, CopyRect (overlap-safe), RRE, Hextile, ZRLE (one `miniz_oxide` inflate stream per connection), DesktopSize and Cursor pseudo-encodings, Bell, ServerCutText | `client.rs`, `zrle.rs` | `raw_rect` … `pseudo_encodings_bell_and_cut_text`; Hextile is fed one byte at a time |
| Client messages: KeyEvent, PointerEvent, ClientCutText; SexOS key → keysym table (Latin-1, Unicode `0x0100_0000+`, specials by PS/2 and normalized codes) | `client.rs`, `keysym.rs` | `client_messages_bytes`, `keysyms` |
| Server-side encoders for the later server half: version, security list/result, ServerInit, update and rect headers, Raw, ZRLE (tiles: solid / packed palette / raw; one zlib stream, sync flush per rect), and a parser for every client message | `server.rs` | `server_handshake_bytes`, `zrle_tile_bytes`, `zrle_server_to_client_roundtrip` |
| Host interop tool | `crates/sex-rfb/examples/snapshot.rs` | Ran against the gate's Python server: decoded framebuffer is byte-identical to the pattern (Raw + stdlib-zlib ZRLE), KeyEvent arrives |
| `lurk` program (sex-app window + `sex_app::net::TcpStream`, needs `run +net`) | `apps/lurk` | QEMU gate below |
| Gate: stdlib Python RFB server (quadrants + "LURK" band, Raw top half, ZRLE bottom half), QMP screendump colour/layout check, key `a` reaches the server as keysym 0x61 | `scripts/lurk_gate.py`, CI step after the ssh gate | **Not run locally** (no QEMU in the container); CI is its first run |

Usage: `run +net lurk HOST:PORT [PASSWORD]`. `HOST:N` with N < 100 means display N
(port 5900+N); `[v6addr]:port` works. The remote screen is fitted to the window with its
aspect kept (nearest neighbour, letterboxed) and redrawn on every update and resize.
Mouse positions are mapped back through the fit; wheel steps become buttons 4/5. Keys
are sent as press+release pairs (Shell delivers key-downs only); Control is wrapped
around the key when held. Log lines: `[lurk.start]`, `[lurk.tcp]`,
`[lurk.connected] name=… size=WxH version=3.N security=N`, `[lurk.frame] updates=N
rects=N area=…` (first five, then every 100th), `[lurk.shown]`, `[lurk.key]`,
`[lurk.failed]`, `[lurk.closed]`.

## Stubbed or missing in the client

- Clipboard: ServerCutText is decoded and logged (`[lurk.cut]`) but not published to
  the Silk clipboard; ClientCutText exists in `sex-rfb` but Lurk never sends it. Next:
  wire `sex_pdx::clipboard_client` both ways (Latin-1 only, as RFB 3.8 requires).
- The remote cursor shape (Cursor pseudo-encoding) is decoded into `Client::cursor` but
  not drawn; the local SexOS pointer is what the person sees.
- Whole-window redraw on each update (no damage-limited redraw or smooth scaling). Fine
  for 1024x768 under KVM; slow under TCG.
- No Tight, TRLE, ExtendedDesktopSize, QEMU extended key events, or VeNCrypt/TLS. VNC
  Authentication is DES and weak: use it on trusted networks or over `ssh -L` until
  VeNCrypt (via `sex-tls`) lands.
- Key-up timing is synthetic (Shell sends only key-downs), so held keys don't repeat
  remotely the way the remote OS would repeat them.
- No connection dialog or saved hosts: the address comes from the command line.

## Next: the VNC server half

The encoders in `sex_rfb::server` already produce everything a server sends. What is
missing is authority: a program must never see the screen or inject input by default.
Proposed grant design (no change to `servers/silk-shell/src/main.rs` in this PR; it
would be a module `servers/silk-shell/src/remote.rs` plus a hook under ~10 lines, in a PR
of workstream C's or with them):

1. **Two new run grants**, alongside `+net`: `+screen` (read the composed desktop) and
   `+input` (inject keys and pointer). `run +net +screen +input lurk serve 5900`.
   Collar asks the person the first time, like the first-run grant question in
   sexshop, and shows a persistent "screen is being shared" chip in silkbar (tray,
   OP_TRAY) while either grant is live; clicking it revokes both.
2. **Screen capture** goes through Display, not the kernel: a new `sex_pdx` protocol
   module (`screen_share.rs`) where Display copies damaged regions of the composed frame
   into the program's own pixel region (the same lending used by `pixel_region`) and
   reports the damage rectangles. Display already composes for the screenshot UI (C8,
   OP_SCREENSHOT_*); this reuses that path, rate-limited, and only while Shell says the
   grant is live. Lurk turns damage into ZRLE rects with `ZrleEncoder`.
3. **Input injection** goes through Shell: the program sends RFB-shaped key/pointer
   events to Shell over PDX; Shell treats them like Input's events (focus, shortcuts,
   capture) but marks them injected, refuses them while the lock screen (B6) is up
   unless the grant explicitly says "unattended", and never lets an injected event
   reach Collar's grant dialogs (so a remote viewer cannot approve its own grants).
   Keysyms map back to key codes through `sex-keymap`'s active layout.
4. **Revocation** is the ordinary capability revocation: closing the chip or ending the
   run drops both, and Display stops copying at once.

Gate for the server half: a host Python RFB client (stdlib, the inverse of the one in
`lurk_gate.py`) connects to `lurk serve` through QEMU `hostfwd`, checks the framebuffer
against a known window, sends a key and sees it typed into Jot.

## Later: RDP client

RDP via a pure-Rust stack, e.g. **IronRDP** (Apache-2.0/MIT, Devolutions): its
`ironrdp-connector`/`ironrdp-session` crates are sans-I/O like `sex-rfb`, so Lurk would
drive them over the same `sex_app::net::TcpStream` and `sex-tls` for the TLS/CredSSP
layer. Work: check `no_std`/alloc or build Lurk on the `x86_64-sex-std` target (as `ssh`
does) if IronRDP needs `std`; pin versions in Cargo.lock; bitmap codecs (RemoteFX,
planar, interleaved) map into the same window pixel path. Gate: an `xrdp` or FreeRDP
`freerdp-shadow-cli` server on the CI host.
