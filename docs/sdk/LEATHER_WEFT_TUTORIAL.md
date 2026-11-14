# Leather: build your first SexOS window

Leather is the SexOS app SDK. Its UI toolkit is Weft, introduced in PR #220. These
examples build against that dependency; they do not add another widget system.
Code is GPL-3.0-only. The program is a capability-holding organ in one address
space: it has no ambient Unix filesystem, sockets or process namespace.

## Start with the hello template

`sdk/leather/hello` is a complete independent Cargo package. From the repository
root, build it with the same position-independent target used by other programs:

```sh
RUSTFLAGS='-C relocation-model=pic -C link-arg=-pie' cargo build --release \
  -Z build-std=core,compiler_builtins,alloc \
  -Z build-std-features=compiler-builtins-mem \
  --target x86_64-sex-app.json --manifest-path sdk/leather/hello/Cargo.toml
```

For an installed app, copy it to `apps/YOUR_THEMED_NAME`, remove the template's
`[workspace]` section, shorten dependency paths from `../../../crates` to
`../../crates`, rename the package and add it to the root workspace members. The
installed name must fit the program store's 16-byte limit. Follow the fabric,
romance, psychedelic or pun naming rule in `docs/DESIGN_GUARDRAILS.md`.

Keep `build.rs`: layout-seeded relinking lets the existing build stage avoid
forbidden WRPKRU/XRSTOR byte sequences. Add your manifest to `[allowed].crates`
and a `cargo_manifest` stage to `sexos_build_spec.toml`, before `boot_image`.
Use the `build_leather_hello` stage as the exact pattern. The normal build scans
executable ranges with `scripts/key_bytes.py` and retries layouts when needed.

The template calls `App::open`, makes a label and button, then consumes
`Response::Clicked` and `Response::Close` from `app.next()`. Weft owns painting,
layout, resizing, focus, keyboard activation and accessibility. Setting widget
text marks damage; the next event-loop turn paints it. The program never draws its
own buttons. A one-MiB app stack is requested with `sex_app::with_stack`.

## Three runnable examples

`apps/leather-samples` is a copyable package with four binaries. Build it with the
same command above, replacing the manifest with
`apps/leather-samples/Cargo.toml`. It is in the normal image build.

| Program | Learn from it |
| --- | --- |
| `run leather-hello` | Retained Weft UI, button response, state and close |
| `run leather-stitch` | UTF-8 multiline document model, line editing, Files read/write/sync |
| `run leather-lantern` | Bounded image decode, fitting pixels into a Weft image widget |
| `run +net leather-postcard` | User-granted HTTP(S) request, bounded reply, displayed errors |

Copy the package to preserve the small separation: `src/lib.rs` contains portable
state and UI; `src/program.rs` binds actions to native file/network operations;
`src/bin` chooses a sample. Host tests drive the same widgets used by the guest.

### Stitch: a small text editor

Edit the current line in Weft's text field. Add Line inserts an empty line after
it; Previous/Next select another line without losing edits. The model preserves
Unicode and blank lines and is limited to 64 KiB. Load accepts UTF-8 text and
normalizes CRLF to LF. Save replaces the file named above the editor and reports
success only after `files::sync` succeeds. This is a line-oriented teaching
editor, not Quil or a multiline custom widget. It has no autosave or close prompt.
The ordinary Weft field provides keyboard selection and local undo.

The default `draft.txt` lives in this program's private Files folder. A file path
from a Shell launch may instead name an existing held document route (`@...`).
Do not turn a path into a kernel address or invent IPC to another app. To open a
file in another application, use the desktop Shell launch/open request.

### Lantern: pictures remain inside the program

The initial swatch image is original, generated for this example. Enter a local
filename or held document path and choose Open Picture. `sex-image` decodes inside
the app's confinement, with a one-MiB input limit, four-million-pixel limit and
4096-pixel side limits, then fits into 480×280. Supported formats are those of
`sex-image` (PNG/JPEG/WebP/GIF; first image only here). A hostile image never runs
inside Display or the kernel. Weft receives decoded pixels through `set_image`.

### Postcard: ask for the network

Start it with `run +net leather-postcard`, enter an HTTP(S) URL and choose Fetch.
Without the grant the existing network API returns a useful refusal. TLS uses the
existing trust path. The reply is limited to 16 KiB and shown as lossy UTF-8 text,
including its HTTP status; this is not a browser. Fetch uses a synchronous bounded
API, so a request can occupy the event loop until it completes or times out.
No network authority is added to the kernel or to another program.

## Tests and guest evidence

```sh
cargo test --locked -p leather-samples --no-default-features
python3 scripts/leather_gate.py --iso sexos-v1.0.0.iso \
  --source-disk BLANK.img --out OUT
```

Host tests check model bounds, Unicode, line insertion, real UI actions and
painting. The QEMU gate clicks Greet, edits two lines, saves/syncs and reloads their
exact hash, decodes the original picture, and fetches an exact local HTTP fixture
through `+net`. It retains a screenshot for each window and closes every run.
A guest compile and a host UI test alone do not establish guest playback or UI
behavior; only report the QEMU result after the gate runs successfully.
