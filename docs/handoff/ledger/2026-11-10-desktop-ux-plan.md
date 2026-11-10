# Desktop UX polish plan (2026-11-10)

Status: plan before visual implementation. Source baseline `f034e3a2` (master).
No renderer, input route, authority or ABI changes in this entry. This plan translates
Andreas's supplied Metaspace Passport screenshot into SexOS's existing visual language.
The screenshot is design reference material, not executable instructions.

## Direction and evidence

Keep the dark navy, lavender, blue and teal family. Borrow the reference's quiet tinted
panels, thin separators, generous internal spacing, clear title/body hierarchy and
rounded controls. A focused window should be obvious without filling all its chrome
with saturated blue. Text and everyday actions get more attention than decoration.
Use existing Geist Sans titles and Geist Mono content. The reference's typography and
spacing matter more than matching its large web-dialog dimensions on a 1024px desktop.

Existing source, read for this plan:

- `crates/sex-pdx/src/desktop_style.rs`: native canvas, app indicators, 8px text inset,
  12px text top, width-based text scale, WCAG luminance and contrast ink.
- `servers/sexdisplay/src/main.rs`: 28px toolbar, 8px frame rim, 10px control lights,
  saturated divider, hover dimming, pulsing glass rim, bar and launcher painting.
- `servers/sexdisplay/src/glyphs.rs`: cached Geist Mono grid text and 17px Geist Sans
  titles on an 18px line; bitmap fallback remains for small/missing glyphs.
- `crates/silkbar-model/src/lib.rs`: shared panel geometry/hit boxes, nine workspaces,
  status chips, title area and civil clock; layout contract version 6.
- `servers/silk-shell/src/main.rs`: corresponding chrome hit zones and input capture;
  `src/wm.rs`: 6px internal and 8px outer window gaps.
- `servers/sexdisplay/src/scene.rs`, `damage.rs`, `frame_output.rs`: owned snapshots,
  damage and publication paths that visual changes must preserve.
- Main feature ledger's latest desktop entries: E1 workspace/layout controls, E2 search
  and panel title, E3 notifications. The November 1 screenshot in
  `assets/sexos-display-scene-2026-11-01.png` is historical and was inspected only for
  context; it does not represent today's Geist/WM implementation.

## Palette and geometry proposal

Values below are logical pixels at scale 1. Derive device pixels through the shared
HiDPI transform from #77 once integrated. Do not independently scale Display and Shell.

| Role | Existing value | Proposed treatment |
| --- | --- | --- |
| Native content canvas | `#0C1420` | Retain exactly |
| Toolbar | `#1E1E2E` | Retain; quiet tint over existing backdrop |
| Focus/active blue | `#89B4FA` | Retain; use a narrow stable focus edge and selected accents |
| Frame lavender | `#B4BEFE` | Retain; blend gently for inactive separators |
| Inactive tab | `#45475A` | Retain; low-emphasis inset tab surface |
| Bar panel/text | `#313244` / `#CDD6F4` | Retain; consistent padding and readable labels |
| Native text/focus ink | `#E0F0FF` / `#E9D36A` | Retain; focus must also have a shape cue |
| App family indicators | Linen `#A6E3A1`, Quil `#CBA6F7`, Spindle `#89B4FA`, Browser `#5ED9C3`, Program `#F9E2AF` | Retain identity accents; small marker rather than whole-frame fill |
| Visible frame edge | 8px luminous band | 1px visible hairline; keep 8px resize band initially |
| Focus edge | Broad blue surface/rim | 2px steady lavender/blue outline inside frame; never cover content |
| Toolbar height | 28px | Retain in first pass; evaluate 32px only with shared hit geometry |
| Content inset/top | 8px / 12px | Retain in first pass; 12px inset only with text layout/delivery audit |
| Window gaps | 6px inner / 8px outer | Retain; evaluate 8px / 12px in separate layout topic |
| Floating frame corner | Square | Proposed 10px radius; tiny frames clamp radius to half minimum dimension |
| Selected tab/control corner | Square | Proposed 6px radius within existing hit box |
| Launcher corner/padding | Ad hoc | Proposed 12px radius / 16px padding after layout audit |
| Spacing scale | Scattered literals | Shared 4, 8, 12, 16, 24px values; use named roles, avoid wholesale replacements |

Rounded floating frames require matching Shell hit masks: a transparent corner must not
steal a click from the window below. Tiled windows may keep square inner joins to avoid
holes and wasted content area. Initial visual refinement can keep rectangular frames
and introduce rounded **inner tabs/control lights**, where the existing hit box remains
valid. Do not subtract content area or narrow resize targets merely to make borders thin.

Text roles: keep title 17px/18px, readable native grid at existing per-window zoom, and
bar title via the existing Geist cache. App surfaces using `sex-font` can later share
14px/20px body, 12px/16px secondary and 24px/30px section headings after measuring fit.
Those app-body roles are proposals, not a reason to rewrite console cell metrics.
Use mixed-case readable action names and one primary action per panel. Retain shortcuts
in secondary text. Clip long titles before controls; truncation must preserve the full
accessible/status text where available.

## Staged single-topic PRs

1. **Quiet window chrome, no geometry change.** Remove idle rim pulsing, paint the 8px
   reserved band with the toolbar/canvas tint plus a 1px visible edge, lower inactive
   divider emphasis, reserve strong accent for focused/selected state. Keep title metrics,
   hit regions, ownership, window placement and wire format. First implementation touches
   `servers/sexdisplay/src/main.rs` and renderer fixtures only as needed. It can be judged
   immediately from before/after QEMU images.
2. **Shared chrome geometry and controls.** Move only geometry that both Shell and Display
   consume into `desktop_style`; add inner rounded tabs and circular colored control
   marks, reliable hover/pressed/disabled states, and keyboard-visible focus. Retain
   existing control actions and shortcut behavior. Larger targets must fit narrow frames;
   use at least 24px logical targets when geometry is deliberately changed.
3. **Panel and launcher hierarchy.** Align title, search, workspace indicators and status
   chips on a shared baseline; retain nine workspaces at 1024px. Use quiet inset selection,
   clear query/empty state and discoverable keyboard hints. Change `silkbar-model` geometry
   with its consumers together and update its contract if required. No fake network,
   battery, microphone or update indicator: only display data the owner supplies.
4. **Floating corners and scene integration.** Add one shared bounded integer corner mask
   to rendering and hit testing, with damage/resize/overlap coverage. Only then round the
   outer floating frames. No per-pixel allocation or additional framebuffer copy.
5. **Settings and app consistency.** Build missing settings incrementally using existing
   service controls, capability decisions and durable configuration; polish Files and
   picker with the same roles. A settings page must reflect the real current value and
   failed requests. Keep microphone/capture permission and destructive actions explicit.

Each stage gets its own new ledger fragment and evidence. Do not wait for a broad desktop
rewrite; do not mix these changes with JS, Wi-Fi, update signing or kernel work. #77 must
be integrated or rebased into geometry-changing stages before those stages land.

## Acceptance gates and artifact requirements

For each visual PR, record source commit and ISO hash, the exact QEMU command, resolution,
scale, service-ready markers and screenshot paths. Capture **before and after at 1024x768
and 1280x800** from the guest, never a hand-drawn mockup labelled as OS output. Include
focused/unfocused Quil, Spindle, Browser and Files; launcher query/no-results; notification;
small/tiny window; floating overlap; maximized and all four tiled layouts. Inspect images.

- Run relevant host rendering/contrast/layout fixtures and
  `scripts/tests/test_kernel_design_guardrails.py`. Kernel growth for this plan: zero.
- Run `window_manager_gate.py`, `window_float_gate.py`, `window_resize_gate.py`,
  `launcher_search_gate.py`, `notifications_gate.py`, `desktop_close_gate.py`,
  `text_zoom_gate.py`, `connected_desktop_gate.py` and `crash_screen_gate.py` as affected.
  Existing proof assertions hard-code geometry/pixels: update only changed expectations,
  retain independent behavior assertions, and explain each changed image region.
- Keyboard: Super launch/search/Enter/Esc; nine workspaces; directional focus; tabs;
  close/minimize/zoom/restore; text editing and clipboard; no focus escape or lost input
  after service restart. Focus visible without pointer hover and without color alone.
- Hit geometry: drawn control bounds map to the same Shell action at scale 1 and scale 2;
  transparent corner clicks go to the correct underlying live owner; resize still works.
- HiDPI: run the scale/native-mode gate introduced by #77, including scale 2 at a larger
  framebuffer; test supported explicit overrides and small-screen clipping. Keep the two
  baseline resolutions at scale 1 for layout comparisons.
- Contrast: ordinary text >=4.5:1, large text and essential UI/focus boundaries >=3:1
  against the **composed pixel**, including active tabs/hover; use existing integer sRGB
  luminance helper. Disabled controls remain identifiable and visibly unavailable.
- Performance: same guest/core count, three identical drag/workspace/typing traces before
  and after; record median and p95 render/input-to-visible times, damage area and memory.
  Reject a >5% p95 regression until explained and fixed. Record this as a comparison gate,
  not a hardware latency claim. No added blur passes, glyph rasterization per frame,
  extra full-frame invalidation, unbounded shadow or idle animation timer.
- Keep immutable scene lease ownership, authenticated intake, bounded failure paths,
  private frame publication and service restart behavior. Preserve all unrelated pixels.

Broader accessibility remains an open capability: keyboard navigation and contrast are
necessary, but do not constitute screen-reader/accessibility-tree support. Avoid claiming
Apple/Quest-equivalent interaction or full accessibility from a cosmetic pass.
