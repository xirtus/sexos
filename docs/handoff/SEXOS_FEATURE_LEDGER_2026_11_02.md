# SexOS feature ledger (from 2026-11-02)

## Handoff summary (as of 2026-11-03, after F28)

**Try it (native image).** Browser: `W` HTML page as text, `1`-`9` follow
links, `Backspace` back, `G` QOI image, `P` PNG image, `B` plain document.
Quil: autosave draft after ~30 s idle (`DRAFT KEPT`), `Ctrl+D` twice deletes
the saved file. Spindle: Shift works; `calc 2*(3+4)/5`, `calc ans*10`,
`calc 2**10`, `calc sqrt(2)`.

**Inline script subset** (`apps/kaleidoscope/src/script.rs`, not a JS
engine): variables, numbers printed like JavaScript, strings and string
methods, booleans, comparisons, `if/else`, `while`/`for` with
`break`/`continue` under a 10,000-step budget, functions (declared before
use, recursion to nesting 32), arrays, objects, templates, `Math`, `JSON.stringify`,
`Object.keys`, `String/Number/parseInt`, output via `document.write`. No DOM,
events, timers, network, storage, clock or entropy names exist in it.

**Native gates added** (all pass at 1280x800 and 1024x768 on the latest
image): `https_html_gate.py`, `https_png_gate.py`, `quil_autosave_gate.py`,
`quil_delete_gate.py`, `spindle_calc_gate.py`. Host gates added:
`quil_autosave_host_gate.sh`, `spindle_calc_host_gate.sh`; script, html_text
and PNG tests run in `sexnet_jobs_host_gate.sh`.

**Open release gates** (details below): full-window reader needs a larger
Display text delivery; Display applies app paints late (screenshots must wait
for the app's `applied` receipt) and Spindle's display stayed pending in the
calc gate; Browser stack headroom is now measured (F29: 46.7 KiB of 64 KiB);
script recursion is capped at nesting 32 (about `fact(7)`); autosave failure
fences document I/O like any uncertain save; no draft cleanup; images are
limited to 48x48 (any standard PNG depth/interlace).

**Next suggested steps.** A DOM-free event model is the next JavaScript step
only if it fits held authority (e.g. a key handler the Browser calls, no
ambient names); measuring the Browser stack high-water mark first; then
larger images once Display delivery grows.


User-noticeable daily-driver features, logged here so they don't collide with
the round numbering in `SEXOS_DEVELOPMENT_ROUNDS_109_PLUS_2026_10_31.md`.

## F1. Browser reads HTML pages as text (2026-11-02)

**What changed.** The Browser (Kaleidoscope) can fetch a fixed `text/html`
page from the BOOT-controlled HTTPS endpoint and show it as readable text in
the existing reader. Press **W** in the Browser. Headings, paragraphs, line
breaks, lists, simple tables, character entities and links are rendered; links
keep their text with a numbered reference and the targets are listed at the
end. `script`, `style`, `head`, `template`, `noscript`, `svg` and comments
produce nothing. Output is bounded by the 12 KiB document limit and a cut is
marked `[page truncated]`.

**SexOS rules kept.** Selecting the page is fixed resource data on the same
BOOT endpoint, trust profile and held route (`sex_http::Resource::PageHtml`,
`/page.html`, `Accept: text/html`); it selects no host, port, route or
capability. No script runs. The converter (`apps/kaleidoscope/src/html_text.rs`)
is application presentation code, handed to the exchange as a plain function;
`tls_exchange` holds no HTML policy. `sex_tls::Document::rewrite` replaces the
ready body with its text form and erases the source bytes. No ABI input
changed.

**Evidence** (`docs/handoff/assets/browser-html-reader-2026-11-02/`):
- Host: `tests/browser_html_text.rs` (9 tests, in `sexnet_jobs_host_gate.sh`),
  `sex-http` codec (40, incl. text/html MIME acceptance and refusal),
  `sex-tls` crate tests (all suites green, incl. `Document::rewrite`), Python
  suite 1132 OK.
- Sealed build `make full` green; master runtime gate GREEN (the first attempt
  was YELLOW from a split SilkBar civil serial line, unrelated; rerun green).
- Native `scripts/https_html_gate.py` at 1280x800/e1000e and 1024x768/e1000:
  exact decrypted request, `[browser.html.converted] source_bytes=672
  text_bytes=342 links=2 truncated=0`, released body hash equals the converted
  text, and reader layout plus glyph pixels match the expected text.
- Regression: `https_document_gate.py` (`/welcome`, ready) still passes.

**Limits.** One fixed page path; no navigation to links yet, no CSS, images,
forms or JavaScript. The reader uses its existing fixed-width pane. The
expected text in the native gate comes from compiling the same converter on the
host; the converter's behaviour is pinned independently by the host tests.

**Next.** Follow links within the controlled endpoint (bounded path set),
wider reader pane, then a bounded script interpreter design that fits held
authority (no ambient DOM/network).

## F2. Browser follows page links (2026-11-02)

**What changed.** While an HTML page is open in the reader, number keys
**1–9** follow the matching link `[n]`. A plain absolute path (`/guide`) is
fetched from the same BOOT-controlled HTTPS endpoint as `text/html` and opened
in the reader, whose header shows the new path. Anything else (another host,
a relative or query URL, a cut link list) is refused locally with
`[browser.html.link.refused]` and causes no DNS or TLS work.

**SexOS rules kept.** The path is request data validated by
`sex_http::Path::from_location` (no `..`, `//`, query, host or port). It goes
through `sex_http::request_https_page` and `sex_tls::Session::page` on the
same held route, profile and expected name; it creates no endpoint or
authority. Link ranges come from the converter's own output
(`Converted::targets`), not from re-parsing page text, so page content cannot
forge a target. No ABI input changed.

**Evidence** (`docs/handoff/assets/browser-html-links-2026-11-02/`): host
html_text 11 tests, sex-http 41, sex-tls suites green, sexnet host gate and
Python suite OK; sealed build and master gate GREEN; native
`https_html_gate.py` at 1280x800/e1000e and 1024x768/e1000: page pixels,
link 3 (`https://elsewhere.example/x`) refused with no network work, link 1
followed with the exact second request `GET /guide`, guide text pixels match;
`/welcome` document gate regression passes.

**Limits.** Only links 1–9; no back/history key yet; fixed endpoint only.

## F3. Quil autosave drafts (2026-11-02)

**What changed.** When a Quil buffer has unsaved edits and nothing is typed
for about 30 seconds, Quil saves a recovery copy into a separate object named
`<title>.draft` (for a new document, `UNTITLED.draft`). Later idle periods
overwrite the same draft object. The user's own document is never written and
stays marked unsaved; the close guard still warns. After a crash or power cut,
the draft is listed in Linen and opens in Quil like any document. An explicit
Save pressed while a draft save is in flight is queued and runs right after.

**SexOS rules kept.** The draft goes through Quil's existing held document
route to sexfiles (`create_new`, then save to the returned object); no new
grant, slot, opcode or ABI input. Names follow the storage rules
(`[A-Za-z0-9._-]`, 16 bytes); a taken name moves to `<title>NN.draft`. While a
draft is pending its idle period, Quil waits with the bounded
`sys_wait_self_ticks(64)` instead of blocking forever, so the countdown can
fire without input; otherwise the loop is unchanged.

**Evidence** (`docs/handoff/assets/quil-autosave-2026-11-02/`): host
`scripts/quil_autosave_host_gate.sh` (4 tests); sealed build and master gate
GREEN; native `scripts/quil_autosave_gate.py` at 1280x800 and 1024x768: two
drafts into the same object (revisions 9 and 17), document never saved and
still dirty, QEMU killed (no shutdown), second boot opens `UNTITLED.draft`
from Linen and loads 17 bytes with the drafted hash; the recovered text is on
screen. Measured idle-to-draft was 37.7 s from the host side. Regressions:
`quil_workflow_gate.py` and `connected_desktop_gate.py` pass on the same build.

**Found on the way.** A first naming choice (`~title`) was rejected by
sexfiles as INVALID, and a failed create during a save leaves Quil's document
I/O fenced (Uncertain) until it is reopened. That is the existing
uncertain-save rule, but it means a storage fault during an autosave blocks
the next explicit save too; worth a recovery path later. Also, after a
Linen reopen, Display applied the new Quil view a few seconds late.

**Limits.** No draft cleanup (sexfiles has DELETE, but Quil holds no delete
route); drafts are not offered automatically on startup; one draft per
document source per session.

## F4. Browser Back key and preformatted text (2026-11-02)

**What changed.** In an HTML page, **Backspace** returns to the page the
current one was followed from (up to 8 levels, fetched again from the same
endpoint); at the first page it does nothing and makes no network request.
`<pre>` blocks keep their spacing and line breaks (tabs become spaces).

**Found on the way.** The first build kept the 8-entry history inline in the
Browser state. The Browser crashed while preparing TLS: its stack grew into a
demand-paged page and the pager rejected it because its 4-page quota was
already spent on heap pages (`sext.fault.reject ... reason=Policy used=4
quota=4`). The history and link table now live on the Browser heap. The
Browser's stack headroom is not measured anywhere; see the release gate below.

**Evidence** (`docs/handoff/assets/browser-html-back-2026-11-02/`): host
html_text 13 tests and sexnet host gate OK; sealed build and master gate
GREEN; native `https_html_gate.py` at both sizes: page, refused off-endpoint
link, follow `/guide` (with a `<pre>` table checked in glyph pixels), Back to
`/page.html` as an exact third request with pixels, Back at depth 0 makes no
request; `/welcome` regression passes.

## F5. Build-time Browser stack budget (2026-11-02)

**What changed.** `apps/kaleidoscope/src/main.rs` now fails to compile when
the Browser `App` state grows past 19,000 bytes, with a message saying to box
the new state. Measured size today: 18,872 bytes (probe with a zero-length
array const against the sealed target and rustflags). The F4 crash happened at
about 19,400 bytes. The assert was checked both ways: green at 19,000, and a
compile error at a temporary 18,000 budget.

**Evidence.** Browser host gates (asset fence, calendar, reader visibility,
text, wheel) OK; sealed `make full` green. No runtime behaviour changed.

**Limit.** This bounds one big stack object, not total stack depth. A real
fix is a stack high-water measurement or a stack quota separate from heap.

## F6. Quil shows "DRAFT KEPT"; HTML images show alt text (2026-11-02)

**What changed.** When Quil has unsaved edits that are already held in the
autosave draft, its status line reads `DRAFT KEPT` instead of `READY`, so the
user can see the work is recoverable; the dirty `*` stays. In the Browser,
`<img>` becomes `[image: alt text]` (or `[image]`); images are never fetched.

**Evidence** (`docs/handoff/assets/quil-draft-status-html-images-2026-11-02/`):
autosave host tests (now incl. `kept`), html_text 14 tests; sealed build and
master gate GREEN; native autosave gate at both sizes now also waits for
`status=UNTITLED * DRAFT KEPT` and captures it after Display applies the
paint; HTML gate page includes an image and its text/pixels match;
`quil_workflow_gate.py` regression passes.

**Found on the way.** Display applies Quil paints a few seconds after Quil
admits them in these runs, so screenshots must wait for
`[quil.ui.applied]` of the latest paint (the gate now does).

## Host-gate repair after F3 (2026-11-02)

F3 broke two host gates I had not run: `quil_clipboard_host_gate.sh` (its
extracted Quil functions now reference `QUIL_AUTOSAVE`) and
`service_custody_host_gate.sh` (it required the first
`DOCUMENT_WORKSPACE.changed()` to follow the Saved branch; autosave's own
refresh comes earlier). The clipboard harness now includes the real
`autosave.rs`; the custody check now requires the Saved branch itself to
refresh the workspace. Both gates pass again.

## F7. Quil Ctrl+D deletes a saved document (2026-11-02)

**What changed.** In Quil, Ctrl+D on a saved document asks
`DELETE FILE? CTRL D AGAIN`; a second Ctrl+D on the same unchanged document
deletes its object. The text stays in Quil as an unsaved `UNTITLED` buffer
(`DELETED / TEXT KEPT UNSAVED`), so nothing typed is lost by accident. Linen
stops listing it, also after a power cut.

**SexOS rules kept.** New request kind `sex_pdx::document::DELETE` (ABI
input; Andreas approved the change in-thread, `abi_version_hash`
recomputed). The object is named by its current generation on Quil's
existing held document route; sexfiles refuses object 2, system slots, a
bound session or a stale generation, and its delete retires the generation
so every other holder goes stale. A refusal is a definite failure, not an
uncertain one. No new grant, slot or route.

**Evidence** (`docs/handoff/assets/quil-delete-2026-11-02/`): host
`document_load_backend` delete mapping test and service custody / Quil
clipboard host gates; sealed build and master gate GREEN; native
`scripts/quil_delete_gate.py` at both sizes: save `gone`, Linen lists it,
first Ctrl+D asks and deletes nothing, second Ctrl+D gives
`[files.document.deleted] target=9 object=3`, Linen refresh drops `gone`,
reboot after QEMU kill still without it; screenshots of both prompts.
Regressions: autosave gate and `quil_workflow_gate.py` pass.

**Limits.** Deletes only the document open in Quil; no Linen-side delete,
no undo/trash.

## F8. HTML page title, quotes and rules (2026-11-02)

**What changed.** The page `<title>` (entities decoded) opens the text as
`Title: ...`; `<blockquote>` lines start with `> ` per nesting level (up to
4); `<hr>` becomes `----`.

**Evidence** (`docs/handoff/assets/browser-html-structure-2026-11-02/`):
html_text 15 host tests; sealed build and master gate GREEN; native
`https_html_gate.py` at both sizes with a page containing title, quote, rule
and image (reader text/pixels, link refusal, follow, Back all pass).

## F9. Spindle Shift key and `calc` (2026-11-02)

**What changed.** Spindle had no Shift: capitals and symbols such as
`* ( ) + % : " ?` could not be typed (its help even said "no shift"). The
Shell already tracks modifiers; it now sends Spindle the held Shift in bit 8
of the key word (the same bit as `app_key::Key::wire`), and Spindle applies a
US-QWERTY Shift layer in insert mode. New command `calc <expr>`: exact
decimal arithmetic (`+ - * / %`, unary minus, parentheses, 9 decimal
places, i128). `0.1+0.2` is `= 0.3`; an inexact division says `~`; overflow,
division by zero, bad syntax and nesting deeper than 16 are named errors.
This parser is meant to be reused as the numeric core of a later bounded
script evaluator.

**SexOS rules kept.** Shift is presentation state from the Shell's own
modifier tracking, sent on the existing Spindle input route; no new route,
grant or ABI input. `calc` uses no clock, storage or network.

**Evidence** (`docs/handoff/assets/spindle-calc-shift-2026-11-02/`): host
`spindle_calc_host_gate.sh` (2 test groups, 20 cases), Spindle editor host
test for the Shift layer (via `service_custody_host_gate.sh`); sealed build
and master gate GREEN; native `scripts/spindle_calc_gate.py` at both sizes
types `calc 2*(3+4)/5`, `calc 0.1+0.2`, `calc 1/3`, `calc 7%0` through real
keyboard events with Shift and checks each result record;
`connected_desktop_gate.py` regression passes.

**Open.** The end-of-run screenshot shows Spindle's pane without the new
output: Spindle admitted its display batches early in the run but no later
paint was applied before capture. Together with the Quil paint lag seen in
F3/F6, this points at Display applying app paints late under these gates.

## F10. Bounded inline script subset in the Browser (2026-11-02)

**What changed.** First step toward scripting. Inline `<script>` blocks in an
HTML page run through `apps/kaleidoscope/src/script.rs`, a deliberately
small subset, not a JavaScript engine: `var/let/const` declarations,
assignment to declared names, `document.write` / `document.writeln`, number
and string literals (with escapes), `+ - * / %`, unary `+ -`, parentheses,
`NaN`/`Infinity`, comments and newline-terminated statements. Numbers are
IEEE doubles printed as JavaScript prints them (`0.1+0.2` gives
`0.30000000000000004`, `1/0` gives `Infinity`, `1e21` gives `1e+21`);
`+` with a string concatenates. Written output is inserted as text, never as
markup. Anything else (loops, functions, objects, `fetch`, `eval`, page or
window access, template strings, external `src`, other `type`s) means the
script writes nothing; refusals are counted in `[browser.html.converted]
... scripts_run=N scripts_refused=M`.

**SexOS rules kept.** The subset has no names for anything outside its own
16 variables: no DOM, timers, storage, network, clock or capability. It is
straight-line, so it always terminates; nesting is capped at 8 levels (the
Browser stack is tight, F4/F5), source at 4 KiB, string arena at 2 KiB and
output at 1 KiB, both allocated fallibly on the heap. Conversion of the test
page takes 4 ticks.

**Evidence** (`docs/handoff/assets/browser-script-subset-2026-11-02/`):
`tests/browser_script.rs` (4 test groups: writes and variables, 19 number
formatting cases, 13 refused programs, bounds) and html_text 15 tests, both
in `sexnet_jobs_host_gate.sh`; Browser host gates OK; sealed build and master
gate GREEN; native `https_html_gate.py` at both sizes: the page's script
writes `Script says 42 / -6`, a `while` script in `<head>`, a `fetch` script
and a `src` script write nothing (`scripts_run=1 scripts_refused=2`), the
released body hash equals the expected text, plus refusal/follow/Back;
`/welcome` regression passes.

**Found on the way: guest HTTPS close fails for some response sizes.** With
the page at 992 bytes (response about 1.1 KB: one full 1024-byte TLS record
and one short one) the guest received, parsed and converted the body, then
its net job went `Receiving -> Failed` at close instead of `Complete`, and
the Browser showed Unavailable. Reproduced three times at both sizes, with
and without 1024-byte chunking on the server. An 841-byte page and a
3.8 KB page complete normally. The gate page is now padded to 3.8 KB with a
comment so this feature is tested; the failing case is kept as an excerpt in
`close-failure-1.1kb/`. Not yet diagnosed (sexnet TCP close / TLS
close_notify around a short final record).

## F11. Fix: HTTPS fetch failed at close for some response sizes (2026-11-02)

**Cause.** In sexnet's duplex job, `advance()` failed the job whenever the TCP
link read `Closed`, before the duplex path ran. After the guest's FIN was
acknowledged (CloseWait -> LastAck -> Closed), a TLS client that had not yet
read the last received bytes (for example the close_notify behind a short
final record) lost the whole exchange. Timing decided it, so it showed up
for a ~1.1 KB response and not for 0.8 KB or 3.8 KB ones (F10).

**Fix.** The duplex job records that the peer's FIN was seen. `Closed` after
our own FIN and the peer's FIN is a graceful end: nothing more is received,
bytes already in the ring stay readable, and the job completes once the
client has consumed them. `Closed` any other way (reset, or our FIN without
a peer FIN) still fails. Legacy single-request jobs are unchanged.

**Evidence** (`docs/handoff/assets/https-graceful-close-2026-11-02/`): two new
sexnet job host tests (graceful close keeps bytes; Closed without both FINs
fails), full `sexnet_jobs_host_gate.sh`; sealed build and master gate GREEN;
native `https_html_gate.py` at both sizes now with the formerly failing
992-byte page (padding removed): `ticket=2 ... to=Complete`; regressions
`https_document_gate.py` (12 KB) and `https_asset_gate.py` pass.

## F12. Numbered, nested and definition lists (2026-11-02)

**What changed.** `<ol>` items are numbered (`start=` honoured), nested lists
are indented two spaces per level and continue the parent item without a
blank line, and `<dl>` shows terms with indented definitions.

**Evidence** (`docs/handoff/assets/browser-html-lists-2026-11-02/`): html_text
16 host tests; sealed build and master gate GREEN; native
`https_html_gate.py` at both sizes with an ordered list containing a nested
bulleted list (released text hash equals the expected text; the page is
1029 bytes, again a two-record response that closes cleanly after F11).

## F13. Script subset: branches, comparisons, booleans, helpers (2026-11-02)

**What changed.** The inline script subset (F10) now has `true`/`false`,
comparisons `< > <= >= === !== == !=`, `! && ||`, `?:`, `if (...) ... else
...` with `{ }` blocks, string `.length`, `.toUpperCase()`,
`.toLowerCase()`, and `Math.floor/ceil/round/abs/min/max`. Still no loops,
functions, calls into anything else or names for DOM/network/storage, so a
script always terminates. Untaken branches are parsed and checked but write
and assign nothing; an undeclared name even in an untaken branch refuses
the script. Expressions now use precedence climbing, which keeps stack use
per nesting level low; the nesting budget is 10 (was 8; statements now
count too). `==` compares across types numerically; `===` does not.

**Evidence** (`docs/handoff/assets/browser-script-branches-2026-11-02/`):
`tests/browser_script.rs` 5 groups (incl. branches/comparisons/helpers and
4 more refused programs), html_text 16; sealed build and master gate GREEN;
native `https_html_gate.py` at both sizes: a second script writes
`SEXOS ok` via `if`, `.length`, `.toUpperCase()`, `Math.max` and `?:`,
alongside the depth-9 arithmetic script (`scripts_run=2
scripts_refused=2`), with no pager fault rejections in the run.

## F14. Spindle `calc` remembers `ans` (2026-11-02)

**What changed.** `ans` in a `calc` expression is the previous successful
result in that Spindle session (`calc 2*(3+4)/5` then `calc ans*10` gives
`= 28`). Before any result, `ans` is a named error, not zero.

**Evidence** (`docs/handoff/assets/spindle-calc-ans-2026-11-02/`): calc host
tests (3 groups); sealed build and master gate GREEN; native
`spindle_calc_gate.py` at both sizes: `calc ans` -> `NoAnswer`, then `2.8`,
then `ans*10` -> `28`, plus the earlier cases.

## F15. Inline emphasis and code markers (2026-11-02)

**What changed.** In the HTML reader, `<code>`/`<kbd>` read as `` `x` ``,
`<strong>`/`<b>` as `*x*` and `<em>`/`<i>` as `_x_`, so emphasis and key
names survive plain-text reading.

**Evidence** (`docs/handoff/assets/browser-html-inline-2026-11-02/`):
html_text 17 host tests; sealed build and master gate GREEN; native
`https_html_gate.py` at both sizes with `Press `W` for this *page*, see
`html_text.rs`.` in the released text.

## S1. Sustained 30-minute alpha session on the F15 image (2026-11-03)

`quil_workflow_gate.py --soak-seconds 1800` on the image built after F15:
1280x800 passed (80 cycles, 171 verified captures, accepted). 1024x768 ran
84 cycles and its session result is `ok=true acceptance=true`, but its
after-run source-seal re-check then failed (`alpha checker/source not
sealed: apps/kaleidoscope/src/main.rs`) because I edited Browser sources while
it ran. The run is sound; the 1024 evidence is not seal-clean. (An earlier
1024 attempt was cut at 1707 s when my session ended.) Autosave did not fire
during either soak: the workflow saves within the idle window each cycle.
Evidence: `docs/handoff/assets/alpha-sustained-2026-11-03/`.

## F16-F20. Script subset: loops, functions, arrays, strings, more Math (2026-11-03)

- **F16 loops.** `while`, `for (init; cond; update)`, `+=`, `-=`, `++`,
  `--`. Every executed statement spends one unit of a 10,000-step budget, so
  an endless loop refuses the script instead of hanging the Browser. Arena
  raised to 8 KiB (heap, fallible).
- **F17 functions.** `function name(a, b) { ... return x }`, declared before
  use, up to 8 functions and 4 parameters; parameters shadow globals for the
  call and names declared inside end with it; call statements `f(x);`.
  Recursion is bounded by the nesting budget (12), which allows only a couple
  of levels (`fact(2)`) because the Browser stack is tight (F4/F5).
  Function bodies are checked at declaration.
- **F18 arrays.** `[a, b]`, `x[i]`, `x[i] = v` (no holes), `.length`,
  `.push(v)`, `.join(s)`; arrays are references (32 arrays, 512 elements,
  doubling growth). Self-containing arrays fail on depth, never loop.
- **F19 strings.** `.trim()`, `.indexOf()`, `.charAt()`, `.slice()` (negative
  from the end), `.substring()`, `.split()` (to an array), `String()`,
  `Number()`, `parseInt()`. Strings are bytes in this subset.
- **F20 Math.** `sqrt` (correctly rounded via an exact integer root; 100,000
  host cases equal Rust's `f64::sqrt`), `pow` (whole exponents), `trunc`,
  `sign`, `PI`. No `Math.random`: a script holds no entropy source.

**Evidence** (`docs/handoff/assets/browser-script-loops-functions-2026-11-03/`):
`tests/browser_script.rs` 10 groups; html_text 17; sealed build and master
gate GREEN; native `https_html_gate.py` at both sizes with six scripts on the
page (loop sum `20100`, while countdown, `sq(12)=144`, depth-10 parentheses
and `fact(2)`, array squares, string split/upper/indexOf/parseInt);
conversion took 9 ticks; no pager rejections.

## F21. Spindle `calc` powers (2026-11-03)

`a ** n` for whole `n` up to 64 (right-associative, tighter than unary
minus: `-2**2` is `-4`; negative powers divide, flagged `~` when inexact).
Host tests; native `spindle_calc_gate.py` types `calc 2**10` -> `= 1024`.

## F22-F23. PNG images in the Browser (2026-11-03)

**What changed.** `crates/sex-graphics/src/png.rs`: an allocation-free PNG
decoder written from the PNG spec and RFC 1950/1951 (stored, fixed and
dynamic Huffman blocks; filters 0-4; 8-bit gray, RGB, palette with tRNS,
gray+alpha, RGBA; non-interlaced; up to 48x48 like the QOI path). Every
chunk CRC and the Adler-32 are checked; consecutive IDAT chunks are read as
one stream; the caller supplies the inflate buffer (heap, fallible), which
doubles as the DEFLATE window. In the Browser, key **P** fetches the fixed
`/art.png` (`image/png`) on the same BOOT endpoint and route; the asset
panel title says PNG. Codec `Resource::ArtworkPng` with
`Resource::is_image()` replacing the QOI-only checks. No ABI input changed.

**Evidence** (`docs/handoff/assets/browser-png-2026-11-03/`):
`tests/png_decode.rs` (six zlib-made fixtures across all colour types,
filters, block kinds and split IDATs; every single-byte corruption and every
truncation refused; no panics), `tests/browser_asset.rs` one-step decode;
sex-http and sex-tls suites; sealed build and master gate GREEN; native
`scripts/https_png_gate.py` at both sizes: exact `GET /art.png`, released
body hash, `[browser.asset.decoded] ... hash=<QOI oracle PIXEL_HASH>` (same
pixels as the QOI fixture), presentation Ready, and 256 sampled screen pixels
equal the straight-alpha blend. Regressions: QOI `https_asset_gate.py`,
`/welcome` document gate, calc gate.

**Found on the way.** Two Browser host fixtures stub `sex_tls::Resource`
and needed `is_image`; three host gates had been failing until fixed in this
commit (they were not part of the native runs).

## F24-F26. `break`/`continue`, template strings, `calc sqrt` (2026-11-03)

- **F24.** `break` and `continue` in `while`/`for` (innermost loop; a
  `break` outside a loop, or crossing a function call, is a syntax error).
- **F25.** Template strings `` `Hello ${who}, ${n * n}` `` with `\n`/`\t`
  escapes; each `${...}` is parsed in place from the script source. Nested
  templates are refused.
- **F26.** Spindle `calc sqrt(x)`: exact integer square root of the scaled
  value (`sqrt(16)` -> `= 4`, `sqrt(2)` -> `~ 1.414213562`, negative -> named
  error); integers only, no floats in Spindle.

**Evidence** (`docs/handoff/assets/script-jumps-templates-calc-sqrt-2026-11-03/`):
script host tests 11 groups, calc host tests; sealed build and master gate
GREEN; native HTML gate at both sizes with a seventh page script
(`odd 1+3+5+7+9 = 25` via continue/break/template); native calc gate at
both sizes incl. `calc sqrt(2)`.

## F27-F28. Script objects and JSON (2026-11-03)

- **F27 objects.** `{a: 1, 'b': x}` literals, `o.a`, `o[key]`, `o.a = v`
  (also `+=`, `-=`, `++`, `--`), `o[key] = v`, `Object.keys(o)`; objects are
  references (16 objects, 256 properties in a fallible heap pool); missing
  properties read as `undefined`.
- **F28 JSON.** `JSON.stringify(v)` for numbers, strings (escaped),
  booleans, arrays and objects; `undefined` members are left out, `NaN` and
  infinities print `null`, a self-containing object stops at the depth
  budget with a refusal.

**Evidence** (`docs/handoff/assets/script-objects-json-2026-11-03/`): script
host tests 12 groups; sealed build and master gate GREEN; native HTML gate
at both sizes with an eighth page script writing
`{"name":"Quil","saves":3,"kind":"editor"} keys 3` (`scripts_run=8`), no
pager rejections.

## F29. Browser stack measured, then halved by a static state (2026-11-03)

**Measured.** `apps/kaleidoscope/src/stack_probe.rs` paints the unused part
of the Browser main task's 64 KiB stack once at startup and reports the
deepest use (`[browser.stack.high_water] bytes=... mapped=65536`). On the F28
image the high water was **65,280 bytes**: the Browser was running within
256 bytes of the mapped stack, which explains the F4 crash and why every new
field was dangerous.

**Fixed.** The 18.9 KiB `Browser` state no longer lives in `_start`'s frame
for the whole session: `App` is now a zero-sized handle and the state is
written once into a static (`BROWSER: MaybeUninit<Browser>`). High water is
now **46,752 bytes** at both sizes (about 18 KiB headroom); the remaining
peak is the one-time construction. The `App` size budget (F5) still bounds
that construction. No behaviour, route or authority changed.

**Evidence** (`docs/handoff/assets/browser-stack-static-2026-11-03/`): Browser
host gates and Python suite (1132) OK after keeping the identity-first
`Browser::new` the output-identity checker requires; sealed build and master
gate GREEN; native HTML gate at both sizes now records and bounds the stack
high water (fails above 60 KiB); regressions PNG, QOI asset, `/welcome`
document and connected desktop gates pass.

## F30. Deeper script recursion (2026-11-03)

With the stack measured (F29), the script nesting budget went from 12 to 32:
`fact(7)` now works (was `fact(2)`), `fact(30)` is still refused. Native HTML
gate at both sizes runs `fact(7)` and depth-10 parentheses; the stack high
water stays at the 46,752-byte construction peak (the script used about
3.7 KiB less of the painted region than before) and there are no pager
rejections. Evidence: `docs/handoff/assets/script-recursion-depth-2026-11-03/`.

## F31. Script `null`, `undefined`, `typeof`, `toFixed` (2026-11-03)

`null` and `undefined` literals (loose `null == undefined`, strict not;
arrays/objects compare by identity), `typeof` (an undeclared name is
`"undefined"`, as in JavaScript), and `n.toFixed(d)` for `d` 0..20, rounding
the exact binary value half away from zero (`(2.5).toFixed(0)` is `3`,
`(-1.005).toFixed(2)` is `-1.00`). A function called with fewer arguments
now sees `undefined` for the rest. Host tests 13 groups; native HTML gate at
both sizes with `number 20.00 true undefined` (`scripts_run=9`); stack high
water unchanged. Evidence:
`docs/handoff/assets/script-null-typeof-tofixed-2026-11-03/`.

## F32. Common named entities and minor headings (2026-11-03)

Typographic and symbol entities common on real pages (`&ldquo; &rdquo;
&lsquo; &rsquo; &bull; &middot; &deg; &times; &euro; &pound; &larr; &rarr;
&reg; &trade; &frac12; &laquo; &raquo;` and more; `&shy;` disappears) now
decode; unknown names stay literal. `<h3>`-`<h6>` read as `## title`. Host
test of the entities (not in the native page: its pixel oracle is ASCII);
native HTML gate at both sizes with `## Notes`. Evidence:
`docs/handoff/assets/html-entities-headings-2026-11-03/`.

## Finding: Spindle text never reaches the screen in the calc gate

In `spindle_calc_gate.py` runs, Spindle's four grid panes (surfaces 154,
160, 161, 162) admit their first batches, after which Spindle's display
outbox stays pending for the rest of the run and Display records no
`display.text.committed` for target 12 at all; Display logged
`[display.child.parent_gate] ... parent=153 observed=0 ... retained=1` for
each pane at startup. The calc results are correct in the serial records,
but nothing is drawn. The connected-desktop gate, which brings Spindle up
through the launcher, does draw it. Closer look: Shell tiles
Spindle's frame 153 at x=640 y=425 w=640 h=375, the same tile the Browser
occupies, and after `[shell.interact.focus] sid=153` the screenshot still
shows the Browser on top there. So keyboard focus moves to Spindle but its
window is not raised above the covering Browser, and Display (which settles
covered text damage) never commits Spindle's text, leaving Spindle's outbox
pending. User impact: typing into a focused but hidden window. Not changed
here (Shell stacking and Display are the concurrent session's area).

## G1-G5. Script: assignment operators, do/switch, array and string methods, callbacks, arrows (2026-11-03)

- **G1.** `*=`, `/=`, `%=`; `do ... while (...)` (with the JavaScript
  semicolon insertion after it); `switch` with `case`/`default`,
  fall-through, `break` ending the switch and `continue` reaching the loop.
- **G2.** Arrays: `.indexOf .includes .slice (negative ends) .concat
  .reverse .sort (string order, undefined last) .pop .shift`.
- **G3.** Strings: `.replace .replaceAll .startsWith .endsWith .includes
  .lastIndexOf .repeat (to 1024) .padStart .padEnd` (string patterns only;
  no regular expressions).
- **G4.** Function values: `function (x) {...}` expressions, named functions
  as values, variables holding functions, and array callbacks `forEach map
  filter find findIndex some every reduce` and `sort(cmp)`. Callbacks spend
  the same step budget; `forEach` visits the length present at the start,
  like JavaScript. Up to 16 functions; the same source position is one
  function, so a function expression in a loop does not fill the table.
- **G5.** Arrow functions `x => expr`, `(a, b) => expr`, `(a) => { ... }`.

Still no closures over enclosing calls' locals beyond the shared globals,
no `this`, no prototypes, no regular expressions, no DOM.

**Evidence** (`docs/handoff/assets/script-callbacks-arrows-2026-11-03/`):
script host tests 17 groups; sealed build and master gate GREEN; native HTML
gate at both sizes with a tenth page script
(`LINEN... QUIL.... SPINDLE. 23 6 four 1 a+b` from filter/map/arrow/
toUpperCase/padEnd/sort/reduce/do/switch/indexOf/replaceAll); conversion 20
ticks; stack high water unchanged at 46,752.

## G6-G8. PNG: low bit depths, 16-bit samples, Adam7 interlace (2026-11-03)

The PNG decoder (F22) now accepts every standard colour type and bit depth:
grayscale 1/2/4 (scaled to 8 bits), palette 1/2/4 (MSB-first packing), 16-bit
grayscale/RGB/gray+alpha/RGBA (high byte kept), and Adam7 interlacing (each
of the seven passes unfiltered separately, then scattered). The inflate
scratch grew to 18,864 bytes (still a fallible heap allocation, freed after
decode).

**Evidence** (`docs/handoff/assets/png-depths-adam7-2026-11-03/`): 13 new
zlib-made fixtures (all depths, interlaced and not, 1x1 and 3x2 Adam7 edge
cases) plus corruption probes in `tests/png_decode.rs`; asset host tests;
sealed build and master gate GREEN; native `https_png_gate.py` with the
original 8-bit PNG and with `--variant adam7-16` (the same oracle pixels as a
16-bit interlaced RGBA PNG, 10,353 bytes) at both sizes: decoded pixel hash
equals the oracle and 256 sampled screen pixels match.

## Release gates noted while doing F1–F2

- The reader frame is bounded by the 512-byte Linen text delivery shared with
  Display (47 columns, ~10 rows at 1280x800). A full-window reader needs a
  larger delivery region, which is a Display/Shell protocol change; deferred
  so it does not collide with the concurrent Display text work.
- The Browser main task runs with a small pre-mapped stack and a 4-page pager
  quota shared with heap growth. Adding ~0.5 KiB of inline state was enough
  to crash it (F4). F5 adds a build-time budget on the `App` state; a stack
  high-water measurement or a stack quota separate from heap is still needed.
- Display applies app paints seconds late (Quil) or not before capture
  (Spindle, F9) in short native runs. Needs a look at Display's paint
  scheduling before relying on screenshots for app output.
- (Fixed in F11) Guest HTTPS fetch failed at connection close for a ~1.1 KB
  response.
