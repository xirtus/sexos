# SexOS feature ledger (from 2026-11-02)

## Handoff summary (as of 2026-11-05, after Q9)

**Try it (native image).** Browser: `W` HTML page as text, `1`-`9`, `0`, `Shift+1`-`6` follow
links 1-16, `Backspace` back, `/` find in page, `G` QOI image, `P` PNG image (any standard depth,
interlaced or not, colour-key transparency), `B` plain document. Quil:
autosave draft after ~30 s idle (`DRAFT KEPT`), `Ctrl+D` twice deletes the
saved file, `Ctrl+I` line/word/char counts, `Ctrl+G` go to line,
Home/End on the line, `Ctrl+Left/Right` by word, `Ctrl+Home/End`, `Ctrl+Backspace/Delete` a word,
find/replace (`Shift+Enter` previous, `Ctrl+Enter` replace all), `Ctrl+L`
select line, `Ctrl+Up/Down` move line, `Ctrl+]`/`Ctrl+[` indent, `Ctrl+Shift+D`
duplicate line, `Ctrl+K` kill to line end, `Ctrl+J` join lines.
Spindle: Shift works; Tab completes command names; Ctrl+U/W/L/R edit and search; `history <text>` filters; `calc` does exact decimals with `** sqrt abs min max
round floor ceil pi e ans`, `exp ln log sin cos tan`, fractional powers, named values (`calc x = 2`), `0x`/`0b` input
and `... in hex|bin|oct` output.

**HTML reader.** Headings, paragraphs, lists (numbered/nested/definition),
aligned tables, quotes, rules, `<pre>`, inline code/emphasis markers, links
with a numbered list, images as alt text, forms as read-only text,
`sup/sub/abbr/q/details`, `ol type=a/i`, media/frame placeholders, Latin-1,
Greek and symbol entities (Display draws 94 non-ASCII glyphs), page title.

**Inline script subset** (`apps/kaleidoscope/src/script.rs`, not a JS
engine, no DOM/events/timers/network/storage/clock/entropy): numbers printed
like JavaScript, strings and string methods, regular expressions
(`regex.rs`, bounded backtracking; `exec`/`lastIndex`, `matchAll`),
`null`/`undefined`, `typeof`, `if/else`, `switch`, all loop forms incl.
`for...of/in`, labelled `break`/`continue`, under a 10,000-step budget;
`var` (function scoped) / `let`/`const`; functions with default, rest and
destructured parameters, arrows, IIFEs, named function expressions,
callbacks, closures (on `return`), `this`, `class ... extends` with
`super`, `static`, fields and `#private` names, `get`/`set`, `new`, `instanceof`, `try/catch/finally/
throw` and Error objects; arrays, objects, `Set`, `Map` with most standard
methods; object-literal shorthand/methods/computed keys/spread;
destructuring (nested, defaults, holes, rest); nested templates; `?.`,
`??`, short-circuit `&& || ?:`, `**`, bitwise operators, assignment
expressions, logical/compound assignment to any member chain, `++`/`--`;
`arguments`, `call/apply/bind`, prototypes (copied at construction), comma
operator, `console.*` (no output), `Promise` (then/catch/finally/all,
microtask order; no async/await),
`Math` (incl. exp/log/trig, no random), `JSON.stringify` (replacer keys, indent) / `parse`, `Date` from
given values only, URI encode/decode, `toPrecision`/`toLocaleString`;
output via `document.write`. Bounds: nesting 64 plus a stack floor 10 KiB
above the Browser stack bottom, 50,000 steps, 16 KiB source, 6 arguments
per call, 48 variables, 32 functions, 64 closures, 128 arrays (512
elements), 64 objects (256 properties), 32 collections.

**Native gates added** (all pass at 1280x800 and 1024x768 on the latest
image): `https_html_gate.py` (also bounds the Browser stack high water,
now 46,752 of 65,536 bytes), `https_png_gate.py` (`--variant adam7-16`
too), `quil_autosave_gate.py` (also Ctrl+I, Ctrl+G, word movement, find
next/previous, replace all, Ctrl+L), `quil_delete_gate.py`,
`spindle_calc_gate.py` (also Tab completion). Host gates:
`quil_autosave_host_gate.sh`, `spindle_calc_host_gate.sh` (calc and
completion); script, html_text and PNG tests run in
`sexnet_jobs_host_gate.sh`. Fixtures that copy production code
(`tests/quil_clipboard_ui.rs`, `tests/spindle_editor.rs`) need stubs when
key handlers call new functions.

**Open release gates** (details below): full-window reader needs a larger
Display text delivery; Display applies app paints late; Display redraws are slow (raster 100-670 ticks per frame, H2; H1 removed the inbox backlog); autosave
failure fences document I/O like any uncertain save; no draft cleanup;
images limited to 48x48; HTML sources up to 32 KiB, converted text 12 KiB.

**Next suggested steps.** `async`/`await` would need a suspendable interpreter. Fix focus raising in Shell/Display (with the
concurrent session); grow the document/body limit with streaming
conversion (12 KiB is fixed in `sex-http`'s codec); an event model for scripts only if it
fits held authority.

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
pending. User impact: typing into a focused but hidden window.

**Re-diagnosed (2026-11-03, H0).** It is not a stacking rule: Display's
`winner()` does raise the focused frame. Display is simply far behind its
inbox. In the calc gate the Shell sends `0xED` focus 300 (log line 4554)
then 153 (line 5411); Display reports `[display.focus.drawn] focus=201` at
line 5968 and `focus=300` at line 7969, i.e. it is still applying the
older focus thousands of records later, and the run ends before it reaches
153. Spindle's pending text and the late Quil paints (F3/F6) are the same
backlog. The fix is Display throughput (repaint cost under TCG), which is
the concurrent session's area; not changed here.

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

## G9-G11. HTML forms and inline extras as text; calc functions (2026-11-03)

- **G9 forms (read-only).** `<input>` shows `[input: placeholder/value]`,
  checkboxes `[ ]`/`[x]`, radios `( )`/`(*)`, submit/reset/button
  `[button: label]`, hidden inputs nothing; `<button>`, `<select>` (options
  separated by `|`) and `<textarea>` show their text in brackets; `form`,
  `fieldset`, `details` are blocks. Nothing is submitted: the Browser holds
  no route for it.
- **G10 inline extras.** `<sup>` `^x`, `<sub>` `_x`, `<abbr title>` adds
  `(title)`, `<q>` quotes, `<summary>` starts a `> ` line.
- **G11 Spindle calc functions.** `abs min max round floor ceil` on the
  exact decimal values (round half away from zero).

**Evidence** (`docs/handoff/assets/html-forms-calc-functions-2026-11-03/`):
html_text 19 host tests, calc host tests; sealed build and master gate
GREEN; native HTML gate at both sizes with
`[input: Search notes] [x] exact [button: Find]` and `x^2 LC(Long Count)`;
native calc gate at both sizes with `calc max(2,9)` -> `= 9`.

## G12-G15. Script: number/object helpers, `in`/`delete`, spread, destructuring (2026-11-03)

- **G12.** `Number.isInteger/isFinite/isNaN`, `isNaN()`, `parseFloat()` (the
  longest leading decimal number), `n.toString(radix)` for whole numbers
  (fractions in other bases are refused).
- **G13.** `Object.values/entries`, `Array.isArray`, the `in` operator on
  objects and array indexes, and `delete o.key`.
- **G14.** Spread `...array` in array literals, function calls and `Math`
  calls (array literals up to 32 items).
- **G15.** Destructuring declarations `var [a, b] = array` and
  `var {x, y} = object` (missing items are `undefined`).

**Evidence** (`docs/handoff/assets/script-spread-destructure-2026-11-03/`):
script host tests 19 groups; sealed build and master gate GREEN; native
HTML gate at both sizes with an eleventh page script writing
`9 9 hex ff true truefalse 5`; stack high water unchanged.

## G16-G17. Aligned HTML tables; Quil Ctrl+I counts (2026-11-03)

- **G16.** When a `<table>` closes, its rows are re-laid as padded columns
  (`name          | size   | kind`), counted in characters; skipped when the
  padding would not fit the 12 KiB text.
- **G17.** In Quil, **Ctrl+I** shows `N LINES / N WORDS / N CHARS` for the
  buffer in the status line (characters are UTF-8 scalars) and records
  `[quil.stats]`. Read-only.

**Evidence** (`docs/handoff/assets/html-tables-quil-stats-2026-11-03/`):
html_text and autosave host tests (incl. `counts`), Quil clipboard and
service custody host gates; sealed build and master gate GREEN; native HTML
gate at both sizes (aligned `key   | value` rows); native autosave gate at
both sizes now also presses Ctrl+I and checks `1 LINES / 2 WORDS / 9 CHARS`;
`quil_workflow_gate.py` regression passes.

## G18-G19. Script character codes and hex literals; calc constants (2026-11-03)

- **G18.** `s.charCodeAt(i)` (byte value in this byte-string subset),
  `String.fromCharCode(...)` (UTF-8 encoded), and hexadecimal literals
  (`0xff`).
- **G19.** Spindle `calc` constants `pi` and `e` to 9 places; results that
  use them are marked `~` (`calc 2*pi` -> `~ 6.283185308`).

**Evidence** (`docs/handoff/assets/script-charcodes-calc-constants-2026-11-03/`):
script host tests 20 groups, calc host tests; sealed build and master gate
GREEN; native HTML gate at both sizes (`code 83 Sex 8`); native calc gate at
both sizes (`calc 2*pi`).

## H1. Display catches up with its inbox (2026-11-03)

**Cause.** Display applied at most `DRAIN_MAX = 16` messages per loop and
then redrew; a redraw costs 200-680 ticks under QEMU, and app text arrives
8 bytes per message (Spindle sends ~17 messages per pane update). Display
therefore fell seconds behind, which is why focus changes, Quil paints and
Spindle text appeared late or not at all (see the finding above).

**Fix.** `DRAIN_MAX` is now 256: up to 256 messages are applied before each
redraw, so a burst of updates costs one frame instead of sixteen.
Rendering, message handling and authority checks are unchanged.

**Result.** In the calc gate Display now reaches `focus=153` (it never did
before), Spindle admits 132 display batches instead of 10, and the end
screenshot shows Spindle in front with `= 1024`, `~ 1.414213562`, `= 9`,
`~ 6.283185308` and the division-by-zero message. The gate now waits for
Spindle's outbox before its screenshot.

**Evidence** (`docs/handoff/assets/display-drain-2026-11-03/`): all Display
host gates (clock, cursor output, frame output, scene, ready protocol,
surface groups, surface lifecycle, text readability) OK; sealed build and
master gate GREEN; native calc gate at both sizes, connected desktop, Quil
workflow, autosave, delete, HTML, PNG, QOI asset and `/welcome` document
gates all pass.

## H2. Display redraw profiled; scanout diffing tried and dropped (2026-11-03)

A temporary `[display.profile]` probe split each redraw (calc gate, 1280x800):
frame preparation 1-12 ticks, scene capture 0-4, **raster 18-667**, scanout
publication 3-10. Publishing only changed scanout words (diffing against the
previous front frame) was implemented and passed every pixel gate, but did
not change redraw time, so it was reverted. The cost is the per-pixel
composition in `raster_full`/`raster_area` (`composite_pixel_source` with
per-pixel fill, bitmap and text lookups). Speeding that up means rendering
per span instead of per pixel, a larger change across a pixel-exact
pipeline; left as a release gate.

## I1-I5. Script: block scope, escapes, methods with `this`, array helpers, exceptions (2026-11-03)

- **I1.** `let`/`const` are block scoped (blocks and `for (let ...)`), shadow
  outer names and end with their block; assigning a `const` and redeclaring
  a `let` in the same block are refused. Code that is only checked, not run
  (untaken branches, a loop's final test), no longer refuses names that were
  never bound, as in JavaScript.
- **I2.** `\xHH`, `\uHHHH` and `\u{...}` string escapes (UTF-8; `.length`
  counts bytes in this subset).
- **I3.** Method calls `o.f(...)` with `this` bound to `o` (also chained,
  `return this`), `this.x = ...` assignments.
- **I4.** `Array.from` (array, string or `{length: n}`), `.fill`, `.at`,
  `.flat` (one level); strings `.at`, `.trimStart`, `.trimEnd`.
- **I5.** `throw`, `try { } catch (e) { } finally { }`: thrown values reach
  the nearest `catch` (across calls); an uncaught throw refuses the script;
  step/size limits are not catchable.

**Evidence** (`docs/handoff/assets/script-scope-this-try-2026-11-03/`): script
host tests 21 groups; sealed build and master gate GREEN; native HTML gate at
both sizes with a thirteenth page script writing
`Sex 2 0/skip/2 3 undefined`; stack high water unchanged.

## J1-J2. Script regular expressions; heap-safe script budgets (2026-11-03)

- **J1 regular expressions** (`apps/kaleidoscope/src/regex.rs`, written
  here): literals `/.../gi`, `.` `[...]` `[^...]` `\d \w \s \D \W \S \b \B`,
  `^ $`, capturing and `(?:)` groups (9), `|`, `* + ? {n,m}` and lazy forms,
  `i` and `g` flags; `re.test`, `s.match` (groups, or all matches with
  `g`), `s.search`, `s.replace`/`replaceAll` with `$1`-`$9`, `$&`, `$$` or a
  callback, `s.split(re)` (with captured separators), `re.source/flags`.
  Backtracking with an explicit stack, a step budget, and a rule that an
  empty loop iteration fails, so `(a*)*b` refuses instead of hanging. `/`
  after a value still divides. Byte semantics; no lookaround or
  backreferences.
- **J2 heap safety.** The first native run of J1 crashed the Browser:
  `memory allocation of 960 bytes failed`. Page conversion runs inside the
  64 KiB Browser heap next to the TLS pipe and the 12 KiB body; script
  buffers had grown to ~23 KiB. Now: string arena 4 KiB, 256 array elements,
  128 object properties; a script starts only if 20 KiB of contiguous heap
  is free (else it is refused and the page still renders); the regex
  backtracking stack is capped at 512 frames and grown fallibly.

**Evidence** (`docs/handoff/assets/script-regex-2026-11-03/`): regex unit tests
(basics, captures, refusals) and script host tests 25 groups; sealed build
and master gate GREEN; native HTML gate at both sizes with a fourteenth page
script (regex replace with a callback summing timings, `match/g`, `test`,
`split`, `$2:$1`) writing `3 49 true 7:linenms`; no Browser fault.

## K1-K3. Highlight/edit markers, Latin charsets, calc variables (2026-11-03)

- **K1.** `<mark>` `==x==`, `<del>/<s>/<strike>` `~~x~~`, `<ins>` `++x++`,
  `<u>` `__x__`, `<cite>` `_x_`, `<figure>` with a `Figure: caption` line.
- **K2.** Pages declaring `charset=windows-1252` / `iso-8859-1` / `latin1`
  in a `<meta>` are transcoded to UTF-8 (Windows-1252 table for 0x80-0x9F);
  undeclared pages stay UTF-8.
- **K3.** Spindle `calc x = expr` names a value (up to 8, 1-8 letters,
  built-in names refused) for the session; unknown names are a named
  error. `calc r = 3` then `calc r*r+1` gives `= 10`.

**Evidence** (`docs/handoff/assets/html-marks-charset-calc-vars-2026-11-03/`):
html_text 24 host tests, calc host tests 4 groups; sealed build and master
gate GREEN; native HTML gate at both sizes (`==held== not ~~ambient~~`);
native calc gate at both sizes with `calc r = 3`, `calc r*r+1`.

## L1. Quil Ctrl+G go to line (2026-11-03)

**Ctrl+G** opens a `Go to line: N` prompt in Quil's status line; digits
type the number, Backspace deletes, Enter moves the caret to the start of
that line (clamped to the last line), Esc or any chord leaves. Recorded as
`[quil.goto] line= reached= old= new=`.

**Evidence** (`docs/handoff/assets/quil-goto-2026-11-03/`): Quil clipboard and
service custody host gates; sealed build and master gate GREEN; native
autosave gate at both sizes now presses Ctrl+G, 5, Enter
(`line=5 reached=1 old=9 new=0`) and End before continuing; Quil workflow
regression passes.

## M1-M6. Script JSON.parse, `new`, classes, dates; PNG colour key; calc 0x/0b (2026-11-03)

- **M1 `JSON.parse`** (objects, arrays, escaped strings incl. `\uXXXX`,
  numbers, true/false/null; depth 16). Malformed JSON throws a catchable
  `SyntaxError: bad JSON`.
- **M2 `new`** with functions as constructors (`this` is the new object),
  `new Array(n)`, `new Object()`, `new Error(msg)` (and Type/Range/Syntax
  errors with `name`/`message`).
- **M3 `class`** with a `constructor` and methods (no `extends`, which is
  refused); methods are set on each instance.
- **M4 Dates from given values only:** `new Date(y, m, d, h, mi, s, ms)`,
  `new Date(ms)`, `new Date('YYYY-MM-DDTHH:MM:SS.sssZ')`, `Date.UTC(...)`,
  getters (`getFullYear ... getDay ... getTime`), `toISOString`,
  `toDateString`; all UTC. `new Date()` and `Date.now()` are refused: a
  script holds no clock authority.
- **M5 PNG colour key:** gray/RGB `tRNS` makes the exact keyed sample
  transparent (16-bit compared at full precision).
- **M6 Spindle `calc`** accepts `0x..` and `0b..` whole numbers.

**Evidence** (`docs/handoff/assets/script-json-new-class-date-2026-11-03/`):
script host tests 27 groups, PNG tests incl. two keyed fixtures, calc host
tests; sealed build and master gate GREEN; native HTML gate at both sizes
with `EPOCH Fri Dec 21 2012 5` (class + JSON.parse + Date); native calc gate
at both sizes (`calc 0xff+1` -> `= 256`); PNG gate (8-bit and Adam7 16-bit)
regression. Browser stack high water rose to 48,064 bytes (of 65,536;
script frames now exceed the construction peak), still under the gate's
60 KiB bound.

## M7-M9. Script short-circuit, `??`, `?.`, bare `var`, `extends`/`super` (2026-11-03)

- **M7 Short-circuit fix:** `&& || ?:` used to evaluate both sides (a
  function on the untaken side still ran). Now only the side JavaScript
  evaluates runs; the other is only checked. Added `??` and optional
  chaining `a?.b` / `a?.[k]` (the rest of the chain is skipped on
  `null`/`undefined`; `f?.()` is refused).
- **M8 Declarations:** `var a = 1, b;`, `let x;` (value `undefined`);
  a repeated `var x;` keeps the value; `const x;` is a syntax error.
- **M9 Class inheritance:** `class B extends A` (A declared earlier),
  `super(...)` in a constructor, `super.m(...)` in methods, inherited
  methods with overriding, and an implicit constructor that passes its
  arguments up.

**Evidence** (`docs/handoff/assets/script-nullish-optional-extends-2026-11-03/`):
script host tests 29 groups; sealed build and master gate GREEN; native
HTML gate at 1280x800 and 1024x768 with `EPOCH Fri Dec 21 2012 5 UTC!`
(inherited `super.loud()`, `cfg.tz?.name ?? 'UTC'`, `cfg.day?.[3] || '!'`).
Browser stack high water 49,632 of 65,536 bytes.

## M10-M12. Script `for...of`/`for...in`, default and rest parameters, `Object.assign` (2026-11-03)

- **M10 `for (x of xs)`** over arrays (elements) and strings (characters);
  **`for (k in o)`** over object keys and array indices (as strings), with
  `var`/`let`/`const` or a bare name, `break`/`continue`/`return`. Items are
  read by position as each iteration starts. `for...of` on a plain object
  is refused (not iterable in JavaScript).
- **M11 Parameters:** defaults `function f(a = 1, b = a * 2)` (a default
  runs only when its argument is `undefined`) and rest `...xs` (last
  parameter; an array) for functions, function values, arrows, class
  constructors and methods.
- **M12 `Object.assign(target, ...sources)`** (null/undefined sources skipped).

**Evidence** (`docs/handoff/assets/script-for-of-defaults-rest-assign-2026-11-03/`):
script host tests 30 groups; sealed build and master gate GREEN; native
HTML gate at both sizes with `... UTC! dayvOK 00070072`. Browser stack high
water 49,952 of 65,536 bytes.

## M13. Quil word movement and word delete (2026-11-03)

- Ctrl+Left / Ctrl+Right move to the previous word start / next word end;
  Ctrl+Home / Ctrl+End go to the start / end of the document; Shift extends
  the selection with all four. Ctrl+Backspace / Ctrl+Delete remove the word
  before / after the caret (or the selection) as one undoable edit. A word
  is a run of ASCII letters, digits, `_` or non-ASCII bytes, so a stop is
  always a UTF-8 scalar boundary (`view::word_left/word_right`).

**Evidence** (`docs/handoff/assets/quil-word-movement-2026-11-03/`): host
test `word_movement_stops_at_word_edges_and_scalar_boundaries`
(tests/quil_view.rs); sealed build and master gate GREEN; native Quil
autosave gate at 1280x800 and 1024x768 now drives `ctrl-right` (0->5
`word_right`), `ctrl-end` (5->9), `ctrl-home` (9->0) and, after draft
recovery, `ctrl-backspace` (`dir=back bytes=3 changed=1 len=14`); Quil
delete gate regression OK.

## M14. Spindle `calc ... in hex|bin|oct` (2026-11-03)

- A trailing `in hex`, `in bin` or `in oct` shows an exact whole result
  as `= 0xff`, `= 0b1010`, `= -0o10`. A fractional or inexact result
  says `calc: only whole results show in hex/bin/oct` (`NotWhole`), and a
  result too long for the line is refused, not cut.

**Evidence** (`docs/handoff/assets/spindle-calc-radix-2026-11-03/`): calc
host tests (`results_show_in_other_bases`); sealed build and master gate
GREEN; native calc gate at 1280x800 and 1024x768 with `calc ans in hex`
-> `= 0x100` and `calc 1/2 in bin` -> `error=NotWhole`.

## M15-M16. Script array/URI builtins; `split` limit fix (2026-11-03)

- **M15 Builtins:** arrays `findLast findLastIndex flatMap lastIndexOf
  toString splice unshift` (splice/unshift keep the array's identity and
  move it when it outgrows its room), `Array.of`, `Object.fromEntries`,
  string `codePointAt` (UTF-8 scalar at a byte index),
  `encodeURIComponent` / `decodeURIComponent` (a malformed escape or
  non-UTF-8 result, a URIError in JavaScript, refuses the script). Calls
  still take at most 4 arguments.
- **M16 Fix:** `split(sep, limit)` ignored `limit` (string and regex
  separators); now the result is cut to `limit` pieces.

**Evidence** (`docs/handoff/assets/script-array-uri-builtins-2026-11-03/`):
script host tests 31 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... a%20b3,1k,v`). Browser stack high water
50,240 of 65,536 bytes.

## M17-M20. Script `static`/`instanceof`, operators, expression statements, `Number` parsing (2026-11-03)

- **M17 Classes:** `static` methods (`C.m()`, inherited by subclasses; a
  method may still be named `static`) and `instanceof` for classes (with
  parents), constructor functions, `Array`, `Object`, `Function` and the
  Error types. Each object records what made it (literal, class,
  constructor or Error).
- **M18 Operators:** `**` (right-associative), bitwise `& | ^ ~ << >> >>>`
  (JavaScript ToInt32/ToUint32), `void`, compound `**= |= &= ^=` and
  logical assignment `||= &&= ??=` (the right side runs only when used).
  Precedence now follows JavaScript's table.
- **M19 Statements:** a statement may start with an expression
  (`[1, 2].forEach(...)`, `(function () {...})()`), and `++x`/`--x`;
  function values can be called directly (IIFE, `fs[0](x)`, `mk()(2)`).
- **M20 Fix:** string-to-number now follows JavaScript: `0x`/`0o`/`0b`
  integers, `Infinity`; Rust's own `inf`/`nan` spellings are `NaN`
  (previously `Number('inf')` was `Infinity` and `Number('0x10')` `NaN`).

**Evidence** (`docs/handoff/assets/script-static-instanceof-operators-2026-11-03/`):
script host tests 33 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... true 1025iife`). Browser stack high water
51,472 of 65,536 bytes (gate bound 60 KiB).

## M21. Script interpreter stack frames cut (2026-11-03)

The Browser stack high water had grown with each script round (48.0 KiB at
M1, 51.5 KiB at M20, gate bound 60 KiB of 64 KiB). Measured from the
release binary (`sub $N, %rsp` per function), one script call level cost
about 3.2 KiB, mostly `atom` (936 B, a 32-value array-literal buffer on the
stack) and `chain` (952 B, every member method in one frame).

- Array-literal items are built in a fallible 512-byte heap buffer.
- Rarely recursive arms moved out of the recursive frames into
  `#[inline(never)]` helpers: object/array literals, `new`, `Date.*`,
  `JSON/Object/Array/Number.*`, `String.*`, class statics (from `atom`);
  number/regex/array/string members (`member`, from `chain`); and
  declarations, destructuring, `document.write`, `class`, `delete`,
  `throw`, `try`, `do`, `switch`, `while` (from `statement_inner`).
- Now: `chain` 328 B, `unary`+`atom` 200 B, `statement_inner` is below the
  reporting floor; one call level is about 1.75 KiB.

**Evidence** (`docs/handoff/assets/script-stack-frames-2026-11-03/`):
script host tests 33 groups unchanged; sealed build and master gate GREEN;
native HTML gate at both sizes with identical page output and Browser stack
high water **46,752 bytes (was 51,472)**: scripts no longer set the peak.

## M22. Script calls take up to 6 arguments (2026-11-03)

- Functions, methods, constructors and built-ins now accept up to 6
  arguments (was 4): e.g. `splice(i, n, a, b, c)`, `new Date(y, m, d, h,
  mi, s)`. A 7th argument still refuses the script (`Limit`). Affordable
  after M21's frame cut.

**Evidence** (`docs/handoff/assets/script-six-arguments-2026-11-03/`):
script host tests 34 groups; sealed build and master gate GREEN; native
HTML gate at both sizes with a 6-argument call (`... 00070074 ...`);
Browser stack high water unchanged at 46,752 bytes.

## M23-M24. Script `++`/`--` in expressions; member-chain assignment (2026-11-03)

- **M23** `n++`, `n--` (old value) and `++n`, `--n` (new value) inside
  expressions (`a[i++]`, `return n++`, `while (c++ < 3)`); a `++` on a new
  line starts a new statement (ASI), as in JavaScript.
- **M24** Assignment targets can be any member chain: `o.a.b = v`,
  `this.items[i] = v`, `m[1][0] = 7`, `o['a'].b[0] -= 1`, with every
  compound and logical assignment operator; `arr.length = n` shrinks an
  array (growing it is refused). Previously only `name`, `name.key` and
  `name[i] = v` worked, and `a[i]++` was a syntax error.

**Evidence** (`docs/handoff/assets/script-increments-member-assign-2026-11-03/`):
script host tests 35 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... 1025iife 51`). Browser stack high water
46,752 bytes.

## M25-M27. Script object literals, richer destructuring, JSON fix (2026-11-03)

- **M25 Object literals:** shorthand `{a}`, methods `{m() {...}}` (called
  with `this` = the object), computed keys `{[k]: v}`, number keys, a
  trailing comma, and `...spread` of objects (null/undefined add nothing;
  spreading an array into an object is refused).
- **M26 Destructuring:** defaults (`{x = f()}`, run only for an undefined
  item), aliases (`{y: why}`), array holes (`[a, , c]`), rest elements
  (`[first, ...others]`, `{a, ...more}`), and patterns in `for...of` /
  `for...in` (`for (const [k, v] of Object.entries(o))`). One pattern
  parser checks and binds both forms.
- **M27 Fix:** `JSON.stringify` printed function-valued members as the
  text `function`; like JavaScript it now leaves them out of objects and
  writes `null` in arrays.

(Closures followed in M28.)

**Evidence** (`docs/handoff/assets/script-object-literals-destructuring-2026-11-03/`):
script host tests 36 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... Zdaymode`). Browser stack high water 46,752.

## M28-M29. Script closures; function-scoped `var` fix (2026-11-03)

- **M28 Closures.** A function value that escapes its creating call by
  `return` (directly, or as a member of a returned object or array) becomes
  a closure over one copy of that call's variables (parameters, `var`,
  `let`, `const`). Functions returned together share the copy, and writes
  persist across calls: counters (`() => ++c`), the module pattern
  (`(function () { var n; return {inc() {...}, get() {...}} })()`),
  `adder(5)`. Capture happens only on escape, so callbacks created in loops
  cost nothing. Bounds: 32 closures and 8 environments per script (heap,
  fallible). Not modelled: two separately returned closures sharing one
  variable, or a closure escaping by assignment rather than `return`.
- **M29 Fix: `var` and parameters were not function scoped.** A parameter
  or `var` reused any outer binding of the same name (restored afterwards
  for parameters only), so `var x = 2` inside a function overwrote a global
  `x`. Each call now has a frame base: parameters are new bindings, and
  `var` reuses only a binding of the same call. Variable slots grew 16 ->
  32 since calls no longer reuse outer slots.

**Evidence** (`docs/handoff/assets/script-closures-var-scope-2026-11-03/`):
script host tests 37 groups (incl. `closures_and_function_scoped_var`); all
earlier expectations unchanged; sealed build and master gate GREEN; native
HTML gate at both sizes (`... Zdaymode2`, an IIFE counter). Browser stack
high water 46,752.

## M30. Spindle Tab completes command names (2026-11-03)

- When no history ghost suggestion applies, Tab completes the command
  name being typed (one word, caret at the end) from a fixed sorted table
  of Spindle's 100 commands (`apps/spindle/src/complete.rs`): a unique
  match gets a trailing space (`sext` -> `sextime `); several matches
  extend to their shared prefix (`proj` -> `project-scene-`); a further
  Tab lists them above the prompt. It reads only the fixed table.

**Evidence** (`docs/handoff/assets/spindle-tab-completion-2026-11-03/`):
completion host test (in `spindle_calc_host_gate.sh`); sealed build and
master gate GREEN; native Spindle gate at 1280x800 and 1024x768 now also
types `sext`, Tab, `0`, Enter (sextime runs) and `proj`, Tab, Tab
(`matches=2 added=10`, then the listing).

## M31. Quil replace all (2026-11-03)

- In find mode with the replacement field active (`Ctrl+F`, query, `Down`,
  replacement), `Ctrl+Enter` replaces every non-overlapping occurrence,
  left to right, as **one undoable edit** that covers only the first match
  to the last (`find::replace_all`, rewritten into a static 12 KiB scratch;
  a span that would not fit is refused, not cut). `NOT FOUND` when nothing
  matches.
- Also fixes the `spindle_editor` host fixture (in
  `service_custody_host_gate.sh`), which M30 left without a
  `complete_command` stub.

**Evidence** (`docs/handoff/assets/quil-replace-all-2026-11-03/`): host test
`replace_all_rewrites_only_the_matched_span` (tests/quil_find.rs);
service-custody, clipboard and sexnet host gates pass; sealed build and
master gate GREEN; native Quil autosave gate at both sizes now, after draft
recovery, finds `n`, replaces with `N` via Ctrl+Enter:
`[quil.replace.all] count=2 start=7 end=12 new_bytes=5 applied=1 len=14`.

## M32-M33. Quil find previous; select line (2026-11-03)

- **M32** In find mode, `Shift+Enter` (query field) selects the previous
  occurrence ending before the selection, wrapping once to the last one
  (`find::prev_match`).
- **M33** `Ctrl+L` selects the caret's whole line including its newline;
  pressed again on a selected whole line, it extends by the next line.

**Evidence** (`docs/handoff/assets/quil-find-prev-select-line-2026-11-03/`):
host test `finds_backwards_then_wraps_once` (tests/quil_find.rs); clipboard
and service-custody host gates pass; sealed build and master gate GREEN;
native Quil autosave gate at both sizes: `Enter` finds `n` at 7 (wrapped),
`Shift+Enter` at 11 (wrapped), replace all, `Esc`, `Ctrl+L`
(`[quil.select.line] start=0 end=14 extended=0 ok=1`); Quil delete gate
regression OK.

## M34-M35. Script getters/setters; labelled break/continue (2026-11-03)

- **M34 Accessors:** `get name()` / `set name(v)` in object literals and
  classes (inherited by subclasses), run with `this` = the object. Kept in
  a separate accessor table (32 per script) so property listing is
  unchanged; unlike JavaScript, `Object.keys`/`JSON.stringify` do not list
  accessor properties. Assigning to a getter-only property is ignored
  (sloppy-mode behaviour). `get`/`set` remain usable as ordinary names.
- **M35 Labels:** `label: statement`, `break label` (loops and plain
  blocks), `continue label` (loops), through nested `for`, `for...of/in`,
  `while`, `do` and `switch`. A labelled jump meant for an outer statement
  stays pending through inner loops.

**Evidence** (`docs/handoff/assets/script-accessors-labels-2026-11-03/`):
script host tests 39 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... Zdaymode231a2a`: a class getter and a
`continue outer`). Browser stack high water 46,752.

## M36. Script `String`/`Number`/`Boolean` as function values (2026-11-03)

- `String`, `Number` and `Boolean` can be passed and stored as functions
  (`xs.map(Number)`, `xs.filter(Boolean)`, `var f = Number`), and
  `Boolean(x)` converts. A script's own binding of the same name wins.
  `parseInt` is deliberately not a function value: `map(parseInt)` relies
  on JavaScript passing the index as a radix, which the subset does not
  model.

**Evidence** (`docs/handoff/assets/script-builtin-function-values-2026-11-03/`):
script host tests 40 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... Zdaymode231a2a6`). Browser stack high water
46,752.

## N1. Script stack floor; nesting limit 32 -> 64 (2026-11-03)

- The Browser gives the script interpreter a **stack floor**: 8 KiB above
  the bottom of its mapped 64 KiB stack (`stack_probe::script_floor`).
  Every statement, expression and call checks the stack pointer against
  it, and nesting that reaches it refuses the script (`Limit`, writes
  nothing) instead of overrunning the stack, whatever the frame sizes.
  Refusals are counted (`[browser.script.stack_floor] hits=N`). Host
  tests and the gate oracle run without a floor.
- With that guard the nesting budget doubles (`MAX_DEPTH` 64): plain
  recursion now reaches `fact(15)` (was `fact(7)`).

**Evidence** (`docs/handoff/assets/script-stack-floor-2026-11-03/`): script
host tests 40 groups; sealed build and master gate GREEN; native HTML gate
at both sizes with `fact(15)` = `1307674368000` and a callback recursion
`r(400)` refused at the floor (`hits=1`, `scripts_refused=3`, no output).
Browser stack high water 57,936 bytes: the refused recursion ran down to
the floor (57,344) plus a leaf frame, under the gate's 60 KiB bound.

## N2-N3. Script `Set`/`Map`; assignment expressions (2026-11-03)

- **N2 `Set` and `Map`:** `new Set(iterable)`, `new Map([[k, v], ...])`,
  `add/set/get/has/delete/clear/size/forEach/keys/values/entries`, chaining,
  SameValueZero keys (NaN equals NaN, objects by identity), insertion
  order, spread (`[...new Set(xs)]`), `Array.from`, `for...of` (a Map gives
  `[key, value]`), `instanceof`. Up to 16 collections per script.
  `keys/values/entries` return arrays rather than iterators.
- **N3 Assignment expressions:** an assignment can appear wherever an
  expression can, with the assigned value as its value: `a = b = 0`,
  `(o.k = 2) * 3`, `if (x = f())`, arrow bodies such as `x => t += x`.

**Evidence** (`docs/handoff/assets/script-set-map-assign-expr-2026-11-03/`):
script host tests 41 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... mispx`); stack floor refusal unchanged
(`hits=1`), high water 58,048 bytes.

## N4-N6. Script number formatting, `exec`/`matchAll`, object helpers (2026-11-03)

- **N4 Numbers:** `toPrecision(p)` (exponential when the exponent is below
  -6 or at least p, as JavaScript), `toLocaleString()` in the en-US form
  (grouped thousands, at most 3 fraction digits), and the `Number`
  constants (`MAX_SAFE_INTEGER`, `EPSILON`, `MAX_VALUE`, ...).
- **N5 Regular expressions:** `re.exec(s)` with `lastIndex` for `g`
  (the `while ((m = re.exec(s)) !== null)` loop), `re.lastIndex`, and
  `s.matchAll(re)` (an array of match arrays, not an iterator).
- **N6 Objects and strings:** `hasOwnProperty`, `Object.hasOwn`,
  `Object.freeze`/`isFrozen` (assignments to a frozen object are ignored),
  arrays' `keys/values/entries`, and `localeCompare` (letters compared
  case-insensitively first, lower case before upper).

**Evidence** (`docs/handoff/assets/script-format-exec-helpers-2026-11-03/`):
script host tests 42 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... 98,765.432 0.012`). High water 58,472 bytes
(the floor-refused recursion; bound 60 KiB).

## N7-N8. HTML entities; script stack floor reserve 10 KiB (2026-11-03)

- **N7** The HTML reader decodes the full Latin-1 letter set (`&eacute;`,
  `&szlig;`, `&AElig;`, ...), Greek letters (`&alpha;`..`&omega;`, capitals,
  `&sigmaf;`), and about 90 other common named references (math
  `&le; &ge; &ne; &infin; &sum;`, arrows, `&check;`, `&dagger;`, card suits,
  typographic spaces and quotes); `&zwj; &zwnj; &lrm; &rlm;` vanish like
  `&shy;`. Unknown names stay literal. (Whether a glyph draws depends on
  the reader font; the native reader pixel oracle only checks ASCII, so the
  gate page stays ASCII and the entities are covered by the host test.)
- **N8** The script stack floor moved from 8 to 10 KiB above the stack
  bottom: a recursion refused at the floor now peaks at 56,784 bytes
  (was 58,472), leaving more room under the gate's 60 KiB bound.

**Evidence** (`docs/handoff/assets/html-entities-stack-floor-10k-2026-11-03/`):
host test `latin1_greek_and_symbol_entities` (tests/browser_html_text.rs,
25 tests); sealed build and master gate GREEN; native HTML gate at both
sizes (`floor_offset=10240 hits=1`, high water 56,784).

## N9. Display glyphs for Latin-1, quotes, dashes and common symbols (2026-11-03)

- Display's 5x7 scalar font grew from 16 to 94 glyphs: the accented
  Latin-1 letters (grave, acute, circumflex, tilde, diaeresis, ring;
  capitals with the letter compressed under the accent, as the existing
  `É`/`Ä`), `ç Ç ñ Ñ`, `° ± × ÷ « » © £ ¢ ¥ § µ ¿ ¡ · ¬ ² ½`, curly
  quotes, en/em dash, bullet, minus, `≤ ≥ ≠ ∞ π α β ✓`. Other scalars still
  draw the explicit box. The table is generated from the ASCII glyphs plus
  accent rows and kept sorted and distinct; it is a constant ROM change
  only (no layout or protocol change).

**Evidence** (`docs/handoff/assets/display-latin1-glyphs-2026-11-03/`,
including `glyph-sheet.png` rendered from the table): display text
readability and surface-group host gates pass (table size assertion now
94), desktop pixel checker tests pass (29); sealed build and master gate
GREEN; native HTML (1280x800) and Quil autosave (1024x768) regressions OK.

## N10. Browser follows links 10-16 (2026-11-03)

- The reader lists up to 16 links but only `1`-`9` followed them. Now `0`
  follows link 10 and `Shift+1`..`Shift+6` links 11-16. A number beyond the
  page's links is refused locally (`[browser.html.link.refused] ...
  network=0`) like any non-endpoint target.

**Evidence** (`docs/handoff/assets/browser-links-10-16-2026-11-03/`): sealed
build and master gate GREEN; native HTML gate at both sizes presses `0`
(`index=10 listed=0 ... network=0`) and `Shift+4` (`index=14`) on the
3-link page before following link 1 as before.

## N11-N12. Script nested templates, named function expressions, replace fix (2026-11-03)

- **N11 Templates:** a template literal now ends at its real closing
  backtick: `\`` escapes and `${...}` substitutions with their own strings,
  braces and **nested templates** (`` `${xs.map(x => `<${x}>`)}` ``) are
  skipped. Before, the first backtick ended the template.
- **N12** Named function expressions can call themselves
  (`var f = function fact(n) {... fact(n - 1)}`); nested destructuring
  patterns (`{a: {b}, c: [d, e = 4]}`, also in `for...of`); `reduceRight`;
  `Array.from(x, mapFn)`. **Fix:** `'s'.replace('b', fn)` with a string
  pattern inserted the text `function`; the function is now called with
  `(match, offset, string)`.

**Evidence** (`docs/handoff/assets/script-templates-named-fn-builtins-2026-11-03/`):
script host tests 43 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... [1][2]a+b`).

## N13. Browser find in page (2026-11-03)

- In the visible reader, `/` opens find: the footer shows
  `FIND: query_ ENTER NEXT/ESC`; letters, digits, space and `-.,` edit the
  query (24 bytes), Backspace erases, Enter selects the next match after
  the selection (ASCII case-insensitive, wrapping once) and scrolls it
  into view, Esc closes. Purely local: no service or authority.
- Shell's Browser key filter (`browser_input::command`) now also passes
  `/` and space; its host test lists them.
- Also fixes the `browser_reader_visibility` host fixture (in
  `service_custody_host_gate.sh`), which N10 left without `page_html` /
  `follow_link`. That gate showed one flaky run (tests sharing the global
  clipboard client across threads), then passed three times in a row.

**Evidence** (`docs/handoff/assets/browser-find-in-page-2026-11-03/`):
clipboard, service-custody, sexnet and Quil clipboard host gates pass;
sealed build and master gate GREEN; native HTML gate at both sizes types
`/`, `sum`, Enter (`[browser.find] query_bytes=3 start=206 end=209
wrapped=0 found=1 selected=1`, checked against the expected text), Esc.

## N14. Spindle Ctrl line editing (2026-11-03)

- Shell now passes Ctrl to Spindle (bit 9 of the key word, beside Shift in
  bit 8). Ctrl chords never insert text: `Ctrl+U` clears the line,
  `Ctrl+W` deletes the word before the cursor, `Ctrl+L` clears the
  scrollback, `Ctrl+R` recalls the newest history entry containing the
  typed text and, pressed again, the next older one.

**Evidence** (`docs/handoff/assets/spindle-ctrl-editing-2026-11-03/`): all
host gates pass (sexnet, service-custody, clipboard, Quil clipboard,
Spindle calc, app activation); sealed build and master gate GREEN; native
Spindle gate at both sizes: `echo ab cd` + Ctrl+W (`len=8`), Ctrl+U
(`len=0`), `0xff` + Ctrl+R recalls `calc 0xff+1` (`len=11`), Enter -> `256`.

## N15. Quil move line up/down (2026-11-03)

- `Ctrl+Up` / `Ctrl+Down` swap the caret's line with the line above /
  below as one undoable edit (the span of both lines is rewritten once);
  the caret keeps its column in the moved line. At the first / last line
  nothing changes (`applied=0 reason=edge`).

**Evidence** (`docs/handoff/assets/quil-move-line-2026-11-03/`): Quil
clipboard, service-custody and sexnet host gates pass; sealed build and
master gate GREEN; native Quil autosave gate at both sizes adds a second
line `x` after recovery and moves it up
(`[quil.line.move] up=1 start=0 end=16 caret=1 applied=1 len=16`).

## N16. Quil indent / outdent lines (2026-11-03)

- `Ctrl+]` indents (a leading tab) and `Ctrl+[` outdents (one leading tab,
  else up to four spaces) every line the selection touches, as one
  undoable edit; the lines stay selected so the chord can repeat. Tab
  itself is the desktop's focus key (Shell consumes it as
  `AccessFocusNext`), which the first native attempt showed, so the
  chords use brackets.

**Evidence** (`docs/handoff/assets/quil-indent-lines-2026-11-03/`): Quil
clipboard, service-custody and sexnet host gates pass; sealed build and
master gate GREEN; native Quil autosave gate at both sizes: Ctrl+A, Ctrl+]
(`[quil.indent] lines=2 out=0 start=0 end=18 applied=1 len=18`), Ctrl+[
(`lines=2 out=1 ... len=16`).

## N17. HTML reader: list paragraphs, marker styles, media, `<pre><code>` (2026-11-03)

- A paragraph (or other block) opening right after a list marker stays on
  the marker's line (`<li><p>x</p>` was an empty bullet and blank lines),
  and blocks inside lists are not spaced apart.
- `<ol type=a|A|i|I>` numbers with letters or roman numerals (with
  `start`).
- `<video>`, `<audio>` and `<iframe>` show `[video: title]`, `[audio]`,
  `[frame]` and their fallback content is skipped (nothing is played or
  loaded).
- `<code>`/`<kbd>` inside `<pre>` get no backtick markers; `&#9;` is a
  space instead of staying literal.

**Evidence** (`docs/handoff/assets/html-lists-media-pre-2026-11-03/`): html
text host tests 26 (new `list_paragraphs_marker_styles_media_and_preformatted_code`);
sealed build and master gate GREEN; native HTML gate at both sizes with
`i. roman one / ii. roman two` and `[video: Clip]` (fallback absent).

## N18. Script destructured parameters, `JSON.stringify` layout, `document.write` expressions (2026-11-03)

- Parameters can be patterns: `([k, v]) => ...`, `function f({x, y = 2},
  [p, q])` (functions, arrows, methods).
- `JSON.stringify(v, keys, space)`: a replacer key list (at every level)
  and an indent (a number up to 10, or text up to 10 bytes), laid out like
  JavaScript (empty `[]`/`{}` stay on one line, `": "` after keys). A
  function replacer refuses the script.
- `document.write(...)`/`writeln(...)` also work as expressions, e.g. an
  arrow body `xs.forEach(x => document.write(x))`.

**Evidence** (`docs/handoff/assets/script-param-patterns-pretty-json-2026-11-03/`):
script host tests 44 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... p118`).

## N19. Quil duplicate line (2026-11-03)

- `Ctrl+Shift+D` copies the caret's line below itself as one undoable
  edit; the caret moves to the same column in the copy. Plain `Ctrl+D`
  keeps its meaning (press twice to delete the saved document).

**Evidence** (`docs/handoff/assets/quil-duplicate-line-2026-11-03/`): Quil
clipboard, service-custody and sexnet host gates pass; sealed build and
master gate GREEN; native Quil autosave gate at both sizes
(`[quil.line.duplicate] start=16 bytes=14 caret=31 applied=1 len=31`);
Quil delete gate (plain Ctrl+D) regression OK.

## P1. Browser private heap 64 -> 128 KiB (2026-11-03)

- The Browser's ELF-owned heap arena (`sex-rt/src/heap.rs`, used only by
  the Browser) grew to 128 KiB, the first step toward a larger page limit:
  with a 12 KiB body the peak was 48,400 bytes (74% of 64 KiB); a 24 KiB
  body needs about 36 KiB more. Not an ABI input; the Browser ELF is
  ~991 KB on the image.
- `browser_reserve_gate.py` now names the heap size and the Browser's
  permanent 768-byte state (page-link table and history boxes, added with
  the HTML link rounds). The gate had been expecting `heap_used=0` after
  a clear and so was failing since those rounds; it is green again.

**Evidence** (`docs/handoff/assets/browser-heap-128k-2026-11-03/`): sealed
build and master gate GREEN; native HTML and PNG gates (1280x800) with
`heap_total=131072` (peak 48,400 used); Browser reserve gate at 1280x800
and 1024x768.

## P2. Script budgets raised with the 128 KiB heap (2026-11-03)

- Script budgets, previously shrunk for the 64 KiB heap: source 4 -> 16
  KiB, output 1 -> 4 KiB, string arena 4 -> 12 KiB, steps 10,000 ->
  50,000, variables 32 -> 48, functions 16 -> 32, closures 32 -> 64,
  array elements 256 -> 512, arrays 32 -> 128, objects 16 -> 64,
  properties 128 -> 256, classes/regexes/collections/accessors doubled.
  A script starts only with 40 KiB of free heap (was 20).
- The interpreter's fixed tables (variables, functions, arrays, objects,
  constructor tags) moved from the Browser stack to fallible heap
  allocations: with the larger tables on the stack, the first native run
  refused `fact(15)` at the stack floor.
- Fixes: `Object.keys/values/entries` and string `split` were capped at
  16 results by fixed buffers; constructor tags became 16-bit so 32
  functions cannot collide with the Error/Set/Map tags.
- The 12 KiB page-body limit stays: `sex-http` is allocation-free, a
  larger receiver would push the shared TLS task past its 32 KiB budget,
  and ~8 gates encode the figure (also Quil's import bound). Follow-up:
  a page-only receive path that converts HTML straight into the 12 KiB
  text document.

**Evidence** (`docs/handoff/assets/script-budgets-heap-tables-2026-11-03/`):
script host tests 45 groups; sealed build and master gate GREEN; native
HTML gate at both sizes with a 16th script (`big 79800 40`: a 400-element
array, a 40-property object), `scripts_run=16 scripts_refused=3`, stack
floor hits 1 (the intended deep recursion), high water 56,952.

## P3. Script Promises (2026-11-03)

- `new Promise((resolve, reject) => ...)` (a throw in the executor
  rejects), `then(f, g)`, `catch(g)`, `finally(h)`, `Promise.resolve`,
  `Promise.reject`, `Promise.all`, adoption of a promise by another, and
  `instanceof Promise`. Reactions are microtasks run in JavaScript's
  order after the script's own code (e.g. `Promise.all` settles after its
  items' reactions), all under the step budget. Nothing asynchronous
  exists to wait for (no timers or network), so every promise settles
  within the script. `async`/`await` remain refused: the interpreter
  cannot suspend a call, so their ordering could not be honest.
- Statements may start with `new`, `void` or `typeof`
  (`new Promise(...).then(...)`, `new A()`).

**Evidence** (`docs/handoff/assets/script-promises-2026-11-03/`): script
host tests 46 groups; sealed build and master gate GREEN; native HTML gate
at both sizes with a 17th script (`promise first then 3 async 1+2`).

## P4. Script optional calls, tagged templates, `String.raw` (2026-11-03)

- `f?.(args)` calls unless `f` is null/undefined (`o.m?.()` too; the
  arguments are then only checked).
- Tagged templates: ``tag`a${x}b` `` calls `tag(['a', 'b'], x)` with the
  cooked text pieces; ``String.raw`a\n${x}` `` keeps escapes as written.

**Evidence** (`docs/handoff/assets/script-optional-call-tagged-templates-2026-11-03/`):
script host tests 47 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`promise first 1\dx-yundefined ...`).

## P5. Spindle `calc` scientific functions (2026-11-03)

- `exp ln log sin cos tan` (radians) and fractional powers of positive
  bases (`2**0.5`, via exp(e ln b)). Computed with i128 integer
  arithmetic at 18 decimal places (range reduction plus Taylor/atanh
  series), shown to 9 and marked `~` except at trivial points (`exp(0)`,
  `ln(1)`, `sin(0)`, `cos(0)`). `ln` of a non-positive number, a
  fractional power of a non-positive base and `tan` at a pole are
  `Domain`; `exp` above 88 is `Overflow`. Results are exact to 9 places
  for exact inputs; inputs built from 9-place values (like `pi/6`) carry
  their own rounding (`sin(pi/6)` is `~ 0.499999999`).

**Evidence** (`docs/handoff/assets/spindle-calc-scientific-2026-11-03/`):
calc host tests (`transcendental_functions_to_nine_places`); sealed build
and master gate GREEN; native Spindle gate at both sizes (`calc ln(2)` ->
`~ 0.693147181`, `calc 2**0.5` -> `~ 1.414213562`).

## P6. Spindle `history <text>` (2026-11-03)

- `history <text>` lists the history entries containing the text, newest
  first, leaving out earlier `history ...` searches; `No history entries
  match.` otherwise. `history` and `history clear` are unchanged.

**Evidence** (`docs/handoff/assets/spindle-history-search-2026-11-03/`):
host gates pass; sealed build and master gate GREEN; native Spindle gate at
both sizes (`history ln` -> `[spindle.history.search] bytes=2 found=1`).

## P7. Script `Math` elementary functions (2026-11-03)

- `Math.exp expm1 log log1p log2 log10 sin cos tan asin acos atan atan2
  sinh cosh tanh cbrt hypot` and the constants `E LN2 LN10 LOG2E LOG10E
  SQRT2 SQRT1_2`, in `apps/kaleidoscope/src/fmath.rs` without libm:
  range reduction (split ln2 / pi/2 constants) and series. A host test
  sweeps -40..40 against Rust's std and keeps every result within 4 ulps;
  `log10`/`log2` are exact at exact powers (`Math.log10(1000)` is 3, as in
  JavaScript engines), `exp(0)`/`log(1)` exact. `Math.random` stays absent
  (no entropy authority).

**Evidence** (`docs/handoff/assets/script-math-elementary-2026-11-03/`):
script host tests 48 groups (incl. `elementary_functions_stay_within_a_few_ulps_of_std`);
sealed build and master gate GREEN; native HTML gate at both sizes
(`big 79800 40 521`: `Math.hypot(3, 4)`, `Math.log10(100)`,
`Math.sin(Math.PI / 2)`).

## P8. HTML tables: captions and implicit row ends (2026-11-03)

- **Fix:** `<caption>` text became an extra first column of the first
  row; it is now its own line above the aligned rows.
- **Fix:** rows without `</tr>` (the end tag is optional in HTML) ran
  together on one line; a new `<tr>` now ends the previous row.

**Evidence** (`docs/handoff/assets/html-table-caption-rows-2026-11-03/`):
html text host tests 27 (`table_captions_and_implicit_row_ends`); sealed
build and master gate GREEN; native HTML gate at both sizes with a
captioned table of implicit rows (`Ports / dns   | 53 / https | 443`).

## P9. HTML repeated breaks and no-break spaces (2026-11-03)

- **Fix:** `<br><br>` collapsed into one line break; repeated breaks now
  leave blank lines (up to two).
- **Fix:** `&nbsp;` was an ordinary collapsible space, so `a&nbsp;&nbsp;b`
  showed one space; no-break spaces are now always written (as plain
  ASCII spaces, keeping the reader output ASCII).

**Evidence** (`docs/handoff/assets/html-breaks-nbsp-2026-11-03/`): html
text host tests 28 (`repeated_breaks_and_no_break_spaces`); sealed build
and master gate GREEN; native HTML gate at both sizes (`wide  gap` /
blank line / `after break`).

## P10. Regression on the final image of the P batch (2026-11-03)

All 11 host gates pass (sexnet jobs, service custody, Quil clipboard,
clipboard, Spindle calc, Quil autosave, Spindle sextime, browser text,
Display text readability, Display surface groups, app activation). On the
sealed image at the batch's head: PNG (8-bit 1280x800, Adam7 16-bit
1024x768), Browser reserve (1280x800, 128 KiB heap), Quil autosave
(1280x800, all Quil editing chords) and Spindle calc (1024x768, incl.
Ctrl editing, Tab completion, scientific functions, history search)
native gates pass; the HTML gate passed at both sizes in P9.

Evidence: `docs/handoff/assets/batch-regression-2026-11-03/`.

## Q1. HTML pages up to 32 KiB of source (2026-11-03)

- `sex-http`: the receiver is generic over its body storage
  (`ReceiverIn<B>`); `Receiver` stays the inline 12 KiB form, and
  `ReceiverIn::page(&mut buffer)` writes into a caller-owned buffer. Only a
  `PageHtml` response may declare up to `MAX_PAGE_SOURCE` (32 KiB); every
  other resource keeps `MAX_BODY`. `keep()` leaves a finished body in the
  caller's buffer (not erased on drop); a failed receive erases it.
  Still allocation-free.
- `sex-tls`: a page fetch receives into a fallible 32 KiB heap buffer
  (the TLS task does not grow). A page that fits 12 KiB is copied as
  before; a larger source becomes the document's buffer
  (`Document.bytes` is now a boxed slice) until the Browser's `rewrite`
  converts it into at most 12 KiB of text. `Pipe` grew by 8 bytes
  (`PIPE_METADATA_BYTES` 56 -> 64); the gates that bind it from source,
  and their synthetic fixtures (`8248` -> `8256`, totals +8), are updated.
- The converted text is still bounded by 12 KiB (Quil import and the
  document view are unchanged); the source can now carry up to 32 KiB of
  markup, styles and scripts.

**Evidence** (`docs/handoff/assets/browser-page-receive-32k-2026-11-03/`):
`sex-http` tests 43 (new `pages_up_to_the_page_source_limit_arrive_in_the_callers_buffer`,
`a_failed_page_receive_erases_the_callers_buffer`); `sex-tls` suites all
pass (new `an_oversized_page_source_is_rewritten_into_a_document`); the
Python gate unit suite gains no new failures (the 7 it had were left by P1,
fixed in Q10); host gates
pass; sealed build and master gate GREEN; native HTML gate at both sizes
follows a **15,630-byte** page (`source_bytes=15630 text_bytes=149`);
native HTTPS document, asset, PNG and Browser reserve gates pass.

## Q2. Script class fields, static fields, `#private` names (2026-11-03)

- Instance fields (`count = 0;`, `y = this.x + 1`), set on each new
  instance before its constructor runs, parent class first; static fields
  (`static made = 0`, readable/writable as `C.made`, `C.made++`, inherited
  by subclasses, initialized in order after the class exists).
- `#name` is accepted as a member name; private fields and class methods
  are no longer listed by `Object.keys/values/entries`, `for...in` or
  `JSON.stringify` (methods were listed before: they live on each instance
  here, on the prototype in JavaScript). Privacy itself is not enforced.

**Evidence** (`docs/handoff/assets/script-class-fields-2026-11-03/`): script
host tests 49 groups; sealed build and master gate GREEN; native HTML gate
at both sizes (`... 1a2a36 ...`: `Base.count` incremented by three
constructions).

## Q3. Script copying array methods, `Object.groupBy`, `structuredClone`; frames re-cut (2026-11-03)

- `toSorted`, `toReversed`, `with(i, v)`, `toSpliced(...)` return changed
  copies; `Object.groupBy(items, fn)`; `structuredClone(v)` deep-copies
  plain arrays and objects (functions, class instances and collections are
  refused).
- The first native run showed `fact(15)` refused at the stack floor: the
  P/Q rounds had regrown the recursive frames (`unary`+`atom` 200 -> 408,
  `chain` 328 -> 520, `member` 616 -> 1,144). `document.write` as an
  expression, `structuredClone` and tagged templates moved to
  `#[inline(never)]` helpers, and `member` dispatches to per-type helpers
  (`member_number/regex/array`): now `unary` 200, `chain` 456, `member` 360.

**Evidence** (`docs/handoff/assets/script-copying-array-methods-2026-11-03/`):
script host tests 50 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`big 79800 40 521 1232`, `scripts_run=17
scripts_refused=3`, floor hits 1, high water 56,016).

## Q4. HTML table header rule (2026-11-03)

- A table row made only of `<th>` cells is followed by a rule under the
  aligned columns (`------+-----`, `-+-` between columns). If a table cannot
  be aligned (no room), the rule stays a short `-` line.

**Evidence** (`docs/handoff/assets/html-table-header-rule-2026-11-03/`):
html text host tests 28 (three table expectations updated for the rule);
sealed build and master gate GREEN; native HTML gate at both sizes
(`name  | port` / `------+-----` / `dns   | 53` / `https | 443`).

## Q5-Q6. Quil Home/End on the line; kill and join lines (2026-11-05)

- **Q5 Fix:** Home and End went to the start and end of the whole document
  (found by the native gate on a three-line document). They now stay on the
  caret's line (`view::line_start/line_end`); `Ctrl+Home/End` remain the
  document ends. `connected_desktop_gate.py` uses Ctrl+End accordingly.
- **Q6** `Ctrl+K` deletes from the caret to the end of the line (at the end,
  the line break); `Ctrl+J` joins the next line with one space, dropping
  its leading whitespace. Each is one undoable edit.

**Evidence** (`docs/handoff/assets/quil-line-home-end-kill-join-2026-11-05/`):
host test `home_and_end_stay_on_the_carets_line`; Quil clipboard,
service-custody and sexnet host gates pass; the Python gate suite gains no
new failures; sealed build and master gate GREEN; native
Quil autosave gate at both sizes (Home `old=31 new=17`, `[quil.line.kill]
start=17 removed=14`, `[quil.line.join] start=1 removed=1`); native
connected desktop gate at 1280x800.

## Q7. Script `arguments`, `call`, `apply`, `bind` (2026-11-05)

- `arguments` gives the running call's arguments as a new array (so it is
  a copy: writing to it does not change parameters). `f.call(this, ...)`,
  `f.apply(this, [...])`, and `f.bind(this, ...)` (64 per script; a bound
  `this` cannot be replaced, as in JavaScript).
- **Fix:** a callback that was not a plain function value during the
  parse-only pass of a function body (`xs.map(fn.bind(this))`) refused the
  whole script; `map`-style methods and `sort` now check only when live.

**Evidence** (`docs/handoff/assets/script-arguments-call-apply-bind-2026-11-05/`):
script host tests 51 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`big 79800 40 521 1232 27`).

## Q8-Q9. Script prototypes, comma operator, `console`, small builtins (2026-11-05)

- **Q8 Prototypes:** `F.prototype.m = ...`, `F.prototype = {...}`,
  `F.prototype` reads, `Object.create(proto)`, and the
  `P.call(this, x)` + `C.prototype = Object.create(P.prototype)` pattern.
  This is a snapshot, not a chain: an instance copies its prototype when
  constructed, so later additions do not reach existing instances, and
  `instanceof` does not walk from C to P. Copied properties are not listed
  by `Object.keys`/`JSON.stringify`.
- **Q9:** the comma operator (`(a, b)`, `a = 1, b = 2;`, `for (...; i++, j--)`;
  `for` updates are now any expression, so `++i` works); `console.log/info/
  warn/error/debug` evaluate their arguments and write nothing (they used
  to refuse the whole script); `substr`, `isFinite`, `Number.parseFloat/
  parseInt`, `Object.defineProperty` (`value`/`get`/`set` only),
  `Object.getOwnPropertyNames`, `Array(n)` and `Error(m)` without `new`.

**Evidence** (`docs/handoff/assets/script-prototypes-comma-console-2026-11-05/`):
script host tests 53 groups; sealed build and master gate GREEN; native
HTML gate at both sizes (`... 1232 27 82`), floor hits 1, high water 56,592.

## Q10. Regression on the head of the Q batch; P1 test debt paid (2026-11-05)

- **Correction:** P1 grew the Browser heap and updated
  `browser_reserve_gate.py`, but not its unit fixtures: 7 tests in
  `test_browser_reserve_gate.py` had failed since then, and P1/P10 did not
  run the Python suite. Q1 and Q5 described them as pre-existing without
  saying they came from P1. The fixtures now use the 128 KiB heap and the
  768-byte baseline. The full Python gate suite passes: 1,132 tests.
- All 11 host gates pass. Native gates on the sealed image at the batch's
  head: HTML, connected desktop and Spindle calc at 1024x768; HTTPS
  document, asset, PNG, Browser reserve, Quil autosave and Quil delete at
  1280x800. All pass.

Evidence: `docs/handoff/assets/batch-regression-2026-11-05/`.

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

## Full-window text (2026-11-06)

Approved ABI change (Andreas, in thread): `text_batch::CAPACITY` 512 to 6144;
`document_view` limits 79 to 110 columns and 24 to 48 rows; abi hash updated.
The wire format is unchanged.

- **Browser** reader fills its window: 104 columns and 39 rows at maximized
  1280x800 (36 body rows), 83 columns at 1024x768. Before: 47 x 10.
- **Quil** paints its whole tile. The leftover title bar and indicator rects
  that drew over the status row are gone.
- **Typing cost.** A keystroke in a 3.5 KB Quil document on QEMU: 0.089 s on
  the old image, 0.25 s with capacity raised, 0.035 s median after the fix
  (`changed_text_rows` in Display repaints only rows whose bytes differ).
  Chunk pacing and serial logging were measured and were not the cause. A change
  touching every row (scrolling) still repaints the body, about 170 ms for a
  half-window tile; the per-pixel compositor is the next cost to cut.
- **Delivery.** Quil sends up to 128 chunks a pass and re-sends a batch after
  4096 polls with no reply. Display refuses a Begin while it owes a reply, and
  the slower pacing used to hide that loss.
- **Gates.** 14 host gates and 1,132 Python tests pass. Native, source-bound:
  master gate GREEN; `http_content`, `quil_autosave`, `clipboard_workflow`,
  `quil_readability`, `https_document` at 1280x800 and 1024x768;
  `connected_desktop`, `quil_workflow`, `https_import`, `spindle_calc` at 1280x800.
  Gates now park the pointer on the frame rim, because the larger text reaches
  where it used to sit.
- **Not green, not caused by this:** `quil_cursor_gate.sh` fails the same rows
  on the Q10 image (insert_mid_line, delete_at_cursor, backspace_joins_lines,
  dirty_during_edit, exact_final_content). `quil_editor_gate.sh` also fails
  (quil_dirty_tracking) but was not compared with Q10, and `quil_viewport_gate.sh`
  was stopped after 7 minutes with no result. These older gates are outside the
  native regression set and need review.
- **Still open:** Spindle remains four panes of 20 columns; g, p, q, y have no
  descenders. Screenshot: `assets/sexos-alpha-full-window-reader-2026-11-06.png`.

## Spindle full-window terminal (2026-11-06)

ABI input changed under Andreas's direction to build it as the docs require:
`spindle_view` layout and limits. The catalog still reserves four Spindle
surfaces; the three children stay at zero height and hold no text.

- One terminal pane fills the frame: 103 x 38 maximized at 1280x800, 81 x 37 at
  1024x768, 49 x 17 in a half-window tile. Before: 40 x 12 split across four
  panes, so sentences broke at the pane edges. Scrollback lines hold 110 bytes.
- Text goes through the atomic batch and waits for Applied, so partial frames no
  longer show. Atomic outboxes re-send a batch after 4096 polls with no reply.
- Native: master gate GREEN; `connected_desktop`, `spindle_calc`,
  `network_contention`, `spindle_reserve_probe`, `https_document`. Maximized
  Spindle at both sizes passes the `check_spindle_viewport_pixels` oracle using
  a local script, not a committed gate.
- Typing cost: 0.22 s per key maximized on QEMU at first, 0.057 s after the two
  changes in the next entry.
- Not green: `launcher_workflow_gate.py` stops at launcher-exhausted because it
  expects Spindle to be out of reopens while 7 remain; the gate predates the
  larger reserve but was not re-run on an older image. `spindle_viewport_gate.py`
  needs a journal disk from an earlier boot and has not run here.
  `desktop_close_gate.py` times out waiting for `[quil.palette.action] kind=esc`, the same on the Q10 image.
- Screenshot: `assets/sexos-alpha-spindle-full-window-2026-11-06.png`.

## Delta text batches and the hidden terminal (2026-11-06)

- **New ABI op** `text_batch::BEGIN_DELTA` (0x28c). Display seeds the Stage from
  the surface's committed text, only changed chunks follow, and Commit still
  verifies the hash of the whole text. A mismatch poisons that sequence, so the
  sender re-sends complete under a new number after 4096 polls with no reply.
  Quil, Browser and Spindle use it.
- **Removed** the shell's `spindle_render`/`spindle_render_cmdline`: a dead copy
  of the terminal behind Spindle that repainted the whole window on every key.
  Measured: maximized Spindle typing 0.207 s with deltas alone, 0.057 s after
  this removal, at both 1280x800 and 1024x768.
- **Gates.** 15 host gates, 1,132 Python tests, master gate GREEN. Native,
  source-bound, all pass: `http_content`, `quil_autosave`, `clipboard_workflow`
  at both sizes; `quil_readability`, `connected_desktop`, `quil_workflow`,
  `https_document`, `https_import`, `spindle_calc`, `network_contention`,
  `spindle_reserve_probe`.
- **Correction.** The Spindle commit was merged with `display_surface_groups_gate`
  and `linen_text_delivery_gate` failing, because I had run only part of the host
  set. Their tests still assumed four panes. This merge fixes them.

## Run-based compositor (2026-11-07)

Display paints each row in runs: `raster_row` fills a run of plain content and
draws its glyphs directly, and falls back to `composite_pixel_source` for chrome,
bitmaps and fill rects that cover part of a run. Maximized repaint on QEMU: about
320 ticks before, 20-50 after (strip repaints are unchanged at about 130).
`fast_rows_match_the_pixel_compositor_exactly` holds both paths equal, including
present bits, across selection, Unicode, wrap, fill rects, clipping and damage.
All host gates and Python tests pass. Native at 1280x800: `http_content`,
`quil_autosave`, `clipboard_workflow`, `quil_readability`, `connected_desktop`,
`quil_workflow`, `https_document`, `spindle_calc`, `spindle_reserve_probe`,
`https_png`; `http_content` and `clipboard_workflow` also at 1024x768.
Also fills the band above Spindle's terminal pane.

## Shared text regions (2026-11-07)

ABI: `text_batch::FILL` (0x28c) replaces `BEGIN_DELTA`; new `text_region` module;
new BOOT grants; `BootFrameReservations` holds 96 ranges, up from 32.

Each catalog row has an 8 KiB region granted at BOOT, writable by that app and
read-only to Display. A batch is Begin, Fill, optional Select, Commit: 3 or 4
calls instead of about 490 for a full screen. Display copies the region once and
hashes its copy, so the app cannot change a text after Fill. Quil, Browser and
Spindle use it; the shell's small surfaces keep chunks. This is the UiIR idea from
ARCHITECTURE.md 1.4 applied to app text, and it removes the delta code.

Measured on QEMU: Quil 0.037 s per key, maximized Spindle 0.063 s, both about the
same as with deltas. Region delivery mainly removes the worst case, a text whose
every chunk changes. Display logs `[display.text.incomplete]` a few times at boot
when a paint is refused while a reply is owed; each recovers by re-sending.

All host gates, 1,132 Python tests, `memory_region_authority_gate`, master gate
GREEN. Native: `http_content` and `clipboard_workflow` at both sizes;
`quil_autosave`, `quil_readability`, `connected_desktop`, `quil_workflow`,
`https_document`, `spindle_calc`, `spindle_reserve_probe`, `https_png`,
`network_contention` at 1280x800.

## First program outside the built-in apps (2026-11-07)

`Run Hello` in the launcher starts `apps/hello` in its own domain: a window, text
through its BOOT text region, closed with F11, run again on the next row.

ABI: `App::Program` and four catalog rows (32 rows total); `PROGRAM_KEY`; the
clipboard participant check excludes programs.

- **Limits.** Each run uses up a row, so three runs per boot. Row 41 only anchors
  the chain and exits at BOOT. The image is a Limine module, not a file from
  storage. Program rows run one at a time.
- **Interim isolation change.** sexusb shares sexinput's PKU key, so a bug in
  either can reach the other's memory. This frees key 5 for programs. Capabilities
  still decide access. It becomes the Senses organ under the organ design Andreas
  agreed: keys map to organ systems, with few vital parts and restartable services.
- **Found on the way.** F11 closes a window; Spindle's help says Alt+F4, which does
  not close it. `shell.tile.apply` had a log budget of 24 that gates rely on as
  geometry, raised to 64; gates reading budgeted logs as truth remain a risk.
- **Gates.** New `program_launch_gate.py`, native at both sizes. All host gates and
  Python tests pass. On the final commit: master gate GREEN, `connected_desktop`,
  `network_contention`, `spindle_reserve_probe`. One commit earlier, with only the
  log budget and hello's text differing: `http_content`, `quil_autosave`,
  `clipboard_workflow`, `quil_readability`, `quil_workflow`, `https_document`,
  `spindle_calc`, `https_png`, at 1280x800.

## Idle cost and active-domain measurement (2026-11-07)

- `[sched.active]` reports, per 1000 ticks, how many distinct domains ran, the
  peak within 10 ticks, the switch count and which domains. On QEMU with 44
  domains registered: 5 running at idle, 7-8 peak while typing. This is today's
  small desktop, not yet many tabs or a music player.
- Display, sexinput, sexusb and SilkBar sleep when idle instead of spinning on
  yield: about 19,000 switches per second at idle before, about 2,700 after.
  sexinput still wakes about 2,500 times a second to poll; interrupts would end that.
  Input now has up to one tick of added latency. Maximized Spindle typing is 0.066 s.
- **Gate repair and correction.** Running every non-QEMU gate script showed that
  my usual list had been too small: `desktop_content`, `silkbar_civil_clock`,
  `sex_rt_heap`, `launcher`, `app_catalog`, `app_resources`,
  `display_surface_lifecycle` failed on stale expectations, some on master since
  earlier merges in this work. All are fixed. No OS behaviour was wrong.
- **Still failing, not investigated further:** `gate_no_alpha`, `gate_render`,
  `audit_invariant_gates`, `daily_driver_master_gate` and these runtime gates:
  `app_resource`, `desktop_close`, `memory_authority`, `paging_recovery`,
  `scheduler_ownership`, `task_exit_cleanup`. Most are older than this work, and
  `desktop_close` fails the same on the Q10 image; the rest were not compared. Four
  others only need arguments or a log file. They should be repaired or retired.
- Native, final commit: master gate GREEN; `connected_desktop`, `https_document`
  at both sizes, `clipboard_workflow` at 1024x768. One commit earlier, with only
  the timed-wait checker differing: `program_launch`, `http_content`,
  `quil_autosave`, `clipboard_workflow`, `quil_workflow`, `spindle_calc`,
  `spindle_reserve_probe`, `network_contention` at 1280x800.

## Reusable program rows (2026-11-07)

`Run Hello` no longer runs out. A closed program's row runs again under a new task.

ABI: `OP_APP_REBORN` (0x10F), activation kind 2, activation states 4 (reusable) and
5 (rebirthing).

- **Kernel.** Grants keep naming the row's boot task; `App.task` is the current run.
  Program rows keep their capabilities and frames at retirement, and their frames
  are zeroed. `rebirth` restores the image from a pristine copy taken at BOOT, rearms
  the same Task with a task id never used before, and tells every service. The task
  starts only after all five have forgotten the old run. Capabilities are not minted.
- **Services.** `ServiceCustody::reborn` lifts its fence only for the task the held
  grant now reports; Display's `Lifecycle::reborn` only from Acknowledged. Shell
  resets its custody entry, lifecycle record and tombstone for that frame.
- **Evidence.** 100 launch-and-close cycles in one boot on QEMU, 97 on one row and
  each with a new task id, no panic. `program_launch_gate.py --runs 8` also checks
  text pixels each run. Host tests cover forged notices, an old run's replayed exit,
  and a new run's own death. No test yet sends a live message from an old run's task
  on a booted system. Rebirth allocates nothing by construction, but kernel memory
  use was not measured across the 100 runs.
- **Limits.** Programs still run one at a time, on one shared key. The image is still
  a Limine module holding hello. Rows of Quil, Spindle and Browser still use fixed
  reopen chains. Net and Files reset only their custody fence on rebirth; programs
  hold no network or document routes today, so their per-target state is untested
  across a rebirth.
- **Log fix.** `spindle_calc` failed about half the time because `[spindle.complete]`
  was printed in several pieces and other output could land inside it. It is now
  one record. Other `serial_println!` receipts have the same risk.
- **Gates.** Every non-QEMU gate except the older ones listed above, 1,132 Python
  tests, `memory_region_authority_gate`, master gate GREEN. Native, final commit:
  `program_launch` (8 runs), `spindle_calc` twice, `connected_desktop`. Two commits
  earlier, differing by that log line and the move of two frame helpers:
  `program_launch` at 1024x768, `http_content`, `quil_autosave`,
  `clipboard_workflow`, `quil_readability`, `quil_workflow`, `https_document`,
  `spindle_reserve_probe`, `https_png`, `network_contention` at 1280x800.

## Programs side by side, first organs (2026-11-07)

Three programs run at once, each on its own hardware key.

ABI: new `organs` module; `app_catalog::PROGRAM_KEY` removed. Activation of a program
row no longer waits for the row before it to end.

- **Organs (`organs.rs`).** Senses: sexusb with sexinput. Core: silkbar with
  sexdisplay. Storage: sexstore with sexfiles. **These pairs can now read each
  other's memory;** capabilities still decide what they may ask for. Keys 5, 6 and 8
  go to program rows. Other services keep their own keys, since nothing needs them yet.
- **Isolation evidence.** `program_isolation_probe.py` on an ISO built with
  `SEXOS_PROGRAM_ISOLATION_PROBE=1`: row 43 reads its own first page, then row 42's.
  The second read takes `[paging.fault.trap] pd=43 addr=0x69000000 error=0x25`
  (protection-key bit set), the kernel ends only row 43, its cleanup completes, row
  42 stays live and closes normally, and another program starts. This covers one
  read in one direction, and it is not part of the normal gate run because it needs
  that build. The shipped hello has no probe code.
- **Limits.** Three programs at once; a fourth launch focuses a live one. All are
  hello from a Limine module. More than four windows stack as thin rows and only the
  focused window shows a title bar, so this desktop layout will not scale. Calls
  between organ members still enter the kernel, so organs give no speed gain yet.
- **Gates.** All non-QEMU gates except the older ones listed earlier, 1,132 Python
  tests, master gate GREEN. Native on the final commit: `program_launch` at both
  sizes with three live programs checked by pixels; `http_content`, `quil_autosave`,
  `clipboard_workflow`, `quil_readability`, `connected_desktop`, `quil_workflow`,
  `https_document`, `spindle_calc`, `spindle_reserve_probe`, `https_png`,
  `network_contention` at 1280x800.
- Screenshot: `assets/sexos-alpha-programs-side-by-side-2026-11-07.png`.

## Programs from disk (2026-11-07)

- **What runs now.** `run fortune` in Spindle starts a program whose file is on the
  disk and in no boot image or Limine module. Spindle asks Shell, Shell asks Files,
  Files reads the store at LBA 2048 into a BOOT region, and the kernel checks the
  file and lays it out on a free program row. `scripts/install_program.py` writes
  the store from the host.
- **Authority.** A name selects bytes and grants nothing. The program holds exactly
  what its BOOT row holds. BOOT-only genesis is unchanged.
- **Checks.** `kernel/src/program_image.rs` accepts only x86-64 position-independent
  ELF, no segment both writable and executable, no two segments sharing a page, all
  inside a 1 MiB window, entry in code, relative relocations only. Host test: every
  truncation and 200,000 corrupted files, with overflow checks on. Natively, a
  missing name, a bad hash, writable code and a disk with no store are each refused
  by a plain denial with the system still working.
- **Clean start.** Stack and image frames are zeroed before each load, and the boot
  image's hello still runs on a row a disk program used.
- **ABI.** New `program_store` protocol (ops 0x4E, 0x4F, Files image slot 100, Shell
  route slot 256), and activation kind 3 with a length in rdx.
- **Limits.** Still three programs at once, on rows from the BOOT catalog of at most
  64. Each row reserves 1 MiB of frames at boot, which is not how thousands will
  work. The file hash is FNV: it catches a damaged disk, not an attacker, and there
  is no signing. The store is a one-sector directory of 15 names, one file at most
  1 MiB, installed from the host only, with no `programs` listing yet. The whole
  DiskFS is 1 MiB (`DISK_TOTAL_SECTORS=2048`), far below a real filesystem. Program
  windows stack as thin rows with no title bar.
- **Gates.** All non-QEMU gates except the older ones listed earlier, 1,132 Python
  tests, master gate GREEN on its second attempt (attempt one reported `missing
  init` in BOOTGRAPH_GATE; cause not investigated). Native: `program_disk` and
  `program_launch` at both sizes; `http_content`, `quil_autosave`,
  `clipboard_workflow`, `quil_readability`, `connected_desktop`, `quil_workflow`,
  `https_document`, `spindle_calc`, `spindle_reserve_probe`, `https_png`,
  `network_contention` at 1280x800.
- Screenshot: `assets/sexos-alpha-programs-from-disk-2026-11-07.png`.

## Thousands of programs alive (2026-11-07)

- **What runs now.** `spawn hum 4000` in Spindle starts 4,000 programs from one file
  on disk, all alive at once, on one core of a 4 GiB QEMU machine. `records` counts
  them; `stopall` asks them to end and `stopall force` ends them regardless.
- **Design.** BOOT makes program records, up to 4,096 and sized by RAM (16 below
  about 768 MiB), from one template: a domain, a task and a page table each, sharing
  one empty sealed capability table and pair of rings. A load takes memory from a
  pool and a fresh task id. BOOT stays the only capability genesis. Records share
  key 10, and only the running record's pages are present.
- **Measured, 4,000 programs, QEMU emulation, 60 s hold.** 44 KiB of program memory
  each, plus about 19 KiB of kernel memory reserved per record whether used or not.
  21.8 s to start them, in small batches. 4,000 idle programs caused 0 record switches
  in 60 s, and typing took 45 ms (35 ms with none). 4,000 programs that wake about
  every 32 ticks took typing to 422 ms median, 815 ms worst. A switch between records
  averaged 6,468 TSC cycles under emulation, which says little about real hardware.
  Deepest record kernel stack 1,904 bytes of 16 KiB, across all runs.
- **Isolation.** `snoop` reads the next program's memory: it is stopped by a page
  fault on that read, alone, and the others stay alive. `hum` checks its memory on
  every wake and no corruption appeared. `spin` never yields and ignores requests;
  `stopall force` still ends it. All memory returned to the pool after every round.
- **Bugs this found.** The NVMe driver relied on firmware to enable DMA and set queue
  entry sizes, which QEMU's firmware does at 512 MiB and not at 4 GiB, so the disk
  failed on the larger machine. Creating I/O queues gave up after a fixed poll count,
  so a boot could occasionally lose its disk, and the master gate passed such a boot.
  All three are fixed, and the gate now fails on a queue setup error. The kernel also
  now checks program files in a private copy, so Files can't change one after the check.
- **ABI.** Syscall 79, `RecordPool` capability, `program_store` record operations.
  Domain space 8,192. Reply bookkeeping is a table of 128 distinct callers per domain,
  and a domain past that refuses new callers without losing any.
- **Organs.** sexbell joined the senses organ on key 4, giving up isolation from
  sexinput and sexusb, to free key 10.
- **Limits.** These programs hold no routes: no windows, files or network, so they
  can compute and sleep and nothing else. Windowed programs are still three at once.
  There's no scheduling priority, so busy programs slow the desktop, and one spinning
  program roughly triples typing latency. The capacity is fixed at boot, and the
  kernel memory is spent whether records are used or not. All numbers are on QEMU;
  none of this has run on real hardware. One core only: `kernel/src/smp.rs` parks the
  others, and record key sharing assumes one core.
- **Gates.** All non-QEMU gates except the older ones listed earlier, 1,132 Python
  tests, master gate GREEN. Native on the final commit: `program_records` at 1,000 and
  4,000; `program_disk` and `program_launch` at both sizes; `quil_readability`,
  `connected_desktop`, `quil_autosave`, `quil_workflow`, `https_document`,
  `http_content`, `clipboard_workflow`, `spindle_calc` at 1280x800, and
  `spindle_reserve_probe`, `https_png`, `network_contention`.

## More than one core, first stage (2026-11-07)

- **What runs now.** Every core starts, with its own GDT, TSS, double-fault stack, idle
  stack and CoreLocal. Before this, the other cores were never started at all. TLB
  shootdowns reach every core by IPI, and page retirement refuses to run if a started
  core isn't covered. Core 1 schedules for real, with its own timer, and runs program
  records. Core 0 keeps the clock and runs every service. Any further cores idle.
- **Measured, 4,000 busy programs, QEMU emulation.** One core against two: typing
  755 ms against 37 ms median, 2,483 ms against 64 ms worst; start 27.8 s against
  3.6 s. With one spinning program too: 814 ms against 41 ms. Idle cost unchanged.
- **Honest scope.** This is isolation of load, not parallel speed for one program: the
  whole desktop still shares core 0, and records share core 1 at one at a time, since
  they share a key and a present page is visible to any core holding that key. Tasks
  do not move between cores, so work stealing is off. Core 2 and up do nothing but
  answer shootdowns. Device interrupts all go to core 0. All on QEMU multi-threaded
  emulation, whose memory ordering is not a real x86 chip's, and never real hardware.
- **Bugs found.** A dead task with nothing to replace it held its core and memory for
  good, now fixed with a per-core idle stack. The kernel's repeated exit hints could
  fill Shell's startup inbox and crash it if Display started slowly, now deduplicated.
  The double-fault IST entry the IDT names was never set on the boot core, which is
  still true there: a double fault on core 0 would not reach its handler cleanly.
- **Gates.** New `smp_gate` at 4 cores. `program_records` at 1, 2 and 4 cores,
  `program_launch` and `program_disk` at 1 and 2. Full native regression at 1 core on
  a94e2fe7; on the final commit, which differs only by two small corrections, master,
  `program_launch`, `program_disk`, `quil_workflow`, `smp_gate`, `program_records`. All
  non-QEMU gates except the older ones listed earlier, 1,132 Python tests.

## Services across cores (2026-11-07)

- **What runs now.** On 4 or more cores, the network stack, Files, Storage, the disk
  driver, Quil, Spindle, Browser and every reserved incarnation of them run on cores 2
  and up, spread across what's there. Shell, Display, SilkBar, input and the rest stay
  on core 0, records stay on core 1. On 1 or 2 cores nothing moves off core 0 but records.
  Every core has a timer, and an IPI wakes a halted core when work arrives for it.
- **Bugs this found and fixed.** Serial lines from different cores interleaved, which
  also broke every log-based gate. Kernel lifecycle and paging locks gave up at once on
  contention, and a service read that as a denied caller. Those locks now wait, and answer
  shootdowns while waiting, since the holder may be waiting on that core. A waker's cancel
  could land just before a task armed its next wait, which then got refused and crashed
  sexnet. The boot core's double-fault stack was never set, now fixed.
- **Honest scope.** Tasks have a fixed home core and do not migrate or balance. Shell and
  Display still share core 0, so the desktop's own hot path gains nothing yet. Device
  interrupts all go to core 0. A real speedup hasn't been measured: QEMU multi-threaded
  emulation on an ARM host says little about real x86, and has never run on real hardware.
  Locks still spin with interrupts off, which is fine for this load but not a design that
  scales to many cores.
- **Gates.** On the final kernel: `https_document`, `network_contention` and
  `http_content` five times each at 4 cores, and `https_document`, `network_contention`,
  `connected_desktop`, `quil_autosave` at 2 and 8. One commit earlier, at 2, 3, 4 and 8
  cores: `smp_gate`, `http_content`, `https_document`, `network_contention`,
  `connected_desktop`, `program_disk`, `program_launch`, `quil_autosave`, `quil_workflow`,
  `clipboard_workflow`, where a failure led to the wake fix above. `program_records` with
  4,000 at 4 cores, typing 32 ms median while loaded. One-core regression, all non-QEMU
  gates except the older ones listed earlier, and 1,132 Python tests pass.

## A real filesystem (2026-11-07)

- **What runs now.** On a disk over about 192 MiB, Files formats a filesystem once and
  mounts it afterward. Spindle has `ls`, `cd`, `pwd`, `mkdir`, `cat`, `write`, `append`,
  `touch`, `rm`, `mv`, `df`. Tested with a 1 GiB disk: 960 MiB usable, directories, and
  files that survive a reboot and an abrupt QEMU stop.
- **Design (`crates/sexfs`).** Copy-on-write: one checksummed B+tree holds inodes, directory
  entries and a block map with a checksum per data block. Nothing committed is overwritten.
  A commit writes new blocks, flushes, then writes one of two superblocks and flushes. Free
  space is not stored: mounting walks the tree, so it can't leak or disagree.
- **Evidence.** Host: a random model across eight seeds and remounts, 400 power cuts at any
  write or flush with torn sectors, every single-block bit flip, a full disk, and three
  mutants of the commit ordering that the power-cut test must catch. Native at 1 and 4 cores.
- **Disk driver.** It asks the namespace its size and no longer refuses writes past 1 MiB.
- **ABI.** `BLOCK_CAPACITY`, the `fs` protocol, Spindle slot 258, Files slots 300 and up.
- **Limits.** Only Spindle can use it. Quil and Linen still use the 15-slot DiskFS. No
  timestamps, permissions or per-client roots yet: any Spindle reads and writes everything.
  16 GiB is the most it will use of any disk, and mounting takes longer as data grows. Each
  operation blocks Files, so one big write stalls every Files client. The native stop kills
  QEMU, which keeps what it already wrote, so only the host tests model a true power cut.
  The fixed 64 MiB program store sits in front of it and should move into it.
- **Gates.** `sexfs_host`, `filesystem` at 1 and 4 cores (three repeats at 4), full one-core
  regression and 4-core subset one commit earlier, all non-QEMU gates except older listed ones.

## The real internet (2026-11-07)

- **What runs now.** `fetch URL FILE` in Spindle downloads over HTTP or HTTPS into the
  filesystem. In QEMU user networking it got an address by DHCP, used the leased resolver,
  and saved `https://www.wikipedia.org/` (122 KB), `https://github.com/` (525 KB) and
  `https://www.cloudflare.com/` (1.2 MB). A 300 KB local download took about 11 s, emulated.
- **Trust.** TLS 1.3 with the bundled public root store: full chain, host name and date,
  RSA PKCS#1 and PSS, ECDSA P-256 and P-384, and the server's handshake signature. A
  self-signed local server is refused. It lives in the network service, so all programs will
  share one checked implementation. The service therefore sees plaintext.
- **Design.** A second, open side of the network service, started the first time something
  asks, leaving the fixed controlled paths and their exact logs alone. An address is data:
  the right to use the network is the BOOT route and a BOOT region shared with the service.
- **ABI.** The `web` protocol (0x211), Spindle slot 259, network slots 340 and up. `sexnet`
  now has a heap and rebuilds with the scalar crypto flags, growing from 237 KB to 830 KB.
- **Limits.** One flow at a time across the machine. Spindle only: the Browser still reaches
  just its fixed test endpoint, so it can't open a real site yet. `fetch` doesn't follow
  redirects or check Content-Length. A TLS 1.3 server that won't use AES-128-GCM with P-256
  fails, and TLS 1.2-only servers fail. No revocation checks. The root store is compiled in.
  Randomness and time use a BOOT grant labelled for the Browser, until the service has its
  own. Throughput is capped near one frame per service turn. IPv4 only. Only tested against
  QEMU's user network, never a real network card or LAN.
- **Gates.** `fetch` at 1 and 4 cores against public sites, plus full one-core regression and
  a 4-core subset on the same kernel. All non-QEMU gates except the older ones listed earlier.

## The Browser opens real websites (2026-11-07)

- **What runs now.** Ctrl+L opens an address bar. Type a URL and Enter, or a bare name for
  HTTPS. Type a link's number and Enter to follow it, Backspace goes back, Esc closes. Up to
  five redirects. In QEMU user networking it showed `https://example.com/`,
  `https://www.rust-lang.org/` (after its 301), and Wikipedia's Microkernel article (278 KB
  of source, 49 KB of text, 1,339 rows). A self-signed site is refused with a message.
- **Design.** The same open side of the network service that `fetch` uses, so there is still
  one checked TLS. The Browser gets its own BOOT route and shared region, paired by plan.
- **ABI.** Browser rows in the `web` slots of `kernel/src/boot_memory.rs`, which changes the
  hash. The Shell's Browser key queue grew from 8 to 32 and now passes every typing key.
- **Limits.** A text reader, not a browser engine: no layout, CSS, images, forms, cookies or
  JavaScript. Menus and page furniture show up as text. 512 KiB of source and 128 KiB of text,
  and only the first 128 links of a page are numbered. HTTP/1.0. One flow at a time across the
  machine. Keys can still be dropped past 32 queued ones; the gate types at about 60 ms a key.
  All the `fetch` network limits apply.
- **Gates.** `browser_web` at 1 and 4 cores (public sites at 1), `fetch`, `filesystem`, `smp`,
  program records and program disk at 1 core on the final kernel. Python tests and all
  non-QEMU gates except the older ones listed earlier. Not the full native suite.

## Programs are files (2026-11-07)

- **What runs now.** `run NAME` looks for `/bin/NAME` in the filesystem, then in the older
  disk store. `run PATH` (anything with a slash, relative or absolute) runs that file, and
  `spawn` takes either form for background programs. A program installs with
  `fetch URL /bin/NAME`. A new filesystem starts with `/bin`. Programs survive a reboot.
- **Answers.** Shell now tells Spindle why a program didn't start: not found, over 1 MiB,
  disk error, damaged, not a program SexOS can run, all program windows in use, or no
  memory for more background programs. The path errors (not found, is a directory) come
  straight from Files.
- **Design.** A path never reaches Shell. Spindle asks Files to `PREPARE` a path; Files
  checks it and answers with a one-use ticket, which Spindle hands to Shell in place of a
  name. A name or ticket still selects data only: the program runs with exactly what its
  BOOT row holds, and the kernel checks every file before it touches a page.
- **ABI.** `fs::PREPARE` (12), `program_store::OP_REFUSED` (0x53), the `TICKET` name word,
  and refusal codes 5 to 7.
- **Limits.** Still three windowed rows and, at the default 512 MiB, 16 background records.
  Programs get no arguments, no environment and no working directory. Files holds one
  prepared path at a time. No signatures or install manifest: anything that is a valid
  program file runs. The older store stays as a fallback until the gates stop using it.
- **Gates.** New `program_file` at 1 and 4 cores (download, by name, by path, relative,
  refusals, background programs, reboot). Program disk, launch and records, filesystem,
  fetch, browser web and SMP at 1 core on the final kernel. Python tests and all non-QEMU
  gates except the older ones listed earlier.

## Eight program windows at once (2026-11-08)

- **What runs now.** Eight programs open side by side, up from three, tiled in a grid
  that stays near square once there are five or more windows (it used to stack thin rows).
  A program that ran before gets its row back first, from `run` and from the palette.
- **Why it was three.** Each windowed program had its own PKU key, and x86 has sixteen,
  nearly all held by services. That, not any table, was the ceiling.
- **Design.** Rows from the fourth on share one key (`organs::SHARED_PROGRAM_KEY`) and take
  turns being present (`kernel/src/program_tenancy.rs`), as background records already do.
  Before a shared row's task runs, the kernel clears the present bit on the last tenant's
  pages and sets it on the new one's, compare-and-swap so a leaf cleared by a retire stays
  cleared. Restores write shared rows' leaves absent, and checks count an evicted page as
  the row's own. All program rows already run on core 0, which is what makes one set of
  present bits enough in one address space.
- **Isolation evidence.** `program_isolation_probe.py` on an ISO built with
  `SEXOS_PROGRAM_ISOLATION_PROBE=4`, fourth argument `4`: row 45 reads its own first page,
  then live row 44's, which shares its key. The read takes `[paging.fault.trap] pd=45
  addr=0x6b000000 error=0x4` (not present), only row 45 ends, and the next program starts.
- **ABI.** Five catalog rows (targets 45 to 49, surfaces 362 to 366, Shell slots 420 to
  439), so `app_catalog::COUNT` is 37. Display holds 32 surfaces, Shell 24 frames.
- **Limits.** Shared-key programs never run in parallel with each other. The catalog's
  64 presentation bits cap rows. Records and windowed programs are still separate kinds.
- **Title bars (same day).** A program's window kept its title bar only while focused,
  since a program row's own surface is its frame and Display only drew chrome for Shell
  frames or the focused surface. Display now treats a program row's surface as a frame,
  so every open program shows its bar. The title still says "Program", not its name.
- **Gates.** Program disk (eight at once) at 1 and 4 cores, program launch, program file,
  records, filesystem, fetch, browser web and SMP at 1 core on the final kernel; the probe
  above; Python tests and all non-QEMU gates except the older ones listed earlier.

## Programs get keys, files and an SDK (2026-11-08)

- **What runs now.** `crates/sex-app` is the first program SDK: `Window::open`, `show`,
  `next` (keys and close) and `files::{read, write}`. `apps/jot` is a notepad built on it.
  Installed with `fetch URL /bin/jot` and started with `run jot`, it takes typing, saves
  with Ctrl+S to `notes.txt`, and finds the note again after a reboot. Spindle reads the
  same file at `/data/jot/notes.txt`.
- **Keys.** Shell now sends typed keys to a focused program row on its close route, as it
  does for Quil and Browser. Programs take no part in the clipboard.
- **Files, confined.** Each program row has a file region shared with Files alone. A run
  started from a file is bound by Shell (`OP_BIND`) to `/data/NAME`, NAME being the file's
  base name; Files makes that folder and roots every path there, both halves of a rename
  too, and refuses `..`. The binding names the run's task, and Files checks it against the
  kernel's lifecycle catalog, so a later run on the same row starts with nothing until it
  is bound itself. The boot image's own program, never bound, has no files.
- **A gap closed.** Regions a shared-key row holds (its text region, now its file region)
  were mapped on the shared key and always present, so one shared-key program could have
  reached another's text region. They now take turns with the row's own pages.
- **ABI.** `program_store::OP_BIND` (0x55) and `DATA`, `fs::PROGRAM_INDEX_BASE`, file
  regions and provider slots 309 to 317 for program rows, and the `sex-app` and `jot` crates.
- **Limits.** One folder per program name, so two copies of a program share it. No file
  picker or way to grant a program another folder. A file over about 63 KiB can't be
  written in one go by the SDK. Programs still get no network and no arguments, and the
  title bar still says "Program".
- **Gates.** New `program_jot` at 1 and 4 cores. Program disk, launch, file, records,
  filesystem, fetch, browser web and SMP at 1 core on the final kernel, and the shared-key
  isolation probe again. Python tests and all non-QEMU gates except the older ones.

## The network service comes back after a crash (2026-11-08)

- **What runs now.** If the network service dies, from a bad pointer or anything else that
  ends its task, the kernel starts it again within a tick. In the probe below it crashed in
  the middle of a Browser request, came back, reset the network card, got a new DHCP lease
  and served the next page. Nothing else restarted, and the desktop stayed up.
- **Design (`kernel/src/service_restart.rs`).** At BOOT, before the service first runs, the
  kernel copies every page it owns: code and data in its 16 MiB slot, and its 64 KiB stack.
  When its task is dead, the kernel stops DMA on every PCI device it holds (clears bus
  mastering), puts the pages back, drops the old run's queued messages, signals, replies
  and reply credits, and rearms the same task at the same entry. The domain, its key and
  every capability stay exactly as BOOT made them; nothing is minted. The heap lives in the
  ELF's own pages, so it comes back clean too. The new run finds the card idle and proves
  DMA custody again, as at BOOT, instead of trusting a controller someone else left running.
- **Evidence.** `browser_web_gate.py --crash` on an ISO built with `SEXOS_NET_CRASH_PROBE=1`:
  `[sexnet.crash.probe]`, `fault.kill ... pd=13`, `[pci.command.quiesce] pd=13 slot=19
  before=263 after=259`, `[service.restarted] pd=13 task=13 restarts=1 pages=255`, the card
  goes Cold to Quiescent, a new lease, then `[browser.web.page]`. At 1 and 4 cores.
- **ABI.** The network service's panic handler now ends its task instead of spinning.
- **Limits.** Only the network service restarts; Files, Display and Shell don't yet. A
  service that dies eight times stays down. Requests in flight are lost, so the Browser
  waits out its 60 second deadline before saying the site took too long. Each restart
  re-runs DHCP. Restarting doesn't help a bug that the same input triggers again.
- **Gates.** The crash probe above at 1 and 4 cores; browser web, fetch, Jot, program disk,
  SMP, filesystem and records at 1 core on the normal kernel. Python tests and all non-QEMU
  gates except the older ones listed earlier.

## Files comes back after a crash too (2026-11-08)

- **What runs now.** The kernel's restart covers Files as well as the network service. In
  the probe, Spindle saved a file, Files crashed on a request, the kernel restarted it, and
  the next commands read the old file and saved a new one. The filesystem was mounted, not
  formatted again: its commits are atomic, so a fresh run finds the last committed state.
  A program then opened and closed, and the new run gave its cleanup acknowledgement, which
  the kernel retries until every service has answered.
- **Spindle.** A file request that gets no answer for 15 seconds now ends with "The disk
  did not answer. Try again." instead of holding the terminal busy forever.
- **Design.** `service_restart` now keeps up to two services. Files is 486 pages (its block
  cache and free-space bitmaps live in its own pages), restored in one copy.
- **Evidence.** `files_restart_probe.py` on an ISO built with `SEXOS_FILES_CRASH_PROBE=1`, at
  1 and 4 cores: `[files.crash.probe]`, `fault.kill ... pd=11`, `[service.restarted] pd=11`,
  `[spindle.fs.expired]`, `[files.fs.mounted] ... generation=4`, both files read back, and
  `[app.private.retired]` for a program closed afterwards.
- **ABI.** Files' panic handler ends its task.
- **Limits.** A write in flight when Files dies is lost, and anything not yet committed. Quil's
  open documents and a program bound to a folder lose their session state in Files: Quil
  must reopen, and a running program's next file request is refused until it is restarted.
  Display, Shell and the drivers don't restart.
- **Gates.** The probe at 1 and 4 cores; filesystem, Jot, program file, program disk, fetch,
  browser web and SMP at 1 core on the normal kernel. Python tests and all non-QEMU gates
  except the older ones listed earlier.

## Programs get arguments, and the network when the person allows it (2026-11-08)

- **What runs now.** `run NAME ARGS` passes the words after the name to the program, which
  reads them with `sex_app::files::args`. `run +net NAME ...` also lets that run reach the
  network, through `sex_app::net::get` (HTTP and HTTPS, through the network service's one
  checked TLS). `apps/peek` shows a page's text: `run +net peek https://example.com/` showed
  Example Domain's text in its window; `run peek URL` without `+net` is refused.
- **Design.** Every program row now holds the same fixed route to the network service and a
  shared region with it, as Spindle does, made at BOOT. Holding them grants nothing: the
  network service serves a program row only for the run whose task Shell bound there, and
  Shell binds only when the person typed `+net`. Shell got a BOOT route to the network
  service for that binding alone. Arguments travel with the path from Spindle to Files at
  `PREPARE`, and Files hands them to the run it binds.
- **ABI.** `fs::ARGS` (13), `fs::ARGS_MAX`, `program_store::RUN_NETWORK` in `OP_RUN`'s third
  word, `web::SHELL_SLOT` (262) and `web::PROGRAM_INDEX_BASE`, network regions and provider
  slots for program rows, the `peek` crate.
- **Limits.** The permission is per run and all-or-nothing: any host, no prompt from the
  program itself, no record of which programs had it. A bare name with arguments always
  means `/bin/NAME`, not the older store. One network flow at a time across the machine.
  Peek's text is a crude strip of tags.
- **Gates.** New `program_net` at 1 and 4 cores, with example.com over HTTPS at 1. Browser web,
  fetch, filesystem, program disk, records and SMP at 1 core; Jot and program file passed on
  the first build of this change. Python tests and all non-QEMU gates except the older ones.

## Programs name their own windows (2026-11-08)

- **What runs now.** `Window::title` in sex-app puts the program's name in its title bar:
  Jot's window says "Jot" and Peek's "Peek", instead of "Program".
- **Design.** `app_lifecycle::OP_APP_TITLE` (0x2D1) carries sixteen printable bytes. Display
  takes it only from the program row that owns the surface, and forgets it at the row's
  next present, so a new run starts with the catalog's name until it gives its own.
- **Limits.** Sixteen characters. Shell's own view of the window (Atlas, the switcher) still
  knows it only as a program row.
- **Gates.** Jot (checks the title arrives), program net and program disk at 1 core; both
  Display host gates that compile the title path.

## Give a program one file (2026-11-08)

- **What runs now.** `edit todo.txt` opens that file in Jot; typing and Ctrl+S save it, and
  Spindle reads the change back. More generally `run NAME @FILE` gives any program that one
  file, and the program names it as `@/absolute/path`.
- **Design.** Spindle, which holds the person's whole filesystem, is the one that hands out
  files: an `@` word is resolved to an absolute path and travels with the program's
  arguments to Files at `PREPARE`. Files lets the bound run reach exactly the `@` paths in
  its own arguments, besides its folder, and refuses `..` or `.` in them. A program can't
  prepare runs, so it can't hand itself more.
- **Limits.** Whole-file grants only, read and write together. No grant of a folder. The grant
  lasts for the run.
- **Gates.** Jot's gate now edits a file given with `edit`, at 1 core. Program net, program
  file and filesystem on the same build. Python tests and all non-QEMU gates.
- **Jot edits in place (same day).** Arrows, Home, End, Delete and Backspace work at a cursor
  drawn as a bar, and a long note scrolls to keep the cursor on screen. `sex_app::nav` maps
  movement keys whichever keyboard code Shell passed. The gate moves left into `buy milk`,
  inserts `fresh `, jumps to the end and appends, then Spindle reads `buy fresh milk and eggs`.

## SexOS installs itself to a disk (2026-11-08)

- **What runs now.** `install boot.img` makes the disk start SexOS. In the gate, SexOS booted
  from the ISO, wrote a file, fetched a 12 MiB boot image over the network, installed it, and
  then the machine started from the NVMe disk alone, under UEFI firmware with no ISO, and
  read back the file written before the install.
- **Design.** `scripts/make_boot_image.py` builds the image on the host: FAT, Limine's UEFI
  loader taken from the ISO's own Limine files, `limine.cfg`, the kernel and every module.
  Files' new `INSTALL` writes the image into a boot slot at LBA 65536 (32 MiB, between the
  program store and the filesystem), syncs, and only then writes one MBR entry, bootable,
  type EFI System, over the image. A half-written install never boots. Only Spindle may ask.
  SexOS was first checked to boot under UEFI firmware (OVMF), which it does.
- **Also fixed.** `fetch` gave up on any download longer than 60 seconds; the deadline now
  resets whenever bytes arrive, so only a silent server times out.
- **ABI.** `fs::INSTALL` (14), `fs::BOOT_LBA`, `fs::BOOT_SECTORS`. The program store must stay
  below LBA 65536. `quil_autosave_gate.boot()` takes `SEXOS_GATE_FIRMWARE` and
  `SEXOS_GATE_NO_CDROM`.
- **Limits.** No partition editor, and
  an MBR, not GPT, so it overwrites whatever partition table the disk had. Not tried on a
  real machine. BIOS-only machines can't boot it.
- **Gates.** New `install` at 1 core. Python tests and all non-QEMU gates; `fetch` itself ran
  only inside the install gate on this build.

## A program that ignores Close is ended (2026-11-08)

- **What runs now.** If a program hasn't ended five seconds after the person closed it, Shell
  ends it. In the probe, a program built to ignore Close was gone 7.7 seconds after F11, was
  cleaned up like any exit, and its row started the next program normally.
- **Design.** `app_lifecycle::force_end` (syscall 80) lets the coordinator holding the row's
  lifecycle grant clear the run's page leaves, program rows only. The program faults the next
  time it runs, which Close's own retries guarantee, and the kernel ends it there; the usual
  service acknowledgements and retire follow, with retire skipping its check of the already
  cleared leaves. Nothing new can be reached this way: it only takes away.
- **Limits.** Quil, Spindle and Browser keep the patient close, since they may hold unsaved work.
  Five seconds is fixed. A program on a key-isolated row and one on the shared key end the same way.
- **Gates.** `program_force_probe.py` on an ISO built with `SEXOS_PROGRAM_HANG_PROBE=1`, at 1 and
  4 cores. Program disk, launch, Jot, program net, records and SMP at 1 core on the normal
  kernel. Python tests and all non-QEMU gates except the older ones.

## Install from the running system (2026-11-08)

- **What runs now.** `install` with no file writes the system that is running: boot the ISO
  (a USB stick on a real machine), type `install`, and the disk starts SexOS on its own.
  The gate booted the ISO, installed, and booted the disk alone under UEFI with its files.
- **Design.** The build now makes the boot image (`boot_image` stage) and puts it on the ISO;
  Limine loads it beside the kernel as the module `boot.img`. At BOOT the kernel maps that
  module, read-only, into Files (`fs::BOOT_IMAGE_SLOT`); nothing else can see it. The image's
  own `limine.cfg` leaves out `boot.img`, so an installed disk doesn't carry a second copy.
- **Costs.** The ISO grows from 9.8 MB to 22 MB, and 12 MiB more memory holds the image.
- **Also fixed.** The Shell app-cleanup host gate failed to compile after the forced-close
  change; its fixture now has the new call, and sex-pdx has a host stub for it.
- **Gates.** `install` (now the running system by default, `--fetch` for the network path),
  filesystem and program disk at 1 core; the master gate with the extra module. Python tests
  and all non-QEMU gates except the older ones.

## Three-hour soak (2026-11-08)

- **Result.** `scripts/soak_gate.py --hours 3` at 4 cores (TCG) on the kernel of commit
  `7422d3ca`: 97 rounds in 3.04 hours, no panic, no service or app fault (forced-stop record
  faults excluded, as they are the design), and background-record memory fully back every
  round. Each round: write and read a file, `df`, download 200 KB, open a page in the Browser,
  run Jot and save, run four programs side by side and close them, start ten background
  programs and force them to stop.
- **Timing.** Rounds took 83 to 147 seconds of wall time, but a steady 57 to 59 thousand guest
  ticks each, so the guest did the same work in the same guest time throughout; the growth was
  on the host side (the gate rereads a 140 MB serial log). Not proof of a full day on hardware.

## Programs leave the desktop's core (2026-11-08)

- **What runs now.** On a machine with spare cores, program rows no longer run on core 0 with
  Shell and Display. Rows on their own keys spread over the spare cores; the rows that share
  a key take turns on the last spare core, which their single set of present bits needs.
  A busy program can no longer slow the desktop's own core. One core still runs everything.
- **Evidence.** At 4 cores: eight programs side by side, Jot, program launch, the shared-key
  isolation probe (row 45 reading live row 44 is ended, no breach) and the forced-close probe.
  The tenancy now asserts that a shared row runs on its home core, so a misplaced row would
  stop the kernel rather than share present bits across cores. At 1 core: program disk, Jot
  and launch on the same kernel.
- **Limits.** Shared-key programs still never run in parallel with each other.

## Move a window off the tiling (2026-11-08)

- **What runs now.** Drag a window by its title bar and it moves and floats: the tiling lays
  out the other windows and leaves this one where it was put, on top while focused. Zoom
  (the green light) puts it back under the tiling. Dragging into the top bar still
  maximizes; dragging sideways along the title bar no longer does.
- **Design.** Shell's move path handled only a few early hard-coded surfaces; it now moves
  any window it knows the bounds of, through the same geometry step the tiling uses
  (`apply_window_geometry`). A single-tab title-bar drag starts a window move instead of a
  tab reorder and sets `FRAME_FLAG_FLOATING`, which the tiling skips. Absolute pointers
  (tablets, QEMU) now drive window drags as relative ones do.
- **Limits.** No resize by dragging edges yet for app windows, and unfocused floating windows
  can sit under tiled ones. Not tried with a physical mouse.
- **Gates.** New `window_float` (drag, then open a program: the tiling leaves the moved window
  alone), program disk, launch, Jot, browser web and filesystem at 1 core. Python tests and
  all non-QEMU gates except the older ones.

## Geist on screen (2026-11-08)

- **What runs now.** Window text is Geist Mono, anti-aliased, and title bars are Geist Sans,
  proportional, both by Vercel under the SIL Open Font License 1.1 (`assets/fonts/geist`,
  with `OFL.txt`). The old 5x7 bitmap font stays for small text (windows under 256 pixels
  wide), the SilkBar clock, and the few letters Geist lacks.
- **Design.** `scripts/make_font.py` renders the fonts once into `crates/sex-pdx/src/font_geist.rs`:
  Geist Mono as 12x18 cells of 8-bit coverage, which are exactly the cells of the existing
  text grid, so wrapping, selection and every layout metric stay as they were; Geist Sans
  as 18x18 cells with their own advances for titles. Display blends coverage onto the
  surface's colour (`coverage_blend`), in both the per-pixel and the row-run paths.
- **Oracles.** Display's host tests and the program pixel oracles now read the same table
  (`scripts/geist_cells.py`): a glyph's fully covered pixels must be exactly the text colour.
- **Limits.** One size. Text is still a fixed grid, so Geist Sans isn't used for body text
  yet; that needs proportional layout, which the Browser's HTML work will bring. Some older
  native gates that compare glyph pixels against the 5x7 font (Quil and Browser reader
  pixel gates) weren't rerun and will need the new oracle.
- **Gates.** Program disk, launch, file, Jot, net and browser web at 1 core; Display's host
  gates rewritten for Geist; Python tests and all non-QEMU gates except the older ones.

## Pictures in program windows (2026-11-08)

- **What runs now.** A program can draw pixels, not only text: `sex_app::pixels()` hands it
  an 800x600 canvas and `show_pixels(w, h)` puts it in the window. **View** uses it:
  `run view @photo.png` shows a PNG up to 800x600, alpha blended over the window colour.
  A larger picture is refused with a message.
- **Design.** Each program row gets one BOOT-minted 4 MiB pixel region (`sex_pdx::pixel_region`),
  writable by the program and read-only for Display, the same shape as the text regions;
  no region is minted at runtime. Program tenancy covers it, so rows sharing PKU key 8
  can't read each other's pictures. Display composites the region under the window's
  text area when the program has said how big the picture is (`OP 0x2D2`), and forgets
  it when the program presents text again. The PNG decoder now runs inside the program,
  in the program's own memory (`png::decode_into`), so a malformed file can only crash
  that program, not Display. The regions come from one buddy reservation per eight
  rows, since one per row ran out the boot frame reservation table.
- **ABI.** New Display op `0x2D2` (pixel size), new Region capability slots (97 in programs,
  600+ in Display). ABI hash updated.
- **Limits.** PNG only (JPEG is next), no scaling or scrolling, pictures don't move with
  a floating window drag until the next present. 36 MiB of RAM go to pixel regions at boot.
- **Gates.** New `program_view` (a 300x200 PNG checked pixel for pixel on screen; a
  900x700 one refused), program Jot and disk at 1 core; Display host gates; Python
  tests and all non-QEMU gates except the older ones.
## Wasm guests on wasmi (2026-11-08)

- **What runs now.** `run wasm @/FILE.wasm ARGS` runs a WebAssembly program in its own
  window; `run wasm` alone runs a small built-in guest. An unmodified Rust `std` program
  built for `wasm32-wasip1` prints, reads typed lines and exits (gate
  `program_wasm_gate.py`, source in `crates/sex-wasm/guests/greet`). This is the start of
  the Guests organ: a guest touches only its own linear memory and the calls it was linked
  with, so many guests can share one hardware key.
- **Design.** `crates/sex-wasm` wraps `wasmi` 2.0 (MIT/Apache-2.0, `no_std` + `alloc`) with
  two import sets. `sex` (native): `show`, `title`, `log`, `next` (keys and close),
  `read`/`write` in the guest's folder, `fetch` (one page), `now`, `exit`.
  `wasi_snapshot_preview1` (subset): console, keyboard lines, arguments, clocks, random
  bytes, exit; other WASI calls link but answer `ENOSYS`. Feature `sexos` gives
  `sexos::AppHost` over `sex_app`. `apps/wasm` is the runner, on the program heap.
- **Authority.** `Grants` only narrow the program's: files in `guests/NAME/` of the
  program's folder (`..`, `.`, empty parts, `\` and a leading `/` refused), `fetch` only
  in a `run +net` run. No environment. Unknown imports are refused by name.
- **Limits.** Guest memory 4 MiB in the runner (16 MiB default in the crate), optional
  fuel, 1 MiB per I/O call and per write in total, about 8 KiB of console. Modules up to
  2 MiB. A guest that keeps waiting after Close is ended; Close during a read is end of
  input.
- **Size.** The runner is 984 KB, under the 1 MiB image limit, because the interpreter is
  built for size (`opt-level = "z"`, one codegen unit, stripped; per-package profiles in
  the workspace `Cargo.toml`) and uses wasmi's loop dispatch, which is slower than its
  tail-call dispatch (1028 KB). Raising `IMAGE_BYTES` would allow the faster one.
- **Gates.** `cargo test -p sex-wasm` (17 tests); `program_wasm_gate.py` at 1 and 4 cores.
  One 1-core attempt saw no keys reach Spindle at all before any wasm code ran; the
  rerun passed.
- **Next.** WASI files over a preopened folder, guest pixels on the program pixel
  surfaces, `wasmtime` for speed, then QuickJS on Wasm.
## View opens JPEG; PNG and JPEG through zune (2026-11-08)

- **What exists now.** `crates/sex-image`: `sex_image::decode(bytes, &Limits)` returns
  width, height and `Vec<u32>` pixels in the usual unpremultiplied `0xAARRGGBB`. PNG (every
  colour type and depth, palette, tRNS, Adam7, first APNG frame) through `zune-png`; JPEG
  (baseline, progressive, 4:4:4/4:2:0, grayscale, CMYK) through `zune-jpeg`. Both are pure
  Rust, MIT/Apache-2.0/Zlib, built `no_std` + `alloc` without SIMD for the soft-float target.
  `Image::fit_within(w, h)` shrinks a photo to a surface with an alpha-weighted box filter.
- **Design.** Decoding belongs in the program that opened the file, never in Display or a
  server. `Limits` (`PHOTO` 12 MP, `SURFACE` 800x600) is checked right after the header,
  before the pixel buffer is reserved, and that reservation is fallible, so a lying header
  is an error, not a heap exhaustion. The decoder writes into the pixel buffer itself and is
  widened in place, so a picture costs one buffer. The in-house `sex_graphics::png` stays
  for the allocation-free 48x48 path.
- **Found upstream.** zune-jpeg 0.5's CMYK to RGBA conversion steps 3 bytes per 4-byte
  pixel; sex-image asks for RGB and adds alpha itself.
- **View.** `run view @photo.jpg` works. A picture that fits is decoded straight into the
  program's pixel surface (`decode_into`); a larger one, up to 3200x2400 and as far as the
  8 MiB program heap allows, is decoded on the heap and shrunk to fit 800x600. Needs the
  program heap (PR #17).
- **Limits.** No GIF/WebP. A 12 MP phone photo does not fit the heap yet (no DCT downscaling).
- **Gates.** 14 host tests against Pillow/ImageMagick fixtures, including 3,000 bit-flipped,
  truncated and marker-damaged files that must error or decode, never panic.
  `program_view_gate` (PNG exact, JPEG within 2 levels, 900x700 shown at 771x600, a text
  file refused) passes at 1 and 4 cores with the heap branch merged.

## Sound on a virtio sound card (2026-11-08)

- **What runs now.** With a virtio sound card (QEMU `-device virtio-sound-pci`), SexOS plays
  sound. Spindle's `beep [HZ] [MS] [VOL]` plays a note, and a notification Bell files in the
  "now" lane plays a two-note chime. Without a card nothing changes: Bell logs
  `[bell.sound.absent]` and `beep` does nothing.
- **Design.** BOOT leases virtio PCI devices the way it leases the NIC: one BAR view (the
  BAR holding all four virtio register blocks, at most 64 KiB), the block offsets as
  immutable BOOT metadata (`sex_pdx::virtio::Layout`, read with syscall 75), a 256 KiB DMA
  pool and the memory/bus-master command bits. No driver sees a physical register address.
  `kernel/src/virtio_boot.rs` parses the vendor capabilities; `devmgr` routes each virtio
  kind to its service. The Bell service (Senses organ) drives the card with the
  `virtio-drivers` crate (rcore-os, MIT) through `servers/sexbell/src/sound.rs`: a transport
  over the leased view, and a DMA layer that bounces buffers through the pool. Notes queue
  (16) and stream a period (43 ms) at a time while Bell waits for messages, so Bell never
  blocks on playback. The kernel restart path already quiesces any held PCI device.
- **ABI.** Syscall 75 (virtio layout); `OP_BELL_TONE` 0xC8 (arg0 Hz, arg1 ms, arg2 volume);
  `virtio::SLOT_SOUND` 20; virtio DMA region id 0x4200; `PciCapData.virtio`;
  `devmgr::init` takes the Bell domain. `crates/sex-pdx/src/virtio.rs` joined the ABI inputs.
- **Limits.** Fixed 48 kHz stereo S16, sine notes only: no PCM streaming from programs or
  files yet (needs a shared buffer from the caller and a mixer). Only modern (non-transitional)
  virtio ids. virtio-net, virtio-blk and virtio-input aren't leased yet; the same lease and
  transport take them next (`virtio_boot::supported` and `devmgr::virtio`).
- **Gates.** New `sound_gate.py`: QEMU records the card to WAV; 440 Hz/400 ms and
  1000 Hz/300 ms notes come out at 440.0 and 1000.1 Hz, 400 and 310 ms, at 1 and 4 cores;
  `--no-card` boots without a card. QEMU's WAV backend writes only delivered audio, so the
  gate checks pitch and length, not the gap between notes. Program launch at 1 core.

## Resize a window by its edges (2026-11-08)

- **What runs now.** Drag a window's left, right or bottom edge, or a corner, and it changes
  size; the text inside rewraps. A window sized by hand floats, so the tiling leaves it at
  that size when other windows open. Zoom puts it back under the tiling.
- **Design.** Shell already had resize hit zones and a Resizing state, but it sent the new
  size straight to Display and to one local record, and the tiling then put the window
  back. Resizing now goes through `apply_window_geometry`, the same step tiling and moves
  use, sets `FRAME_FLAG_FLOATING` at the start of the drag, and follows absolute pointers
  (tablets, QEMU) as well as relative ones.
- **Limits.** The resize zone is the 4-pixel rim, which is narrow for a real mouse. The
  other tiled windows don't grow into the space a shrunk window leaves until the next
  tiling. Programs get no resize event; the text grid rewraps in Display, and a pixel
  picture keeps its size. Not tried with a physical mouse.
- **Gates.** New `window_resize` (corner drag of 120x80 pixels, exact new bounds, then a
  program opens and the tiling leaves the window alone), window float and program launch
  at 1 core; Python tests and all non-QEMU gates except the older ones.

## Programs get a heap (2026-11-08)

- **What runs now.** SDK programs (`sex-app`) can use `alloc`: `Vec`, `String`, `Box`,
  `BTreeMap`, and the `no_std` crates that need them (fonts, images, tiny-skia, rustls,
  Taffy). Peek now reads up to 1 MiB of a page instead of 64 KiB, on the heap. A program
  can hold up to 8 MiB (its image plus heap) instead of 1 MiB.
- **Design.** Each program row already reserved, at BOOT, a window of frames on its own
  hardware key for the image it loads. The window grows from 1 MiB to 8 MiB
  (`program_store::WINDOW_BYTES`, separate from the 1 MiB file limit `IMAGE_BYTES`). A
  run asks for the window's pages after its image with a self-only call
  (`sex_pdx::program_memory::grow`, syscall 81): the kernel finds the row by the caller's
  domain, checks it is the row's live current run, zeroes the pages and sets their
  leaves on the row's key. No frame, capability or key is created at runtime. The pages
  join the run's manifest, so Close, a forced end and retire take them back like the
  image; a shared-key row's new pages take turns with the rest (`program_tenancy::grant`).
  The leaves were clear, so a local TLB flush is enough. `sex_app::heap` puts talc (MIT,
  `no_std`) over that span as the `#[global_allocator]`, growing it in 64 KiB+ steps; when
  the window is full an allocation is refused (`try_reserve` sees it), not faulted.
- **ABI.** New syscall 81 `SYS_PROGRAM_MEMORY` (rdi pages ≤ 256, 0 queries → rax status,
  rsi first page, rdx pages held); `WINDOW_BYTES` 8 MiB; kernel `MAX_PAGES` 512 → 2064.
  `program_memory.rs` joins the ABI inputs; hash updated.
- **Cost.** 8 MiB of frames reserved per reusable program row (eight rows: 64 MiB, was 8)
  and about 1.5 MB more kernel bss for the larger page manifests. Boots and runs in 512 MiB.
- **Evidence.** New `heaptest` program and `scripts/program_heap_gate.py`: each run starts
  with no pages, finds every granted page zero, uses Vec/Box/BTreeMap/String, reuses
  freed memory, fills the window until refused (7 MiB past a 48-page image), then
  allocates again. Four runs on a reused row, all eight rows side by side (shared-key
  rows included), each retire taking the grown pages.
- **Limits.** The window is fixed per row; a program needing more than 8 MiB needs a
  bigger constant or, later, frames from a shared pool. Pages return only at exit.
  Background program records (`program_records.rs`) don't get a heap yet.

## Anti-aliased drawing with tiny-skia (2026-11-08)

- **What runs now.** `run shapes` draws rounded windows with soft shadows, gradients,
  circles and smooth lines in its own window; the arrow keys move the front window and
  redraw it. Any SDK program can do the same with `sex-canvas`.
- **Design.** New crate `crates/sex-canvas` over tiny-skia 0.11.4 (BSD-3, `no_std` +
  `alloc`, floats through libm). It draws straight into SexOS `0xAARRGGBB` words with no
  copy: colours go to tiny-skia with red and blue swapped, so its `R,G,B,A` bytes land
  as SexOS words. Soft shadows are analytic (Gaussian falloff of the distance to the
  rounded rectangle, from a 256-step table), with no blur pass or scratch buffer.
  `Canvas::window` draws a whole frame and returns its content rectangle; `Layer`
  composites an owned buffer into any strided one. Uses the program heap and the
  program pixel surface. No ABI change.
- **Evidence.** 11 host tests in sex-canvas. `scripts/program_shapes_gate.py` installs
  Shapes, runs it, and compares the window's pixels with the same scene rendered on the
  host by `cargo run -p sex-canvas --example desktop`: exact (0 of 3712 sampled pixels
  differ) before and after an arrow key, at 1 and 4 cores, then F11 retires the run.
- **Limits.** (Programs were soft-float here; see "Programs use hardware floating
  point" below.) Shapes draws at a fixed 400x320
  because a program can't yet learn its window's size. sexdisplay's own chrome is not
  drawn with sex-canvas yet (it has no heap).

## Programs use hardware floating point (2026-11-08)

- **What runs now.** SDK programs are built with SSE2 instead of soft-float. Shapes
  draws its 400x320 scene in about 0.2 s at 1 core and 0.11 s at 4 cores under QEMU
  TCG, against 1.3 s and 0.7 s before, and the result is still pixel-exact against
  the host rendering. Float-heavy code (tiny-skia, decoders, Wasm, future codecs)
  runs at hardware speed.
- **Design.** New `kernel/src/fpu.rs`. The kernel stays soft-float and never touches
  x87/SSE state, so a task's registers stay in the core across kernel entries. Each
  `Task` has a 512-byte FXSAVE area. `scheduler_finish_switch` saves the outgoing task
  before `release_after_stack_switch`, so no other core can resume it first. It then
  restores `current_task` unless that task's state is already loaded on this core
  (per-core `FPU_LOADED`). `rearm` resets the area, so a new run never sees the last
  run's registers. Boot sets CR0.MP, clears EM and TS, sets CR4.OSFXSR and
  OSXMMEXCPT, and limits XCR0 to x87+SSE, so AVX register files stay off and can't
  carry state between tasks.
- **Build.** New target `x86_64-sex-app.json` (the same as `x86_64-sex.json` but with
  `+sse,+sse2` and the default float ABI). A build-spec stage may now name its own
  `target`, and every `iso_root/programs/*` stage uses the app target. The kernel,
  servers and BOOT apps are unchanged.
- **ABI.** A program run now starts with RSP = 8 mod 16 (as if called), which SSE code
  needs. `rearm` aligns the top idempotently. No syscall change.
- **Evidence.** New `fputest` program and `scripts/program_fpu_gate.py`. Three copies
  run at once for 15 s each. Each fills XMM0-15 and MXCSR with values only it uses,
  yields and spins thousands of times, and checks every register and an exact float
  series. A reused row starts with the reset MXCSR. The gate checks that the program
  has SSE instructions and the kernel has none. It passes at 1 and 4 cores. With save
  and restore removed, it fails ("a run started with another run's MXCSR"). The
  shapes, heap, view and launch gates also pass.
- **Limits.** Only x87+SSE: no AVX or XSAVE yet. Eager save/restore on every switch
  costs about 2x512 bytes of copying per switch, even for soft-float tasks.
## Geist at any size (2026-11-08)

- **What runs now.** Display draws Geist from the TrueType files at run time instead of
  a table rendered once at one size. Grid text at 2x and up is Geist Mono fitted to the
  cell the text scale gives (12x18 today, any 6s x 9s cell later); window titles are
  Geist Sans, kerned (GPOS `kern`) and placed to a quarter pixel.
- **Design.** New `crates/sex-font` (no_std, no heap): `ttf-parser` (MIT/Apache-2.0)
  reads the fonts without allocating, and its own rasterizer turns outlines into 8-bit
  coverage by signed-area accumulation in fixed point (1/256 px), integer-only, so the
  softfloat target and host tests give bit-identical pixels. `sex_font::draw_cell` sizes
  a monospaced font to any cell (19 px at 12x18, as before). Display keeps drawn glyphs in
  a fixed cache (`servers/sexdisplay/src/glyphs.rs`: 2048 slots over 192 KiB, emptied
  whole when full) and recent title layouts, so a size costs one draw per character.
  The generated `sex-pdx::font_geist` table and `scripts/make_font.py` are gone; the
  `abi_version_hash` returns to its pre-table value since `sex-pdx/src/lib.rs` lost that line.
- **Oracles.** `scripts/geist_cells.py` now asks sex-font's `cells` example for the
  cells, so pixel oracles check the screen against the renderer Display runs. Display's
  host gates link sex-font (built for the host with cargo) and test titles by layout.
- **Limits.** 1x text keeps the 5x7 bitmap font (crisper at 6x9). No text shaping yet:
  `harfrust` needs a heap, so it waits for the app heap work. No hinting, so stems are a
  little lighter than FreeType's. Text is still a fixed grid; per-window text size (zoom)
  is the next visible step and needs only a scale per surface now.
- **Gates.** sex-font unit tests; display text readability and surface groups host gates;
  program launch (pixel oracle) at 1 and 4 cores in QEMU (UEFI/OVMF boot).

## Text size per window (2026-11-08)

- **What runs now.** Alt+= makes the focused window's text larger, Alt+- smaller and
  Alt+0 restores it. Each step is a whole text scale (cells 6s x 9s, s from 1 to 5), so
  Geist Mono is drawn fresh at 18x27, 24x36 and 30x45 as well as 12x18; scale 1 is the
  5x7 font. Text rewraps to the new width at once.
- **Design.** Display owns it: `Surface::text_zoom` is a step from the width's default
  scale, stored clamped so the opposite key always acts. Shell's `desktop_keys` turns the
  Alt shortcuts into `OP_TEXT_ZOOM` (new, 0x2D4, `sex_pdx::desktop_style`; ABI change),
  which Display accepts only from the window manager. The shortcut's release never
  reaches the app. Display's glyph cache grew to 512 KiB for the larger cells.
- **Limits.** Not remembered across runs. Apps that lay out their own rows for the
  default scale (Browser) wrap at Display's columns when zoomed.
- **Gates.** New `text_zoom_gate.py` (default, two steps up, one down, reset; every fully
  covered pixel exact against sex-font's cells) at 1 and 4 cores; program launch at 1
  core; display text, scene, cursor, frame output and surface-group host gates; Shell key
  routing in the clipboard host gate.

## Tiles fill the space a floating window leaves (2026-11-08)

- **What runs now.** When you let go of a window you moved or resized, the remaining tiled
  windows retile across the whole content area; the floating one stays where it was put,
  above them. The resize grip is now 8 pixels, the rim plus the content inset, so a mouse
  can find it without covering any text.
- **Gates.** `window_resize` now also checks the retile (`[shell.window.retile]`, another
  window tiled from the left edge, the resized one untouched); window float.
- **CI.** Master's GitHub CI had been red for weeks. The fixes so far: fetch crates before the
  offline host gates; read `cargo tree` without CI's colour codes in the TLS platform check;
  install mtools for the boot image; and update two log checkers to today's truth (more
  than seven BOOT region grants, a 128 KiB runtime heap, PKU keys that no longer equal PD
  ids, with leaves checked against each client's own key). Host builds on ARM got
  non-x86 stubs for `program_memory::grow` and `virtio::layout`.
## Web pages with CSS, laid out and drawn in Geist (2026-11-08)

- **What runs now.** `run web` is a graphical browser program: HTML is parsed, styled
  with CSS and laid out inside the program, then drawn with proportional Geist Sans (Geist
  Mono for code) into its pixel surface. `run +net web URL` follows redirects and fetches
  the page's screen style sheets (`<link rel=stylesheet>`, print sheets skipped) and its
  PNG/JPEG pictures. Up/Down/Space/PgUp/PgDn/Home/End scroll, Right/Left move a focus ring
  between links, Enter opens one (fragments scroll to the anchor), Backspace goes back,
  `/` types an address. The text Browser (kaleidoscope) is unchanged.
- **Design.** Two new crates, both `no_std` + `alloc`, integer-only:
  `crates/sex-html` (parser with implied `html/head/body`, void and raw-text elements,
  self-closing `p/li/td`, entities; CSS with compound/descendant/child/sibling selectors,
  attribute selectors, `:not :nth-child :first-child`, specificity, `!important`, inline
  `style`, `@media` width queries, `em/rem/%/vw/pt`, `calc()` approximated; the cascade
  indexes rules by id/class/tag; layout does block flow with sibling margin collapsing,
  line boxes with half-leading and baseline alignment, `text-align`, `white-space`,
  lists with markers, automatic table layout from min/max-content (colspan,
  `border-collapse`), flex rows (grow, basis, shrink, wrap, gap, `justify-content`,
  `align-items:center`), inline blocks, images, form controls; output is a display list
  plus link rectangles and anchors) and `crates/sex-web` (a `Host` measuring with
  sex-font, glyphs through sex-font's cache, synthetic bold and oblique, rounded boxes with
  4x4-sampled corners, nearest-neighbour pictures from sex-image). Layout widths and paint
  pen positions come from one function, so text lands where layout put it. `sex-app::net`
  gains `fetch`, which keeps headers (`Reply::header`) for redirects; `get` is unchanged.
  The parser runs in the program's confinement, never in Display. No ABI change.
- **Evidence.** 15 host tests in sex-html (block-level images, inline fragment anchors and cell alignment among them). `scripts/program_web_gate.py`: the welcome page,
  after Down, and with a link focused match `cargo run -p sex-web --example page`
  exactly (0 of 9020 sampled pixels differ, each). `scripts/program_web_net_gate.py`
  serves `scripts/web_site` over HTTP: a 302 is followed, 1 sheet and 1 PNG load, the page
  matches the host render exactly, Right+Enter opens the second page (exact), Backspace
  returns (exact). Both at 1 and 4 cores. Welcome layout takes 22 to 102 ticks, a repaint
  12 to 37 ticks under TCG.
- **Limits.** Drawn at 410x330, the size Shell tiles a program, because a program can't
  learn its window size yet. Floats and positioned boxes stay in normal flow; no
  `::before`, no grid (laid out as blocks), no web fonts, no bold Geist file (synthetic).
  No scripts yet: next is QuickJS-NG on Wasm with DOM bindings, then Canvas 2D. HTTP/1.0
  only, so no keep-alive; gzip not requested. Gates need a fresh `--out` directory.

## Web runs page JavaScript (2026-11-08)

- **What runs now.** Pages' classic scripts (inline and `src`, in document order) run in
  `run web`. They can read and change the page (`document`, `getElementById`,
  `querySelector(All)`, `createElement`, `appendChild`, `innerHTML`, `textContent`,
  `classList`, `style`, `dataset`, attributes), listen for clicks and keys
  (`addEventListener`, `onclick` attributes and properties, bubbling, `preventDefault`), and
  use `setTimeout`, `setInterval`, `requestAnimationFrame`, promises, `console`,
  `location`, `localStorage` (per page, in memory). The page is laid out again after any
  change. Buttons and elements with click listeners join links in Right/Left focus; Enter
  clicks them. A page's key handlers see each key first and may keep it.
- **Design.** New crate `crates/sex-js` (`no_std` + `alloc`). The engine is QuickJS-NG
  v0.10.1 (MIT), compiled to wasm32 by `scripts/build_quickjs_wasm.sh` (wasi-sdk 25,
  reproducible, 545 KB, checked in as `crates/sex-js/quickjs.wasm`) and run on wasmi, so
  scripts are confined by the interpreter inside Web's own confinement, and floating point
  stays in software. The guest has one native call, `__host(op, ...strings)`; the DOM API
  is JavaScript (`src/prelude.js`) over it, and `Document` answers the ops on sex-html's
  tree. Long scripts stop after 2 s (QuickJS interrupt handler). sex-html gains tree edits
  (`create/insert/detach/set_attr/set_text`), HTML serialization and fragment parsing, and
  styles in tree order. `Link` gains `node`; `Dom::activatable` decides what can be
  focused. sex-app gains `Window::poll`, `Window::next_within` and `with_stack`: Web now
  runs on a 2 MiB stack from its heap, as the 64 KiB a run starts with overflowed (a
  guarded, growable program stack is coming from the limits work). wasmi is now built at
  opt-level 3 for speed (programs may be 16 MiB since #21). No ABI change.
- **Evidence.** 11 host tests in sex-js (DOM edits, `innerHTML`, events, timers,
  navigation, a runaway loop stopped, language features). `scripts/program_web_js_gate.py`
  serves `scripts/web_site/js.html`: the script builds a list, animates a bar to the end
  with `setInterval`, a button click and a `k` key change the page; each state matches
  `cargo run -p sex-web --example page` (which runs the same script) exactly, 0 of 9020
  sampled pixels differ, at 1 and 4 cores. The engine starts in 1.7 s at 1 core and 0.8 s
  at 4 cores under TCG.
- **Limits.** No Canvas 2D or WebGL yet (next). No `fetch`/`XMLHttpRequest`, no layout
  boxes for `getBoundingClientRect` (zeros), no text inputs, no ES modules as modules
  (run as classic scripts), pictures a script adds are not loaded.

## Canvas 2D in Web pages (2026-11-08)

- **What runs now.** `canvas.getContext('2d')` in `run web`: paths (`moveTo`, `lineTo`,
  quadratic and cubic curves, `arc`, `ellipse`, `rect`, `roundRect`), `fill` (nonzero and
  even-odd), `stroke` with width, caps and joins, `fillRect`/`strokeRect`/`clearRect`,
  solid colours (any CSS colour), linear and radial gradients, `globalAlpha`, the common
  `globalCompositeOperation`s, `save`/`restore`, transforms, and `fillText`/`measureText`
  in Geist with `font`, `textAlign` and `textBaseline`. Animation through
  `requestAnimationFrame` redraws the window.
- **Design.** The prelude queues each call as a short command (fields split by U+001F,
  commands by U+001E) and sends them to the host in batches at the end of each script or
  event. `sex-web::canvas` runs them on a premultiplied surface with tiny-skia (through
  sex-canvas) and Geist, and the page shows the surface as a picture `canvas:<node>` laid
  out as a replaced element sized by its `width`/`height`. Path points are transformed when
  added; stroke widths and text sizes scale with the transform. `sex_web::measure` gives text
  widths without a glyph cache, for `measureText`. No ABI change.
- **Evidence.** sex-js test `canvas_commands_are_queued`. `scripts/program_web_canvas_gate.py`
  serves `scripts/web_site/canvas.html` (gradient, text, stroked arc, rotated square, curves,
  measured text, six animation frames, six more after a button click): both states match
  the host render exactly, 0 of 9020 sampled pixels differ, at 1 and 4 cores. The JS gate
  still passes.
- **Limits.** No `clip`, `drawImage`, `getImageData`/`putImageData` (zeros), patterns,
  shadows or line dashes yet; `arcTo` is a straight line; `strokeText` fills. WebGL next.

## WebGL in Web pages (2026-11-08)

- **What runs now.** `canvas.getContext('webgl')` (also `experimental-webgl`, and
  `webgl2` with the same feature set for now) in `run web`: buffers, vertex arrays, element
  draws (8/16/32-bit indices), GLSL ES 1.00 and 3.00 shaders, uniforms of every type
  (arrays, matrices, samplers), 2D and cube textures from bytes or from a page picture
  (`<img>` or another canvas) with flip-y and premultiply, depth test, culling, blending,
  scissor, color mask, polygon offset, points and lines, `readPixels`, and the
  `getParameter`/`getShaderParameter`/`getActiveUniform` queries libraries use.
- **Design.** New crate `crates/sex-gl` (`no_std` + `alloc`): a WebGL 1-shaped `Context`
  over a software rasterizer (near/far clipping, top-left fill rule, perspective-correct
  varyings). Shaders: GLSL ES is preprocessed by pp-rs (MIT/Apache, vendored as
  `vendor/pp-rs-0.2.1`, ported to `no_std`, its 35 tests pass), rewritten to GLSL 450
  (loose uniforms to blocks, combined samplers split, explicit locations, ES built-ins
  renamed), parsed and validated by naga 30 (MIT/Apache), and run by a small interpreter
  over naga's IR. The prelude queues WebGL calls like Canvas 2D, with one ArrayBuffer of
  bulk data per batch; queries flush the batch and ask synchronously. Web shows each canvas
  that drew a new frame as the picture `canvas:<node>` (`sex_web::canvas::present_gl`).
  The workspace now has one `[patch.crates-io]` entry (pp-rs), allowed by name in
  `check_tls_platform.py`.
- **ABI change (guest/host, sex-js only).** The QuickJS guest passes `ArrayBuffer`
  arguments to `__host` as raw bytes, and an op with bit 16 set returns an `ArrayBuffer`;
  ops 64 and up take byte arguments (`Page::call_bytes`). `quickjs.wasm` rebuilt (sha256 in
  `build_quickjs_wasm.sh`). No kernel or system ABI change.
- **Evidence.** sex-gl tests: shader translation, a colored triangle, a lit textured cube,
  ES 3.00 with blending and scissor, and the ES 3.00 forms three.js uses (precision on
  declarations, `const in` parameters, boolean uniforms, struct uniforms, `modf`).
  sex-js test `webgl_draws_a_triangle_from_js`. On the host, three.js r170 (unmodified
  `three.cjs`) renders a `MeshStandardMaterial` cube lit by ambient and directional light.
  `scripts/program_web_webgl_gate.py` serves `scripts/web_site/webgl.html` (a lit cube
  textured with the page's logo, six animation frames, six more after a button click): both
  states match the host render exactly, 0 of 9020 sampled pixels differ, at 1 and 4 cores.
  The page, net, JS and canvas gates still pass.
- **Limits.** Framebuffers and renderbuffers are stubs (drawing goes to the canvas), no
  mipmaps (level 0 only), derivatives are 0, no WebGL 2 instancing draws, uniform blocks,
  3D textures or transform feedback yet: those are next, for three.js. Shader compiles take
  a moment in software float until programs get SSE. three.js in `run web` still needs a
  longer script time budget (loading its 1.3 MB takes about a second on the host).

## three.js runs in Web (2026-11-08)

- **What runs now.** three.js r170 (MIT), the unmodified `three.cjs` from npm, loads in
  `run web` and renders with `WebGLRenderer`: `MeshStandardMaterial` (PBR) and
  `MeshPhongMaterial` meshes under ambient and directional light, animated with
  `requestAnimationFrame`.
- **Changes.** Page scripts (`Script::eval`) get their own time budget,
  `Limits::load_budget_ms` = 120 s, since fetching and loading three.js takes about 12 s
  and 30 s in QEMU; event handlers and timers keep `budget_ms`, now 10 s (browsers ask
  about a script after about that long). The engine may grow to 64 MiB with a 48 MiB
  QuickJS heap. Web accepts pages and scripts up to 4 MiB (was 1.5 MiB). No ABI change.
- **Evidence.** `scripts/program_web_three_gate.py` serves `scripts/web_site/three.html`
  with `scripts/web_site/three/three.cjs` (and its MIT licence): a PBR cube and a Phong
  sphere, six frames, then six more after a button click. Both states match the host
  render exactly, 0 of 9020 sampled pixels differ, at 1 and 4 cores. The other web gates
  still pass.
- **Limits.** Slow: loading three.js takes about 30 s in QEMU (software float, wasmi
  interpreting QuickJS), and a frame of shading costs thousands of instructions a pixel in
  the shader interpreter. Next: compile shaders to a flat bytecode, then WebGPU.

## Master CI green again (2026-11-08)

- GitHub CI on master passes every step for the first time in weeks (run 42502328687,
  commit 21db7443). Besides the fixes listed above, the profile gates now name each stage's
  exact normal rustflags (the Browser's soft-crypto cfgs), the disk persistence and desktop
  close gates wait for markers the system still prints and follow Quil's unsaved-close
  prompt, and the desktop, launcher, Quil-successor and Spindle pixel checkers read Geist
  cells through `check_desktop_pixels.cell_errors` (fixtures use `draw_cell`).
- Not a bug, though it looked like one: the kernel logs `[ps2.irq1.entry]` only for the
  first 16 interrupts, so missing entries later in a log don't mean keys were lost.

## Video with FFmpeg: Play (2026-11-08)

- **What runs now.** `run play @VIDEO.mp4` plays an H.264 MP4 in a window, paced to its frame
  rate. FFmpeg (n7.1.2: libavformat with the MOV demuxer, libavcodec with the H.264 decoder,
  libswscale, libavutil; LGPL-2.1+, no GPL parts) runs inside Play's own confinement. Play
  decodes each frame into a private buffer, then copies it into its pixel surface and shows it.
  Anything that is not a video is refused in words ("Cannot play that video: ...").
- **How.** `crates/sex-libc` is a small C library in Rust for programs that link C:
  malloc over the program heap (with alignment up to 4 KiB), string.h, printf/scanf engines,
  stdio over hooks, libm-backed maths, and time from ticks. `scripts/build_ffmpeg.sh` cross-builds
  FFmpeg with clang for x86_64-sex's soft-float, no-SSE, no-red-zone code (musl headers with
  `long double` mapped to `double`). `apps/play/src/ffmpeg.c` is a small bridge (open, next
  frame as BGR0 scaled to fit 800x600, close); `scripts/build_play.sh` builds Play as a PIE.
- **Gates.** `scripts/program_play_gate.py`: Spindle fetches Play and a 320x240, 25 fps,
  50-frame test clip; all 50 frames are shown in about 2 s on 1 and 4 cores, colour probes
  match the clip, the last frame is on screen, and a text file is refused.
- **Next.** Sound (AAC/MP3 through Bell's virtio sound card), SSE in programs for larger video,
  and a presented/consumed handshake with Display so frames never overlap their presentation.

## Games: Solitaire and Chess (2026-11-08)

- **What runs now.** `run solitaire` (Klondike, draw one) and `run chess` (White against the
  computer) open in their own windows. `run web` links to `about:games`, which lists both.
  They are original pages in plain JavaScript on Canvas 2D (`apps/web/src/games/`), built into
  Web as `about:solitaire` and `about:chess`, so every game is also a test of the browser.
- **How.** `apps/web` builds the same `main.rs` three times, as `web`, `solitaire` and `chess`;
  `first_page()` picks the start page from `CARGO_BIN_NAME`. Two `copy_file` stages put the
  copies in `iso_root/programs`. Built-in pages are a table of `about:` addresses.
- **Play.** Programs get keys only (no mouse yet), so both games are keyboard games. Solitaire:
  arrows move, Enter picks up/puts down, Up takes more of a column, Space draws, A sends a
  card home, U undoes, N deals. Chess: arrows, Enter picks a piece then its square, Esc lets
  go, U takes back, N, L for Easy/Medium/Hard. Full rules (castling, en passant, promotion to
  a queen, mate, stalemate); perft matches the standard counts except under-promotion.
- **Engine.** Alpha-beta with quiescence, incremental piece-square score, moves in typed
  arrays. It stops at a node budget (100/400/1500) or 1.5 s, whichever comes first, to stay
  inside the page's 2 s script limit. QuickJS on wasmi under QEMU TCG manages about 160
  positions a second, so the computer searches only 2 plies: a beginner's opponent.
- **Gate.** `scripts/program_games_gate.py`: both games match the host render pixel for pixel
  when started and after one move (0 of 9020 sampled pixels differ), and the computer answers
  e2-e4 without hitting the script limit. `program_web_gate.py` still passes.
- **Next.** Pointer input for programs (clicks and drags), splitting the chess search across
  timer ticks so it can think longer, and the three.js games (3D chess, maze, pinball,
  three-quake) once WebGL lands. The upstream cards repo has no licence, so it was not used.
- Screenshot: `assets/sexos-games-solitaire-chess-2026-11-08.png`.

## Games: Maze in WebGL (2026-11-08)

- **What runs now.** `run maze` (also `about:maze`, listed at `about:games`): walk a 9x9 brick
  maze in first person and find the turning gold diamond. Up walks a cell, Down steps back,
  Left/Right turn, A walks by itself with a hand on the right wall, N makes a new maze.
  Original page in plain WebGL 1 (`apps/web/src/games/maze.html`): a depth-first carved maze,
  textures made in the page from bytes (brick, floor tiles, ceiling), one textured shader
  with per-vertex distance light, depth test and culling. The maze95 site named in the
  request publishes no source repository or licence, so it was not used.
- **Games and the script limit.** Each game now draws its first frame on a timer tick, and
  chess draws after the computer's move on its own tick, so setup, thinking and drawing
  each get the page's 2 s script budget. The gate fails on any `InternalError: interrupted`.
- **Speed.** The maze matches the host render pixel for pixel, but under QEMU TCG a 410x274
  frame takes about 2 s with soft-float programs and about 1.1 s on the SSE branch (#18), so
  a step or turn is 3 frames. Faster software GL (or a GPU) is what makes 3D games playable.
- **Gate.** `scripts/program_games_gate.py` now also starts Maze, compares its first frame and
  the frame after one step with the host render, and times the step.
- Screenshot: `assets/sexos-games-maze-2026-11-08.png`.

## Faster software shaders (2026-11-08)

- **What changed.** `sex-gl` no longer walks the naga shader tree for every pixel. Each
  shader is lowered once into a flat program (`Step`s with jumps; calls inlined, loops as
  counted back-jumps), and common expressions become prepared operations (copy, swizzle,
  compose, load, field, float binary ops, mat*vec, dot, normalize, min/max/clamp/mix) with
  their operand shapes worked out ahead of time.
- **Eight pixels at a time.** Fragment shaders then run 8 pixels per step (`wide.rs`):
  every register is a row of 8 lanes, so each prepared operation (and texture lookups,
  now prepared too) works on all 8 and the arithmetic vectorizes. Each pixel keeps its
  own program counter; the lowest one runs next with only the pixels waiting there, so
  branches, loops, early returns and `discard` that differ between pixels stay exact. The
  rasterizer depth-tests a triangle's fragments, shades them in groups and then writes
  them in order (they cover different pixels, so the result is the same).
- **Effect.** The lit textured cube bench (`cargo run --release -p sex-gl --example bench`,
  410x274) goes from 24 ms to about 8 ms per frame on the host (3x) with an identical
  pixel hash; host renders of the maze, WebGL and three.js pages are byte-identical
  before and after. New test `divergent_branches_loops_and_discard_per_pixel`. No ABI
  change.
- **Evidence.** `cargo test -p sex-gl -p sex-js -p sex-web`, and the WebGL, Canvas, JS,
  Web, games and three.js gates at 1 and 4 cores.
- **Next.** WebGPU (WGSL) on the same rasterizer and interpreter.

## WebGPU in Web pages (2026-11-08)

- **What runs now.** `navigator.gpu` with render pipelines: adapter and device, buffers
  (`mappedAtCreation`, `queue.writeBuffer`, vertex, index, uniform, read-only storage),
  textures (`writeTexture`, `copyExternalImageToTexture` from page pictures), samplers,
  WGSL shader modules, bind groups (`layout: 'auto'`), render pipelines (vertex formats,
  topology, culling, depth compare and write, blending, write masks), command encoders,
  render passes (clear, viewport, scissor, `draw`, `drawIndexed`) and
  `canvas.getContext('webgpu')`.
- **How.** It runs on the same software renderer as WebGL. `sex_gl::Program::from_wgsl`
  builds a program from a WGSL module's vertex and fragment entry points (naga
  `wgsl-in`). Each uniform or storage binding maps WGSL's byte layout (member offsets,
  array strides, matrix column padding) onto the interpreter's lanes. Textures and samplers
  bind by group and binding, and a sampler sets the filters and wrapping of the textures
  sampled with it. Clip depth runs 0..w. The page side (`prelude.js`) keeps buffers and
  textures, records passes, and replays them on `queue.submit` as GL batches with four
  new commands (`gp`, `gb`, `gx`, `gs`). Vertex and fragment modules are parsed
  separately, so they may reuse names. Integer vertex formats (and WebGL 2
  `vertexAttribIPointer`, new command `aip`) reach shaders as integers. Runtime-sized
  storage arrays hold up to 1024 elements, filled from the bound buffer. No ABI change.
- **Not yet.** Compute passes, render targets other than the canvas, instancing beyond
  repeated draws, and `@builtin(instance_index)`.
- **Evidence.** Tests `wgsl_pipeline_with_uniform_layout_texture_and_sampler` and
  `wgsl_separate_modules_integer_attributes_and_storage_arrays`. The gate
  `scripts/program_web_webgpu_gate.py` serves `scripts/web_site/webgpu.html` (a lit,
  textured, depth-tested cube in WGSL) and matches the host render pixel for pixel at 1 and
  4 cores, before and after a click turns it six frames. The WebGL, JS and Web gates still
  pass.
- Screenshot: `assets/sexos-web-webgpu-2026-11-08.png`.
## Games: 3D Chess on three.js (2026-11-08)

- **What runs now.** `run chess3d` (also `about:chess3d`, listed at `about:games`): the chess
  game on a three.js board with lathe-turned pieces, an extruded knight, Phong lighting, a
  cursor ring, move and check marks, and the same keys as 2D Chess. Original page
  (`apps/web/src/games/chess3d.html`) on the unmodified three.js r170 already in the tree.
- **Shared engine.** The rules, computer player and keys moved to
  `apps/web/src/games/chess-engine.js`, used by both chess pages. Built-in pages now load
  their relative `<script src>` from Web's built-in table (`about:chess-engine.js`,
  `about:three.cjs`; `games/three.cjs` links to `scripts/web_site/three/three.cjs`). The
  computer may think up to 4 s now that handlers get 10 s.
- **Fitting the script limit.** three.js builds each material's shader program on first
  use, about 6 s apiece here, so the page builds them one per timer tick
  (`renderer.compile` on a small scene), then uploads the piece shapes and renders the
  empty board on their own ticks, before the first real frame.
- **Speed in QEMU (soft-float).** three.js loads in about 20 s; setup takes about 25 s
  more; each redraw after a key takes about 12 s (2 s of three.js scene work in QuickJS,
  the rest software shading). Playable, but slow.
- **Gate.** `scripts/program_games_gate.py` starts 3D Chess, matches the host render after
  setup and after picking a pawn (0 of 9020 sampled pixels differ), and plays e2-e4 to the
  computer's reply. `SEXOS_GAMES=chess3d,maze` runs only some games.
- **Not done, and why.** Graveyard Pinball publishes no source or licence and needs WebGPU
  with TSL. three-quake (GPL-2 code, shareware data) needs ES modules (99 files), `fetch`
  of an 18 MB pak, pointer lock and real-time drawing; a bundle of it into one classic
  script plus `fetch` would be the first step. The 3D-Chess-Game repo (MIT) needs a mouse,
  GLTF models and a Stockfish worker.
- Screenshot: `assets/sexos-games-chess3d-2026-11-08.png`.

## Games: XIRTUS Pinball, and Canvas drawImage (2026-11-09)

- **What runs now.** `run pinball` (also `about:pinball`, listed at `about:games`): an
  original pinball table played from the keyboard. Its theme follows Anderson Mancini's
  Graveyard Pinball, used with his permission (Andreas asked him): coffin slingshots, skull
  bumpers, a skull crypt, a dot-matrix display, a chrome ball and glowing flippers under a
  purple moon. No code or art is taken from it. Rolling over the six X I R T U S letters
  opens the portal, and sinking the ball in the crypt then raises the multiplier. Three
  balls a game. Left or Z flips left, Right or / flips right, Space launches, N restarts.
- **Physics.** Fixed steps of 1/120 s (1/240 s near the flippers), driven by elapsed time
  so the speed doesn't depend on the frame rate. Walls are capsule segments and bumpers are
  circles, both found through a 16-pixel grid, and flippers transfer their own surface
  speed. A ball that stops on the table gets a nudge. Web sends keyup right after keydown,
  so a key press gives the flipper a 0.22 s pulse. A Node simulation of 30 games
  (`node`, stub canvas, a careless robot player) ended every game, with none stuck.
- **Canvas `drawImage` (new in the browser).** `drawImage(img or canvas, ...)` in the 3-,
  5- and 9-argument forms is a `di` command in `sex-web/src/canvas.rs`. It reads the
  source from `fonts.images` (`canvas:<node>` or the image's `src`). A same-size copy onto
  whole pixels with source-over runs in integer code, with no tiny-skia and no floats;
  anything else goes through a tiny-skia pattern fill. Test: `draw_image_copies_another_canvas`.
  Pinball draws the table once into offscreen canvases and each frame copies back only the
  rectangles the ball and flippers left. The ball is a sprite copy, and the score digits
  are dot-matrix glyphs drawn once and copied.
- **`num()` in the canvas prelude** prints whole numbers without float rounding (slow
  without a float unit). This affects every canvas page, and their gates are unchanged.
- **Speed in QEMU (soft-float).** About 8 frames a second at close to real time (each frame
  advances 12 to 15 physics steps). JS takes most of a frame: physics about 30 ms and
  drawing about 25 ms. Canvas work in Rust dropped from 16 to 58 ms per frame to 2 to 6 ms
  with the integer copy. SSE (#18) should speed up the JS part, which is all float math in
  wasmi.
- **Gate.** `scripts/program_games_gate.py` matches the set-up table against the host
  render (0 of 9020 sampled pixels differ). It then launches a ball, flips every 1.5 s
  until the ball drains, and reports score, frames and fps.
- Screenshot: `assets/sexos-games-pinball-2026-11-09.png`.

## Web: fetch(), URL and URLSearchParams (2026-11-09)

- **fetch().** Scripts can now `fetch(url)`. The Promise resolves to a `Response` with
  `ok`, `status`, `statusText`, `url`, `headers` (content-length) and `arrayBuffer()`,
  `text()`, `json()` and `blob()`. Network errors reject with a TypeError, as browsers do.
  - **How it works.** New ops 66 `FETCH` (url → request id) and 67 `FETCH_BODY` (id →
    bytes once; with bit 16 an ArrayBuffer, otherwise text). Web serves requests on the
    next tick after the script returns and dispatches `fetched\tID\tSTATUS\tLENGTH`.
  - **Where it loads from.** The same places as the page: built-in `about:` files,
    `@file` pages (relative to the page's folder), and http(s) (redirects followed, the
    real status passed on).
  - **Limits.** Bodies up to `FETCH_MAX` = 32 MiB (pages and scripts keep
    `SOURCE_MAX` = 4 MiB). At most 64 requests and unread bodies at a time.
  - Pages opened from a file (`@dir/page.html`) now also load their `<script src>` from
    beside them.
- **URL and URLSearchParams.** Parsing, relative resolution with dot segments, and
  `searchParams`.
- **ABI / limit changes.** `sex_js::Limits::default()` now gives a 160 MiB engine memory
  and a 128 MiB QuickJS heap (was 64 and 48). three-quake needs about 100 MiB for its
  18 MB pak and first map. The wasm memory still grows only as a page uses it.
- **Host page example** answers fetches from files beside the page.
  `SEX_WEB_BUDGET_MS` raises the handler time limit there, for heavy pages.
- **Gate.** `scripts/program_web_fetch_gate.py` serves `scripts/web_site/fetch.html` over
  the network. The page fetches a JSON file, a PNG's bytes, three.cjs (1.3 MB) and a
  missing file, and parses a URL. The statuses must be 200, 200, 200, 404, and the page
  matches the host render (0 of 9020 sampled pixels differ). In QEMU the 1.3 MB fetch
  takes about 12 s.
- **three-quake (mrdoob, GPL-2 code, shareware data), status.** Bundled with esbuild and
  three r183 into one 2.1 MB classic script (not in the tree). On the host page example
  it loads the pak and the first map, but only with the higher heap and a handler limit
  well over 10 s. It reaches the renderer after about 11 minutes of host time. three
  r183's WebGL2 shader prefix (`precision highp usampler2D;`) isn't parsed yet; the
  browser thread has that. Under QEMU it would take hours per frame, so it waits for a
  much faster JS engine.

## Games: Metaspace Pinball theme, and where the frame time goes (2026-11-09)

- **Metaspace table.** Andreas asked for the pinball to use the Xirtus Arcade games and feel like part of the XIRTUS Metaspace. It's now XIRTUS Metaspace Pinball, using the Metaspace palette: ink, teal, mint, gold and ivory, with the four-pointed sigil.
  - The six rollover lights are the arcade worlds ZENDRIVE, MANDELPOD, MACHINE ELF, RABBITVERSE, TSUNAMI and Rubber Duckie (Z M E R T D), each in its own colour. The display names a world when you find it.
  - Finding all six opens the gate, and sinking the ball in the gate's star raises the multiplier.
  - The background is the archipelago, with islands under the stars and waves rolling into the drain.
  - Physics and layout are unchanged, and the page stays GPL-3.0-only. No assets are copied: everything is drawn in code.
- **Speed measurement.** The same master pinball page was run two ways:
  - In QEMU (the games gate): 7-9 fps, with physics 26 ms and drawing 22 ms per frame at 10 steps.
  - On the host with the `page` example (release, self-playing copy): 52-54 fps (the example's frame pacing), with physics 0-1 ms and drawing 1-3 ms at 2 steps.
  - The cloud container has no `/dev/kvm`, so QEMU runs as TCG: every instruction is emulated, and programs are soft-float until SSE (#18).
  - Most of pinball's QEMU slowdown is therefore emulation, not SexOS.
  - Heavier pages (three.js) are bound by QuickJS running inside wasmi even on the host. The browser thread owns that work.

## Media: Play with sound, an SSE FFmpeg, more formats (2026-11-09)

- **FFmpeg built for SSE.** `scripts/build_ffmpeg.sh` now builds FFmpeg n7.1.2 for the program target (x87 + SSE2, no AVX). It uses FFmpeg's nasm assembly and runtime CPU detection. AVX, AVX2, AVX-512, FMA and XOP are turned off, since the kernel saves state with FXSAVE.
  - Formats: MP4/MOV, Matroska/WebM, Ogg, MP3, AAC, FLAC and WAV.
  - Video codecs: H.264, HEVC, VP8, VP9 and MPEG-4.
  - Sound codecs: AAC, MP3, Opus, Vorbis, FLAC and PCM.
  - swresample turns any sound into 48 kHz stereo.
- **Sound for programs.** Each program row gets a 256 KiB sound ring, shared only with Bell. `sex_app::sound::Sound` is the API: open, write, played, pending, volume, close.
  - Bell mixes up to 16 rings with its own notes into the virtio sound card.
  - Bell counts what the card has played per ring, so a program has a real sound clock.
  - ABI changes:
    - client slot 260 (`sound_ring::CLIENT_SLOT`);
    - Bell slots 900+k;
    - regions 0x4800+k;
    - `OP_PLAY` 0xC9;
    - program rows now hold `SLOT_BELL`;
    - program-row fixed pages go from 1200 to 1400.
  - Bell's mix buffer is a static, because Bell's stack is small.
- **Play.**
  - Streams the file through `files::read_at`, with a 256 KiB cache, so file size is not limited by memory.
  - Plays sound through the ring and paces pictures by the sound clock, or by ticks when there is no sound.
  - Pictures are decoded ahead into a short queue, so sound never starves while a picture waits.
  - Sound-only files show "Playing sound".
  - Play is built for the SSE target with `scripts/build_play.sh`.
- **Gate.** `scripts/program_media_gate.py` records QEMU's virtio sound card to a WAV file and plays three clips:
  - an MP4 with H.264 and AAC (440 Hz);
  - a WebM with VP9 and Opus (660 Hz);
  - an MP3 (880 Hz).

  Each clip must show all its pictures, no picture may be more than 500 ms late, and each clip's tone must appear in the WAV at its pitch for about 2 s. Each clip takes about 2.6 s.

## sex-media: one player for Play and the browser; pause and seek (2026-11-09)

- **`crates/sex-media`** holds the FFmpeg bridge and the player, so Play and Web share them.
  - `Decoder` reads only through a `Source` trait (`len`, `read_at`, `available`), which can be a file, a fetched body, or bytes still arriving. A growing source is decoded only while 512 KiB lies ahead of the read position (`AHEAD`), or once the whole stream is there.
  - `Player::step()` never waits, so it suits a program's own loop or a browser's event loop. It decodes ahead, feeds the sound ring, and makes each picture current at its time.
  - The player has `play`, `pause`, `seek`, `time_ms`, `set_volume` and `set_muted`.
  - Without a built FFmpeg the crate still builds and refuses every file (`cfg(ffmpeg)`).
- **Sound rings gain `PAUSED` and `SKIP_TO`** (header offsets 40 and 48).
  - `PAUSED`: Bell takes nothing from a paused ring and rests while only paused streams are open.
  - `SKIP_TO`: Bell drops the queued sound, which counts as played, so a seek is heard at once.
  - Bell no longer trusts a WRITTEN count below TAKEN.
- **Play** now uses sex-media.
  - Keys: Space or K pauses and plays; Left/J and Right/L seek 5 s; Home goes to the start.
  - Play is in the boot image as `programs/play`.
- **CI builds FFmpeg**: clang, nasm and musl-dev are installed, and target/ffmpeg is cached, keyed on `scripts/build_ffmpeg.sh`.
- **Gate.** `program_media_gate` adds an 8 s clip:
  - Space pauses, and no pictures are shown for 1.5 s.
  - Space plays again.
  - L seeks from about 1.4 s to 6.4 s, and the next picture is at 6.6 s.
- **Known issue** (reported, not media): PS/2 extended keys (arrows, E0 prefix) don't reach apps, which is why Play also takes J/K/L.

## Web speaks HTTP/1.1: POST, chunked, gzip, cookies, XHR (2026-11-09)

First step toward YouTube (plan: docs/handoff/YOUTUBE_GAP_REPORT_2026_11_09.md).
- **New crate `sex-fetch`** (no_std + alloc, no I/O, host-tested): addresses, HTTP/1.1 requests with any method, headers and body (header injection refused, browser-owned headers like Host and Cookie can't be set by pages), reply heads, Content-Length / chunked / until-close framing (conflicting lengths refused), gzip and deflate (zune-inflate, capped so a gzip bomb stops at the limit), redirects joined per RFC 3986, and an RFC 6265 cookie jar (Domain/Path/Secure/HttpOnly/Max-Age/Expires, `__Host-`/`__Secure-`, per-domain and total caps).
- **`sex_app::net::request`**: sends a `sex_fetch::Request` over the existing flow and returns the decoded reply. The download timeout is now sixty seconds without a byte rather than sixty seconds in all, so long downloads that keep moving finish.
- **Browser**: pages, style sheets, pictures and script loads go through it with the window's cookie jar; `fetch(url, {method, headers, body})`, real `Response.headers`/`url`, `Request`, `Headers` iteration, `XMLHttpRequest` (text, json, arraybuffer, blob; Range works), and `document.cookie` (HttpOnly cookies stay hidden from scripts). New host ops FETCH_HEAD 68 and COOKIE 69; FETCH 66 takes method, headers and body.
- **Gate** `scripts/program_web_http11_gate.py`: a local server sends a chunked + gzip page with Set-Cookie, the page POSTs JSON with its own headers (the server sees all three cookies), then reads 16 bytes by XHR Range (206). Passes in QEMU, along with the web net, fetch and JS gates.
- Seen while testing: a 1.3 MB body takes about 11.7 s in QEMU (about 110 KB/s). That is roughly 360p video speed, so bigger windows and several flows (stage Y3) come next.

## Net: flows made on demand; the browser fetches side by side (2026-11-09)

YouTube stage Y3b (plan: `docs/handoff/YOUTUBE_GAP_REPORT_2026_11_09.md`).
- **sexnet** used to have one open-internet TCP flow for the whole machine. It now makes flows as programs ask for them, each with its own 64 KiB receive window, 16 KiB send queue and TLS session (about 100 KiB), and keeps them for the next caller. The ceiling is 4096 flows, against one runaway program; running out of memory answers `BUSY`, never a fall (allocations use `try_reserve`).
- **Kernel**: BOOT gives sexnet a growing memory arena (`service_memory::reserve(net)`), as Files already had, so its heap follows the flows.
- **smoltcp** gets its `alloc` feature; `Engine::new` no longer takes a fixed socket table, and `open_add_flow` makes a flow.
- A caller numbers its flows itself (0 to 254, in bits 24..31 of the request word, so callers that never name one use flow 0 as before). `CONNECT` takes a free flow or makes one; every other call reaches only the caller's own flow. A program that ends lets go of all of its flows.
- **DNS** is still one question at a time; another caller's question gets `BUSY` until the first is answered (at most 5 s), and the client retries.
- **`sex_app::net::Exchange`**: one request on a flow of its own, read a step at a time (`start`, `poll`, `finish`, `cancel`). `request` and the old `fetch` are built on it.
- **Browser**: script `fetch`/XHR requests run side by side, up to 16 at once, each reported as it lands. Requests made by a script on hearing of one join in.
- **ABI**: `sex_pdx::web` gains `request_on`, `flow_of` and `CLIENT_FLOWS`; the ABI hash is recomputed.
- **Gate** `program_web_http11_gate`: twelve requests that each take 1.5 s finish together in about 1.7 s; the server saw all twelve at once and sexnet made twelve flows.

## Net: connection setup a step at a time (2026-11-09)

- `sex_app::net::Exchange::start` now returns at once with a flow number; each `poll` makes one call to the network service: look up the name, connect, start TLS, wait for the handshake, send, then read. A site whose name, connection or handshake is slow no longer holds up the other requests a program has going (Codex review on #56).
- A program asks the resolver one question at a time (the service answers one per caller); names it looked up are remembered for five minutes (up to 256).
- The old blocking `open`/`send_all` path is gone; `request` and the HTTP/1.0 `fetch` run on `Exchange`.
- Gate `program_web_http11_gate`: a request beside one to an unreachable address (10.0.2.123) lands in about 1.2 s; the unreachable one gives up after 30 s.

## Net: keep-alive, connections reused across requests (2026-11-09)

YouTube stage Y3d.
- **sex-fetch**: requests no longer send `Connection: close`. `Head::keeps_connection(http10)` reads the reply's `Connection` header (HTTP/1.1 keeps unless `close`; HTTP/1.0 only on `keep-alive`).
- **sex_app::net**: when a reply's framing (chunked, `Content-Length` or no body) ends exactly at the received bytes and the server keeps the connection, the flow goes into a per-program idle pool (host, port, TLS; up to 32, dropped after 30 s). The next request to the same site takes the newest kept flow and sends at once, with no connect or TLS handshake. A kept flow that turns out closed before the first reply byte is retried on a new flow. `[net.receive.stats]` shows `reused=` and `kept=`.
- **sex-tls (open web)**: the session is full duplex (`split()` into reader and writer), so one TLS session carries any number of requests. A host test against an OpenSSL server with a test root sends 3 requests on 1 connection (`[sexnet.tls.open.host]`, in `sexnet_jobs_host_gate.sh`).
- **Gate** `program_web_http11_gate`: the page's own server sees 17 requests over 13 connections, and the log shows a reused flow.

## Programs make their own machine code: the base for a JIT engine (2026-11-09)

- **Why.** Andreas approved porting a JIT engine (V8 recommended) and aiming the browser at Chromium-class speed. Report and plan: `docs/handoff/JIT_AND_BROWSER_ENGINE_PLAN_2026_11_09.md` (stages J1-J8).
- **ABI change: `SYS_PROGRAM_CODE` (82).** A program run seals pages of its own heap as code (executable, read-only) or opens them back to data (writable, not executable). Self-only, current run only, at most 256 pages a call. Every core's TLB entry is shot down and the run's page manifest is updated, so retire still checks every leaf. `sex_pdx::program_code`, `kernel/src/app_resources.rs::code`.
- **Code that cannot unlock keys.** Sealed bytes, and the executable segments of every program file at load, may not hold WRPKRU, XRSTOR or XRSTORS at any byte offset. Before this, any native program could rewrite its PKRU and reach every other domain's memory. None of the 20 programs in the image contain the patterns.
- **SDK.** `sex_app::code::CodeBuf`: page-aligned heap pages with `load`, `seal`, `open`, `entry`.
- **Program.** `run jit` compiles an expression to x86-64 and runs it against an interpreter (3 to 9 times faster under QEMU). It shows a constant hiding WRPKRU being refused and re-encoded, and code patched in place. W writes to sealed code and E runs open data; each ends only that run.
- **Gates.** `scripts/program_jit_gate.py` at 1 and 4 cores (also refuses a copy of the program with WRPKRU patched into its code); `scripts/program_code_host_gate.sh` for the byte check.

## C++ programs (JIT stage J5c, 2026-11-09)

- `scripts/build_libcxx.sh` builds libc++ and libc++abi (LLVM 18, no exceptions) for `x86_64-sex-app.json`; CI caches it like FFmpeg.
- `crates/sex-libc` gains what libc++ calls, all in Rust: one "C" UTF-8 locale (`newlocale`, `uselocale`, `*_l` ctype, `strtod_l`/`strtold_l`, `strftime_l`), UTF-8 multibyte and wide-character conversion, `wcslen`, `wmemchr`, `vasprintf`, `__assert_fail`, `__cxa_atexit`/`atexit` (run by `exit`), `.init_array` constructors (`cxx::run_constructors`), and `syscall(SYS_futex)` mapped to program-thread wait/wake.
- `run cpp` checks static constructors, vector/sort, unordered_map<string>, map, virtual calls, std::function, ostringstream, snprintf, std::thread + std::mutex and thread_local: 10/10 at 1 and 4 cores (`scripts/program_cpp_gate.py`).
- Policy: C++ only for ported engines (V8 next), each in its own organ; SexOS itself stays Rust.


## Net: TLS 1.2 and 1.3 with every common suite and curve (2026-11-09)

Parity report workstream G, part 1.
- **Why.** The open-web session spoke only TLS 1.3 with AES-128-GCM over P-256 (embedded-tls). Sites that offer only TLS 1.2, prefer ChaCha20 or AES-256, or ask for X25519 or P-384 failed.
- **sex-tls `open`** now runs rustls 0.23's unbuffered client (no_std) on a provider of our own in `crates/sex-tls/src/open/provider.rs`: AES-128-GCM, AES-256-GCM and ChaCha20-Poly1305 for TLS 1.3 and TLS 1.2 (ECDHE-ECDSA and ECDHE-RSA), SHA-256/384, key exchange on X25519, P-256 and P-384, ALPN `http/1.1`. X25519 is ours (`open/x25519.rs`) on fiat-crypto's verified field code, checked against RFC 7748 and OpenSSL. Certificates still go through rustls-webpki with the same RSA/ECDSA algorithms and the public roots. `Session`'s API is unchanged; sexnet logs `[net.tls.agreed] version= suite= group=` per session.
- **No vector registers in the service.** sexnet builds with `chacha20_force_soft` and `poly1305_force_soft` beside the AES/POLYVAL cfgs; `scripts/check_tls_platform.py` checks them for the sexnet stage and pins the new crates (rustls 0.23.46, aes-gcm 0.10.3, chacha20poly1305 0.10.1, hmac 0.12.1, fiat-crypto 0.2.9, optional under `open-web` so the Browser's graph is unchanged).
- **Tests.** `cargo test -p sex-tls --features open-web`: ten OpenSSL servers each held to one version, suite and group (all nine suites, all three groups, RSA/P-256/P-384 certificates); a server no trusted root signed is refused as a certificate failure; keep-alive. QEMU gate `scripts/program_web_tls_gate.py`: the browser loads a page from a TLS 1.3-only server (agreed TLS13_AES_256_GCM_SHA384 on X25519) and fetches from TLS 1.2-only servers held to ECDHE-RSA-AES256-GCM-SHA384 on P-384 and ECDHE-ECDSA-CHACHA20-POLY1305 on P-256. Its image is built with `SEXOS_TLS_GATE_ROOT` (one extra root, gate builds only).
- **No ABI change.** Live top-site checks need a real network (the cloud proxy re-terminates TLS): try `run +net web https://...` for google.com, wikipedia.org, github.com, cloudflare.com, amazon.com, youtube.com on Andreas's machine or bare metal.

## Net: IPv6 and sockets for programs (2026-11-09)

Workstream G (plan: `docs/handoff/LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md`), the IPv6 and socket half; TLS is separate.
- **IPv6 in sexnet.** smoltcp gets `proto-ipv6`, `proto-ipv6-slaac` and room for three addresses. IPv6 starts with the open side (the first program call), never at boot, so the controlled engine and its gates see no IPv6 frames: the interface is made again with a link-local address from the MAC (EUI-64) and SLAAC on, which sends a router solicitation and takes a global address and default route from the router advertisement. smoltcp 0.14 adds the SLAAC route only to a table with no IPv4 route, so the IPv4 default route steps aside while SLAAC updates. A DHCP lease now replaces only the IPv4 address.
- **NIC.** The e1000/e1000e multicast table passes two groups only: all-nodes (router advertisements) and this MAC's solicited-node group (neighbor solicitations). No multicast-promiscuous mode.
- **Names in both families.** `dns_wire` asks and parses AAAA (`query_type`, `parse_type`, with the same strict validation and CNAME chains). A lookup asks A, and AAAA when the machine has a global IPv6 address; it answers once both came, or 50 ms after the first (RFC 8305's resolution delay), IPv6 first. The resolver itself is still reached over IPv4 (DHCP's).
- **Ready on either family.** `UP` answers OK with a DHCP lease or a global IPv6 address; `[net.ipv6.slaac] address=... router=...` is recorded once.
- **Sockets.** A run Shell bound to the network (`run +net`) can now connect to IPv6 addresses, listen for TCP connections and use UDP, all under the same gate as before. Programs may take ports 1 to 39998 except 68; the ports above are the service's own (its resolver, outbound flows, job tickets), and one port has one listener or socket. A listener takes connections one at a time: on `ACCEPT` the connection becomes the caller's new flow number and another flow listens on. A flow that was shut down is left to finish when closed rather than reset, and a new connection takes a flow the stack is done with first.
- **SDK (`sex_app::net`).** `IpAddr` (V4/V6, `parse` with or without brackets, RFC 5952 display), `up`, `addresses`, `resolve`; `TcpStream` (`begin`/`poll_open` or `connect`/`connect_to`, `read`/`read_within`, `write`/`write_all`, `is_closed`, `shutdown`, `close`), `TcpListener` (`bind`, `accept`, `accept_within`), `UdpSocket` (`bind`, `send_to`, `recv_from`, `recv_from_within`). `Error::InUse` is new. Exchanges (HTTP, so the browser too) resolve in both families, cache both, connect by `CONNECT_TO`, and move to a name's next address when the first fails or takes over 3 s. `sex_fetch::Url` takes bracketed IPv6 hosts (`http://[fd00::1]:8080/`), kept in the Host header.
- **Program `talk`.** `run +net talk tcp HOST PORT TEXT`, `udp HOST PORT TEXT`, `listen PORT`, `addresses`, and `check ADDRESS PORT` for the gate.
- **ABI changes** (`crates/sex-pdx/src/web.rs`; the ABI hash is recomputed): new request kinds `RESOLVE_ALL` 11, `CONNECT_TO` 12, `UDP_BIND` 13, `UDP_SEND` 14, `UDP_RECEIVE` 15, `LISTEN` 16, `ACCEPT` 17, `ADDRESSES` 18, `SHUTDOWN` 19; new status `IN_USE` 8; region layouts `ADDRESS_BYTES` 17 (family 4 or 6, then 16 address bytes) and `ENDPOINT_BYTES` 19 (address, then port big-endian); `FAMILY_V4`, `FAMILY_V6`, `PROGRAM_PORT_MAX`, `put_address`/`address_of`/`put_endpoint`/`endpoint_of`. Kinds 1 to 10 are unchanged (`RESOLVE` still answers 4 IPv4 bytes; `CONNECT` still packs IPv4 and port), so Kaleidoscope and Spindle are untouched. `STATE` now says OPENING for a listener and OPEN after our own FIN.
- **Tests.** Host (`sexnet_transport_host_gate.sh`): router solicitation and SLAAC address and route, no IPv6 before the open side starts, TCP to an IPv6 peer with shutdown, UDP port rules and echo, a listener taking a connection, A+AAAA resolution with the delay, AAAA wire parsing; `sex-fetch` bracketed URLs. QEMU gate `scripts/program_ipv6_gate.py`: on QEMU's socket backend with a peer the script plays itself (router, neighbor discovery, small TCP/UDP echo and HTTP server, a TCP client), because the build hosts have no IPv6 for QEMU's user network. The machine takes `fd00:5e:0:1:5054:ff:fe12:3456` by SLAAC, `talk` echoes over TCP and UDP and accepts a connection on 7070, and `peek http://[fd00:5e:0:1::1]/page.html` loads a page.
- **Not yet.** DNS over IPv6 and DHCPv6/RDNSS (the resolver is DHCPv4's), IPv6 fragmentation and MLD, a listen backlog beyond one, UDP datagrams past one frame, TLS to IPv6 literals (the name check takes a host name).
## Ordinary Rust and C programs: Rust std, POSIX as a guest library, PDF (2026-11-09)

Workstream H of `LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md`. PR #68.
- **Rust `std` target: `x86_64-sex-std.json`.** Programs link the real standard library, built from rust-src (`build_std = "std,panic_abort"` in a spec stage). The target names Hermit's library interface (`os = "hermit"`): upstream std already calls `sys_read`, `sys_spawn2`, `sys_futex_wait` and so on instead of trapping into a POSIX kernel, so no std fork is needed. Static PIE, only `R_X86_64_RELATIVE`, local-exec TLS; the loader takes it unchanged.
- **`crates/sex-std`** answers those calls inside the program, over its own grants: files in the program's folder, which is `/` to it (`..` never leaves; `path::resolve`); stdout/stderr to the window as a console and the serial record (`[std.out]`, `[std.err]`), stdin as lines typed into the window; threads as workers (`sex_app::thread`), `Mutex`/`Condvar`/parking on the kernel's wait/wake; `Instant` from the tick; memory from the program heap; `argv` from Shell's argument line (`args::split`, double quotes keep a word). After `main` returns, a console that printed stays until it is closed, with how the program ended. `sex_std::window()` hands the window to a program that draws.
- **Not yet:** `std::net` says `Unsupported` until the program socket API (sockets thread); `SystemTime` is build time plus uptime until programs get a read-only civil clock; Hermit's std refuses `fsync`, `rename` and `set_len`.
- **sex-app:** `files::direct` (no window needed: stat, read_at, write_at, create, truncate, list, mkdir, remove, rename, sync, args) for any task; `mailbox` keeps messages one task took from the shared inbox for another (a worker's file answer seen by the main task, keys seen by a worker); one file request in flight per program.
- **POSIX guest library:** on the std target, `sex-libc` exports `posix.rs`: descriptors (open, read, write, lseek, close, stat/fstat/lstat, unlink, mkdir, rmdir, rename, remove, access, ftruncate, fsync, isatty), buffered `FILE` streams (fopen/fdopen/freopen, fread/fwrite, fgets/fgetc/ungetc, fputs/puts/putchar, fseek/ftell, feof/ferror, printf family to any stream), `opendir`/`readdir`, `exit`/`atexit`/`abort`, sleep. All of it is library code over `sex-std`; nothing is in the kernel or a service. The no_std program target keeps its callback-only C library. `sex-std` adds `sys_rename`, `sys_fsync`, `sys_ftruncate` for it.
- **Programs:** `run stdhello` (Rust std: HashMap, fs, threads, mpsc, time), `run chello` (an `int main(argc, argv)` C program: stdio, files, folders, errno, qsort), `run pdf [NAME.pdf]` (hayro renders pages; Right/Left, Home/End, Up/Down, +/-/0; writes and opens `welcome.pdf` when given no name).
- **Gate:** `scripts/program_std_gate.py`, now also a CI step.
- **ABI:** no kernel ABI change; no `ABI_INPUTS` file changed, `abi_version_hash` unchanged.

## Files: USB sticks and other systems' disks, the Files app, an open/save picker (2026-11-09)

Workstream I of `docs/handoff/LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md`.
- **New crate `sex-volumes`** (no_std + alloc, no outside crates): MBR (with logical partitions) and GPT tables, and four filesystems behind one `Volume` trait that is lent the disk on every call, so it can sit on a file, a partition or (next) a USB stick.
  - **FAT12/16/32, read and write**, with long names, case flags, every FAT copy kept equal, FSInfo, directories that grow. Writes are ordered so a pulled stick loses at most the change in hand.
  - **exFAT, read and write**: up-case table lookups, set checksums and name hashes, the allocation bitmap, contiguous (NoFatChain) files that turn into chains when they must, ValidDataLength.
  - **ext2/3/4, read only**: 32/64-bit group descriptors, meta_bg, extent trees of any depth, classic block maps up to triple indirect, holes and unwritten extents, inline data, htree folders. Unknown incompatible features are refused.
  - **NTFS, read only**: fixups, the `$MFT`'s own runlist, resident/non-resident attributes, `$ATTRIBUTE_LIST`, `$I30` index trees with `$UpCase` lookups (DOS aliases open too), sparse files, initialized size. Compressed and encrypted files list but don't read.
- **Evidence** (`scripts/volumes_host_gate.sh`, now in CI): 37 host tests on images made by mkfs.vfat/mtools, mkfs.exfat/exfat-fuse, mke2fs and mkntfs/ntfs-3g (fixtures in `tests/volumes/`, scripts in `scripts/volume_fixtures/`). Our FAT and exFAT writes pass `fsck.vfat -n` / `fsck.exfat -n` and read back through mtools / exfat-fuse, including a 400-step random model with remounts. ext and NTFS readers survive byte-flip fuzzing without a panic.
- **Files service**: volumes appear under `/media/NAME` (the label, else the image's name). Today a volume comes from a disk image on the SexOS filesystem (a loop mount: its writes are batched and committed like any file's, so copy-on-write keeps a whole image at every commit); USB mass storage, when the USB thread lands it, is one more source. A mounted image can't be written, truncated or removed under its volume.
- **Spindle**: `mount IMAGE`, `unmount NAME` ("Safe to remove."), `df PATH`, and `ls`/`cat`/`write`/`rm`/`mv`/`mkdir` work under `/media`. `mv` between disks answers "on another disk". The old `files` status text is now `files-status`.
- **Folder grants**: `run NAME @DIR` gives a program a folder, and everything in it; `@/` is the whole filesystem. `files` in Spindle runs the Files app with `@/`, `files DIR` with that folder.
- **Files app** (`apps/files`, installed as `/bin/files` like any program): browse, Ctrl+C/X/V copy, cut and paste (across disks it copies and removes), F2 rename, Ctrl+N new folder, Delete moves to the same disk's `.Trash` (with where it came from), Ctrl+T shows the trash, Ctrl+R restores, Ctrl+Z undoes step by step, Ctrl+D disks, Ctrl+H home, free space in the title line.
- **Open/save picker, a portal**: a program calls `sex_app::files::pick(Open|Save|Folder, suggestion)`. Files (the service) holds the question; the Files app, which the person gave the whole filesystem, shows "jot asks to open a file" and the person picks. The program then holds exactly that one path (`@/...`) for the rest of its run, nothing else. Only a run holding `@/` (or Spindle) can see or answer questions, and it can only give out a path it holds itself. Jot: Ctrl+O opens through the picker, Ctrl+Shift+S saves as.
- **ABI** (hash recomputed): `fs::MOUNT` 15, `UNMOUNT` 16, `PICK` 17, `PICKED` 18, `PICKS` 19, `ANSWER` 20, `PICK_OPEN/SAVE/FOLDER`, `PICK_HINT_MAX`, `MEDIA`; statuses `OTHER_DISK` 13, `READ_ONLY` 14, `UNSUPPORTED` 15, `PENDING` 16. `@` grants now cover what is inside a granted folder. `sex_app::files` gains `stat`, `list`, `mkdir`, `create`, `remove`, `rename`, `truncate`, `write_at`, `space`, `pick`, `questions`, `answer`.
- **Gate** `scripts/program_files_gate.py` (QEMU): a FAT12 stick image and an ext2 Linux disk are fetched and mounted; the stick is listed, read and written from Spindle; writing to the Linux disk is refused as read only; the Files app copies HELLO.TXT from the stick to `/`, trashes it and undoes; Jot asks for a file, the person picks one on the stick in Files, Jot edits and saves it there and is still refused outside it; the stick unmounts. Also `program_jot` and `filesystem` again.
- **Limits**: no clock reaches Files, so new FAT/exFAT entries are dated 1980-01-01. Sticks arrive as images until USB mass storage exists. ext4 and NTFS don't write yet. Programs can't run from `/media`. The trash never empties itself, and the undo history lasts as long as the Files window. The picker needs the Files app open (the program waits until it is). Files is text, keyboard only; no icons or mouse yet.
## USB: keyboard, mouse and tablet together, hubs and hot-plug (2026-11-09)

- **What runs now.** SexUSB drives any number of devices at once (up to 32), on root ports
  and behind hubs: two keyboards, a mouse and a tablet behind a hub all work together, and
  unplugging or plugging in a device (or a whole hub) while running works. Keyboards send
  all six keys a USB keyboard reports, so overlapping keys while typing fast are no longer
  lost, and every key has a code: digits, punctuation, arrows, F1 to F12, keypad, Insert,
  Delete, Home, End, Page Up and Down (arrows and the rest arrive as the same codes the
  PS/2 keyboard gives). Mice are read through their HID report descriptor, so wheels,
  report IDs and 16-bit motion work, not only the 3-byte boot report.
- **Design.** The single-device, proof-only driver (one 4,800-line function) is replaced by
  three parts: `xhci.rs` (controller, rings, commands, control transfers; events for other
  endpoints that arrive while waiting are kept for the main loop), `usb.rs` (enumeration,
  hubs with route strings and transaction translators, hot-plug, HID) and `hid.rs` (report
  descriptor parser, host-tested). For real PCs: the firmware handoff (USB Legacy Support),
  scratchpad pages, 64-byte contexts and USB 2/3 port detection from the Supported Protocol
  capabilities. Keys from every keyboard merge into one snapshot for Input, and a device
  that leaves releases what it held. The USB DMA region is now 2 MiB (was 256 KiB).
- **ABI.** `OP_USB_KEYBOARD_REPORT` arg2 now carries up to six pressed usages, one per byte
  (byte 0 is the old single key, so the first key reads as before).
- **Limits.** Polled, no MSI yet. USB 3 hubs are written but QEMU has none to test with.
  No mass storage, audio or video class yet. Keyboard LEDs (Caps Lock) are not set.
  Only the first pointer collection of a device is read (no touch screens, no pens).
- **Gates.** `usb_path_gate` lanes 1 to 4 unchanged and a new lane 5 (hub, two keyboards,
  mouse, tablet, rollover, hub unplug, mouse plugged in later); `usb_wheel_host_gate` with
  a rollover test and the descriptor parser's tests; master runtime gate; host unittests;
  architectural truth.

## Power off and restart through ACPI (2026-11-09)

Parity report workstream A (`LINUX_AND_DESKTOP_PARITY_REPORT_2026_11_09.md` section 6).
- **Kernel** `kernel/src/hal/acpi.rs` replaces the stub. Fixed hardware comes straight from the FADT and never needs AML: PM1 event/control blocks, the reset register, the SCI, ACPI mode (SMI_CMD), HW-reduced sleep registers. `\_S5` is read from the DSDT bytes by a bounds-checked reader (`sex_pdx::power::s5_from_dsdt`) and, when AML runs, from the interpreter.
- **AML in quarantine.** The `acpi` crate's interpreter runs on its own 256 KiB stack. If firmware AML makes it panic, the panic handler returns to the ACPI code (`sex_aml_enter`/`sex_aml_escape`), AML is set aside for that boot and shut down/restart keep working from the tables. A boot self-test proves the escape before any firmware AML runs (`[acpi.aml.quarantine] self_test=1`). `\_PIC(1)`, `_INI`/`_STA`, `\_PTS(5)` run there.
- **Shut down**: `\_PTS(5)`, SLP_TYPa/b then SLP_EN on PM1a/PM1b (or the HW-reduced sleep control register). **Restart**: FADT reset register, then port 0xCF9, the keyboard controller, a triple fault.
- **Power button**: fixed event via the SCI (vector 0x29, level/active-low unless the MADT overrides). GPEs are masked until there is code for them, so an unhandled GPE cannot hold the SCI asserted.
- **ABI change: `SYS_POWER` (84)** and capability `CapabilityData::Power(sex_pdx::power::Grant)` at slot 62. BOOT grants Shell `REQUEST|SENSE`, Files `FLUSH`, the status bar `SENSE`. Ops `INFO`, `OFF`, `RESTART`, `READY`, `SENSORS`. ABI hash recomputed.
- **Files first.** A shut down or restart (from Shell or the power button) sends Files `power::OP_PREPARE` (kernel message, pd 0); Files commits waiting writes, flushes the disk cache and answers `READY`. The kernel goes ahead after `FLUSH_TICKS` (about 5 s) if Files never answers.
- **Desktop**: launcher rows "Shut Down" and "Restart" (`Command::ShutDown`, `Command::Restart`), shown unavailable where the kernel cannot do them; the palette batch proof skips them.
- **Sensors and devices**: batteries (PNP0C0A `_BST`/`_BIX`/`_BIF`), AC (ACPI0003 `_PSR`), lid (PNP0C0D `_LID`), thermal zones (`_TMP`), read through `SENSORS`. I2C HID devices (PNP0C50/ACPI0C50) are listed at boot for a later driver (`[acpi.i2c_hid]`). Known limit: the `acpi` crate 6.1.1 has no custom region handlers, so laptop batteries behind an Embedded Controller read as absent until we add an EC handler (patch or vendor the crate).
- **CPU power**: Intel HWP (IA32_HWP_REQUEST, balanced EPP) or AMD CPPC on every core, so the CPU picks its own frequency. Idle uses MWAIT into the deepest `_CST` FFH state with exit latency at most 200 us, only where the APIC timer keeps running in deep idle (ARAT); otherwise HLT (C1).
- **Gates**: `scripts/power_acpi_gate.py` (QEMU `system_powerdown` ends the VM with exit 0 after Files flushed; launcher Restart resets through the FADT register and the machine boots again; launcher Shut Down ends it; 1 and 4 cores) and `scripts/power_host_gate.sh` (ABI encodings, EISA ids, `\_S5` reader). Both run in CI.
- **Bare metal next week**: check `[acpi.power]` on the PKU PC: s5=1, reset method, power_button=1, aml=1 quarantined=0, idle state and hwp/cppc.

## Real PCs: wired network, HD Audio and SATA (2026-11-09)

Workstream D of the parity report. Handoff and bare-metal checklist: `docs/handoff/REAL_PC_DRIVERS_2026_11_09.md`.
- **Leases.** BOOT now leases Intel e1000e/I217-I219, igb (82576, I210/I211, I350), igc (I225/I226) and Realtek r8169/r8125 to SexNet (Realtek registers in BAR2/BAR1); Intel HD Audio (class 04:03, any vendor) to Bell at slot 20 with its own DMA pool; AHCI (class 01:06:01) to the drive service at slot 21. `kernel/src/device_lease.rs`, `kernel/src/nic_boot.rs`.
- **ABI.** `sex_pdx::pci_command::Model` gains I219, I82576, I210, I350, I225, Rtl8168, Rtl8125 plus `Family`; new `HDA_DMA_*`, `AHCI_DMA_*`, `SLOT_AHCI_HOST`; kernel `PciCapData.class`. `abi_version_hash` recomputed. `[bell.sound.ready]` gains `card=`.
- **Sound.** Bell drives HD Audio (`servers/sexbell/src/hda.rs`): CORB/RIRB, analog pin to converter routing (speaker, headphones, line out; never HDMI), one always-running 48 kHz stereo stream over 16 periods. Gate `sound_gate.py --card hda`: 440 Hz/400 ms and 1000 Hz/310 ms recorded.
- **Network.** igb/igc advanced descriptors and queue registers, igc DEV_RST, PHY autonegotiation restart, I219 ich8lan bits, DRV_LOAD; Realtek in `servers/sexnet/src/realtek.rs`. Gates `program_net_gate.py --nic igb|e1000e` (DHCP, HTTP download), `ethernet_peer_gate.py --model igb`; Realtek and igc host-tested only (QEMU lacks them).
- **SATA.** `apps/sexdrive/src/ahci.rs` serves the block interface when no NVMe is present. `SEXOS_GATE_DISK_BUS=ahci` for gates; filesystem and disk persistence gates pass on SATA and NVMe.


## Hardware census, VMD warning, NVMe waits for real SSDs (2026-11-09)

- **What runs now.** Every boot prints one `[hw.device]` line per PCI device with its
  vendor, model, owning service and support level (driven, firmware, platform,
  in_progress, planned, none), `[hw.warning]` lines for things a person must act on
  (Intel VMD/RST RAID hiding the NVMe drives, an NVIDIA GPU with no native driver, Wi-Fi
  with no driver) and a `[hw.census]` summary. On the QEMU gate machine: 8 devices, all 8
  working.
- **NVMe on real SSDs.** Controller disable/enable now waits up to CAP.TO (500 ms units)
  in timer ticks instead of a fixed poll count (a few ms on bare metal), gives up at once
  on a fatal status while enabling, and the admin completion doorbell honours the
  doorbell stride. A namespace with 4 KiB sectors is logged as unsupported. The first
  NVMe controller is the system disk; later ones are logged, no longer replacing it.
- **Design.** Device knowledge is data in `crates/sex-hwdb`, shared by BOOT now and by
  driver manifests later (plan stage D2 in `DRIVERS_AND_X17_R1_PLAN_2026_11_09.md`).
  The table never leases a device.
- **ABI.** None; `abi_version_hash` unchanged.
- **Gates.** `scripts/hwdb_host_gate.sh` (CI), `tests/sexdrive_controller_ready.rs` in the
  sexdrive DMA host gate, master runtime gate GREEN.
- **Limits.** One NVMe controller; no VMD driver, MSI-X or IOMMU yet. On a laptop the
  census is read from the framebuffer boot log (PR #78).

## Driver manifests, many devices per service, MSI-X interrupts (2026-11-09)

Stages D2 and D3 of `DRIVERS_AND_X17_R1_PLAN_2026_11_09.md`. Handoff: `docs/handoff/DRIVER_MANIFESTS_AND_MSIX_2026_11_09.md`.
- **Leases by manifest.** Each driver service's match table (class, vendor, device, lease kind, first slot, vector count) is data in `crates/sex-hwdb/src/manifest.rs`; `devmgr` only builds leases of each kind. New controllers are a data change, not a kernel edit.
- **Many devices per service.** First device of a kind keeps its old slot (NVMe 16, xHCI 8, NIC 19, sound 20, AHCI 21); more land at slots 160..191. Two NVMe drives now lease side by side.
- **MSI/MSI-X.** BOOT programs MSI-X (or MSI) vectors 0x50..0x8F, masked, INTx off. An interrupt sets a pending bit for the owning service and wakes it (no ring messages, no overflow). sexdrive's NVMe completions are interrupt-driven: `[sexdrive.nvme.irq] mode=msix polling=0`.
- **ABI.** Syscall 86 `SYS_DEVICE` (list own leases, unmask/mask own vector, take pending bits); `sex_pdx::device`; syscall 70 also returns on a pending interrupt bit. `abi_version_hash` recomputed.
- **Gates.** `hwdb_host_gate.sh` with manifest tests; filesystem gate on MSI-X NVMe, also with a second NVMe; sound, network, USB and master runtime gates.
- **Limits.** Only NVMe uses interrupts so far; vectors go to the boot core; no interrupt remapping until the IOMMU; second-device drivers are follow-ups.

## Design guardrails: SexOS stays a microkernel SASOS, not a slow Linux (2026-11-09)

- **What changed.** An audit of master and PRs #68 to #84 against the sex-manual
  (`docs/handoff/LINUX_DRIFT_AUDIT_2026_11_09.md`). New binding rules in
  `docs/DESIGN_GUARDRAILS.md`: the kernel keeps mechanism only, no Linux-shaped kernel
  API, compatibility layers are libraries, Linux drivers are temporary Tuxedo bridges with
  a tracked native (REA) replacement. `docs/BRIDGE_LEDGER.md` lists every bridge (none
  today), every native driver that used Linux/BSD as reference, device firmware and
  third-party ring-0 code.
- **Removed from the kernel.** Unreachable `sys_fork`/`sys_execve`/`sys_pipe`/
  `sys_translate_driver` ("load Linux driver")/VFS stubs, unused `benchmark.rs`, seven
  uncompiled directories (desktop, input, network, ...), backup files, and nine `std`
  phase tests that never compiled. Kernel builds; 18,383 non-blank lines.
- **Gate.** `scripts/tests/test_kernel_design_guardrails.py` (host unit tests in CI):
  kernel deps exclude compat crates, no fork/pipe/socket-named kernel files, no backups in
  `kernel/src`, kernel line budget 21,500, and a ledger row for any GPL-2.0-only or
  Tuxedo-using component.
- **Open decision.** #70/#82 run firmware AML in ring 0; the audit recommends a ring-3
  `sexacpi` server.

## Discrete GPU off at boot (NVIDIA N1, 2026-11-09)

Plan: `docs/handoff/NVIDIA_GPU_PLAN_2026_11_09.md` (stages N1-N6).
- **Kernel** `kernel/src/hal/dgpu.rs`: every display controller is named at boot (`[gpu.found]`), NVIDIA chips from their own `PMC_BOOT_0` register (GA104, GA106, ...). A GPU behind a root port that does not hold the boot framebuffer is powered off like Linux runtime PM: D3hot for each function, the GPU's `_PS3` and `_PR0`..`_PR3` resources off, then the root port's when the GPU is alone behind it (D3cold). Shared resources stay on. `power_on` restores power and each configuration header for the later driver.
- **AML work budget**: `acpi::with_aml(budget, f)` runs firmware AML quarantined with a limit on register accesses and stalls, so a firmware loop cannot hang the boot.
- **Gate** `scripts/dgpu_power_gate.py` in CI: QEMU root port + display controller + SSDT power resource (`tests/acpi/dgpu_gate_ssdt.asl`); checks the method order off/on/off on the debug console, the restored header, the boot GPU kept and the desktop up.
- ABI: none.
## Net: ALPN http/1.1 (2026-11-09)

- The open-web TLS session names `http/1.1` in its handshake (ALPN), as browsers do (the rustls move set `alpn_protocols`). Host test `[sexnet.tls.open.host]` now proves it on the wire: an OpenSSL server offering `h2` and `http/1.1` must pick `http/1.1`. YouTube stage Y3e.
