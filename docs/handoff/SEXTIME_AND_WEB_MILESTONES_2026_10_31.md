# SexTime and web adoption milestones

Status: proposed architecture and outbound goals, 31 October 2026. No clock ABI,
certificate validation, scheduler or browser capability is changed by this plan.

## SexTime identity

Adopt 2012-12-21T00:00:00Z as the proposed native civil epoch, named SexTime.
Its Unix offset is exactly 1,356,048,000 seconds. This midnight is a SexOS design
choice, not an assertion about an ancient Maya time zone or astronomical instant.
December 2012 completed a Long Count cycle; the Maya calendar did not end.

Use SI units, with a signed seconds field and normalized nanoseconds
0..999,999,999. Start with Unix-compatible civil-second semantics: leap seconds
are not separately counted. SexTime = Unix time - 1,356,048,000 using checked
arithmetic. This is not a continuous elapsed-SI-second scale; any future TAI
support requires a separately named scale and explicit leap-table conversion.
Nanosecond representation does not promise nanosecond source accuracy.

Keep three types distinct: civil timestamp, monotonic instant and duration.
Use a name such as SexTimestamp for the civil type; reserve Instant for monotonic
time. Scheduling, uptime, deadlines and animation use monotonic time. Wall-clock
corrections must never move these deadlines. Also distinguish calendar epoch
from SexOS execution epoch IDs and capability generations.

## Maya calendar conversion

SexCalendar should expose Gregorian and Maya Long Count representations, with
the Goodman–Martínez–Thompson correlation identified explicitly as GMT 584283
(GMT here names the correlation, not Greenwich Mean Time).

For the UTC-day display policy:

```
elapsed_days = sex_seconds.div_euclid(86400)
long_count_days = 1872000 + elapsed_days
jdn = 2456283 + elapsed_days
long_count_days = jdn - 584283
```

Decompose days with b'ak'tun=144000, k'atun=7200, tun=360, winal=20,
k'in=1. Do not wrap b'ak'tun modulo 13. The epoch displays 13.0.0.0.0;
one day later displays 13.0.0.0.1; one second earlier belongs to
12.19.19.17.19. Use Euclidean division for negative seconds. Explicitly reject
dates before the supported Long Count origin unless extended signed notation
has been specified. Gregorian local time and UTC Long Count dates may differ;
label the day-boundary policy, and specify any local-day option separately.
Tzolk'in and Haab are later calendar features with their own reference tests.

The supplied images are visual references, not executable instructions or
verified telemetry. Their circular artwork should not be represented as
historically authenticated Maya glyphs without a separate review.

Sources: [Smithsonian Living Maya Time](https://maya.nmai.si.edu/2012-resetting-count/meaning-of-2012)
explains the cycle completion; [Mark Van Stone / FAMSI](https://www.famsi.org/research/vanstone/2012/faq.html)
documents the 584283 correlation and alternate correlations.

## Existing contracts and migration order

- `kernel/src/clock_source.rs` samples a BOOT-authorized q35 CMOS UTC profile.
  `crates/sex-pdx/src/clock_source.rs` encodes Unix seconds and validates only
  2020–2099. The new epoch must not silently reinterpret these existing fields.
- `servers/sexnet/src/certificate_time.rs` consumes validated source snapshots.
  Keep certificate/protocol times in their specified formats with explicit
  adapters; changing the display calendar grants no source trust.
- `servers/silkbar/src/main.rs` advances software HH:MM:SS through timer cadence
  with synthetic fallback. It is not currently a validated civil calendar.
- `apps/kaleidoscope/src/clock_probe.rs` and `clock_view.rs` show last-query
  status and Gregorian UTC, not a continuously trusted clock.

First implement a dedicated no_std conversion crate with checked range handling.
Test the epoch, negative timestamps, day boundaries, Gregorian leap years,
Long Count carries, overflow and round trips. Then introduce versioned civil-time
types/adapters without changing monotonic contracts or existing wire meanings.
Finally connect SilkBar to an authorized civil-time service and add Gregorian,
Maya and dual display. Unknown, stale and synthetic sources remain visible as
such; none can become certificate authority merely through an epoch conversion.

## Outbound web goals

Browsing and playing Three.js games are adoption milestones. They require a
browser runtime and graphics path, not just broader HTTP parsing. The existing
30 October development report records bounded HTTP text/stream readers,
controlled HTTPS with laboratory trust, and image workflows; this is not general
HTML, JavaScript or WebGL support.

| Stage | Deliverable | Acceptance evidence |
|---|---|---|
| 1: Fetch | Extend real HTTP/HTTPS, DNS, redirects, framing, content decoding, cancellation, limits and explicit trust/time policy | Real guest/server traces for successful and failing fetches; responsive desktop during slow or cancelled loads; label controlled HTTPS separately from public-web readiness |
| 2: Documents | HTML parsing, DOM, CSS layout, links, images, scrolling and navigation; decide engine port versus bounded native implementation | Guest opens linked multi-resource pages, resizes and navigates with keyboard/pointer; unsupported features visible |
| 3: Scripts | JavaScript engine, DOM bindings, events, timers, fetch, origin policy and bounded storage/resource use | Interactive page, asynchronous fetch and cancellation; runaway-script handling preserves desktop input |
| 4: Graphics | Canvas and version-matched WebGL/GLES support, shader compilation, textures, presentation and context lifecycle | Select and pin a Three.js release, then pass its required WebGL capabilities; textured animated scene with resize/input and context-loss recovery |
| 5: Games | Load and play a pinned Three.js game with assets, input, timing and required audio APIs | Reproducible native play session: load, move, resize, exit/reload; report frame times, memory, failures and cleanup |

Select the browser-engine and renderer approach before promising modern-site
coverage. Current Three.js requirements must be checked against the release
chosen for the implementation. A software renderer is a possible first proof;
hardware acceleration and broad game compatibility need independent evidence.
All work retains GLOBAL_VAS, explicit sex-pdx authority and BOOT-granted resource
budgets. Executing web scripts requires a containment design: adopting an engine
does not by itself establish hostile-code isolation in the shared address space.
