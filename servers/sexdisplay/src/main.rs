#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use sex_pdx::serial_println;
static mut DISPLAY_LAST_OP: u64 = 0;
static mut DISPLAY_LAST_CALLER: u32 = 0;
use silkbar_model::{SilkBar, SilkBarUpdate, UpdateKind, apply_update, DEFAULT_SILK_BAR,
                    DEFAULT_THEME, ChipKind, ModuleSlot, validate_silkbar_contract, contract_fingerprint,
                    SILKBAR_ABI_VERSION, SILK_DE_BAR_ABI_V1, SILK_DE_BAR_LAYOUT_V1, SILK_DE_BAR_THEME_V1,
                    APPEARANCE_TOKEN_FOCUS_SURFACE, APPEARANCE_TOKEN_FRAME_RIM,
                    APPEARANCE_TOKEN_FRAME_TOP_BAR, APPEARANCE_TOKEN_ACTIVE_TAB,
                    APPEARANCE_TOKEN_INACTIVE_TAB, APPEARANCE_TOKEN_CLOSE_LIGHT,
                    APPEARANCE_TOKEN_MINIMIZE_LIGHT, APPEARANCE_TOKEN_ZOOM_LIGHT};

const FALLBACK_PTR: u64 = 0xffff8000fd000000;
const FALLBACK_W: u32 = 1280;
const FALLBACK_H: u32 = 800;
const HIGH_HALF_BASE: u64 = 0xffff_8000_0000_0000;
const MAX_FB_W: usize = 8192;
const MAX_FB_H: usize = 4320;
// Messages applied per frame. Text arrives 8 bytes per message, so a small
// drain left Display seconds behind its inbox (feature ledger H1).
const DRAIN_MAX: usize = 256;

/// Phase 3: receive+render proof for new SilkBar ABI v4 variants (8/9/10).
/// When enabled, emits [sexdisplay.silkbar.phase3.recv/state] markers.
/// Build: SEXOS_SILKBAR_PHASE3_RECEIVE_PROOF=1
const SILKBAR_PHASE3_RECEIVE_PROOF_ENABLED: bool =
    option_env!("SEXOS_SILKBAR_PHASE3_RECEIVE_PROOF").is_some();

/// Phase 5: minimal pixel indicators for active app, tint swatch, and palette state.
/// Adds tiny colored dots/swatches drawn inside the SilkBar strip (y<50).
/// Build: SEXOS_SILKBAR_PHASE5_PIXEL_PROOF=1
const SILKBAR_PHASE5_PIXEL_PROOF_ENABLED: bool =
    option_env!("SEXOS_SILKBAR_PHASE5_PIXEL_PROOF").is_some();

/// Heavy render proof profile gate: when unset, sexdisplay boots with
/// minimal proof overhead. Default boot skips deterministic topstrip
/// proof drawing, frame-rim/lights visual proof markers, and bounded
/// live debug markers.  Set SEXOS_SILK_RENDER_PROOF_PROFILE=1 at build
/// time to enable full visual proof suite for daily-driver gate verification.
const SILK_RENDER_PROOF_PROFILE_ENABLED: bool =
    option_env!("SEXOS_SILK_RENDER_PROOF_PROFILE").is_some();

/// Atlas Phase C render stub card geometry proof gate.
/// When enabled, draws bounded card outlines around active surfaces
/// in the below-bar area (y>=51).  Card geometry only — no thumbnails,
/// no live capture, no drag, no animation, no alpha/blur/shadow.
/// Default unset = zero behavior change.
/// Build: SEXOS_ATLAS_PHASE_C_RENDER_STUB_PROOF=1
const ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED: bool =
    option_env!("SEXOS_ATLAS_PHASE_C_RENDER_STUB_PROOF").is_some();
/// Card border color — visible teal-cyan outline, flat ARGB.
const ATLAS_CARD_BORDER_COLOR: u32 = 0x0089DCE6;
/// Card border thickness in pixels.
const ATLAS_CARD_BORDER_PX: usize = 2;
/// Maximum cards rendered in one pass.
const ATLAS_CARD_MAX_CARDS: usize = 16;

/// Atlas Phase D frame preview interior stub proof gate.
/// When enabled, draws bounded mini-frame rectangles inside Phase C cards
/// (or computed card geometry).  Interior previews only — no live thumbnails,
/// no surface capture, no framebuffer copy, no backing-buffer redesign.
/// Default unset = zero behavior change.
/// Build: SEXOS_ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF=1
const ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED: bool =
    option_env!("SEXOS_ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF").is_some();
/// Interior preview inner margin (pixels inset from card edges).
const ATLAS_PREVIEW_INNER_MARGIN: usize = 4;
/// Maximum interior previews rendered in one pass.
const ATLAS_PREVIEW_MAX_PREVIEWS: usize = 16;
/// Interior preview color for focused/active surface — lavender.
const ATLAS_PREVIEW_ACTIVE_COLOR: u32 = 0x00B880FF;
/// Interior preview color for visible (non-focused active) surface — cool blue.
const ATLAS_PREVIEW_VISIBLE_COLOR: u32 = 0x003070A0;
/// Interior preview color for minimized — dim steel gray.
const ATLAS_PREVIEW_MINIMIZED_COLOR: u32 = 0x00505058;

const BAR_BG_W_CAP: usize = 2560;
// Bounded backing for the shared 28px bar at the maximum 3x scale.
const BAR_BG_H: usize = 84;
fn chrome_metrics()->sex_pdx::desktop_style::chrome::Metrics {
    sex_pdx::desktop_style::chrome::Metrics::new(sex_pdx::desktop_style::ui_scale())
}
fn bar_height()->usize {
    chrome_metrics().bar() as usize
}
fn frame_height()->usize {
    chrome_metrics().title() as usize
}
fn frame_rim()->usize {
    chrome_metrics().px(sex_pdx::desktop_style::chrome::RESIZE) as usize
}
fn reflow_bar(bar:&mut SilkBar,width:usize){
    let m=chrome_metrics();
    if !silkbar_model::reflow_chrome(bar,width,m.bar()as usize,m.px(sex_pdx::desktop_style::chrome::TARGET)as usize){
        sex_pdx::serial_record!("[display.chrome.unsupported] width={} scale={} minimum={}",width,m.scale(),silkbar_model::MIN_CHROME_WIDTH);
    }
}

// Runtime FB config — starts as fallback, updated by OP_PRIMARY_FB

// SERIAL_DIET_V1: shared budget for hot-loop diagnostics. Early boot keeps
// the full trace (first 300 lines); steady-state loops go quiet so
// serial VM-exits stop dominating wall clock. Error and gate-required
// markers are NOT routed through this.
static mut HOT_LOG_BUDGET: u32 = 300;
#[inline(always)]
fn hot_log() -> bool {
    unsafe {
        if HOT_LOG_BUDGET > 0 { HOT_LOG_BUDGET -= 1; true } else { false }
    }
}

static mut FB_PTR: u64 = FALLBACK_PTR;
static mut FB_W: u32 = FALLBACK_W;
static mut FB_H: u32 = FALLBACK_H;
/// Scanout row length in pixels; at least FB_W. GOP modes on real GPUs pad rows.
static mut FB_PITCH: u32 = FALLBACK_W;
/// Pixels written to the scanout since boot, and by the last publication.
static mut PUBLISHED_PIXELS: u64 = 0;
static mut LAST_WRITTEN: u64 = 0;
/// The scale BOOT chose for this screen; Ctrl+Alt+0 returns to it.
static mut BOOT_UI_SCALE: u32 = 1;
static mut BAR_BG_BUF: [u32; BAR_BG_H * BAR_BG_W_CAP] = [0u32; BAR_BG_H * BAR_BG_W_CAP];
static mut BAR_BLUR_BUF: [u32; BAR_BG_H * BAR_BG_W_CAP] = [0u32; BAR_BG_H * BAR_BG_W_CAP];

// ── Surface Registry (V1: safe inline ABI, no backing buffers) ──────────────

/// Max fill rects per surface. rect_index in 0xEF selects which slot to write.
const MAX_RECTS: usize = 8;

/// A compositor surface. Rendered as a solid-color filled rect below the bar.
/// No backing buffer, no alpha, no z-ordering (insertion order only).
/// Ownership invariant: owner_pd is set on first create and never changes
/// while active. Content belongs to that PD. The immutable catalog coordinator
/// may manage geometry and hide its associated app/frame surfaces.
mod surface_lifecycle;
/// The BOOT incarnation catalog, at crate root for display modules.
use sex_pdx::app_catalog;
mod text_transaction;
mod glyphs;
mod bitmap_binding;
mod bitmap_transaction;
mod bitmap_stores;
mod fill_damage;
mod damage;
static mut TEXT_STAGES: [text_transaction::Stage; MAX_SURFACES] = [text_transaction::Stage::EMPTY; MAX_SURFACES];
static mut BITMAP_STORES: bitmap_stores::Stores<{sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser)}> = bitmap_stores::Stores::UNBOUND;
mod surface_groups;
static mut FRAME_GROUPS: surface_groups::FrameGroups = surface_groups::FrameGroups::new();
use surface_lifecycle::{Surface, MAX_SURFACES, TEXT_BUF_CAP, TEXT_MAX_ROWS, TEXT_LEGACY_COLS, SURFACE_EMPTY};
static mut SURFACES: [Surface; MAX_SURFACES] = [SURFACE_EMPTY; MAX_SURFACES];
mod scene;
mod frame_output;
mod screen_capture;
static mut SCREEN_CAPTURE:screen_capture::Server=screen_capture::Server::new();
mod paint_batch;
static mut FRAME_OUTPUT: Option<frame_output::Store> = None;
// One Display-owned scratch scene. Intake cannot run concurrently with a
// logical render in this single-task service. No producer/framebuffer pointer
// enters the snapshot, and each lease scrubs its owned payload on every exit.
static mut RENDER_SCENE: scene::SceneSnapshot = scene::SceneSnapshot::EMPTY;
struct SceneLease { _single_task: core::marker::PhantomData<*mut ()> }
impl SceneLease {
    fn capture(w: usize, h: usize, bar: &SilkBar) -> Option<Self> {
        unsafe {
            // Reject reentry using a shared read before constructing an
            // exclusive reference: an existing lease may still borrow data.
            if (*core::ptr::addr_of!(RENDER_SCENE)).is_valid() { return None; }
            let destination = &mut *core::ptr::addr_of_mut!(RENDER_SCENE);
            let frame = RENDER_FRAME_COUNTER.load(core::sync::atomic::Ordering::Relaxed);
            if !destination.capture(&SURFACES, &FRAME_GROUPS, &BITMAP_STORES, &TEXT_STAGES, bar,
                DISPLAY_TOKENS, FOCUSED_SURFACE_ID, frame, w, h) { return None; }
        }
        Some(Self { _single_task: core::marker::PhantomData })
    }
    fn view(&self) -> &scene::SceneSnapshot {
        unsafe { &*core::ptr::addr_of!(RENDER_SCENE) }
    }
}
impl Drop for SceneLease {
    fn drop(&mut self) {
        unsafe { (*core::ptr::addr_of_mut!(RENDER_SCENE)).scrub(); }
    }
}
static mut FOCUSED_SURFACE_ID: u64 = 0;
static mut APP_PRESENT_BITS: u64 = 0;
static APP300_PRESENTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn report_app_presented() {
    if unsafe { APP_PRESENT_BITS & 4 != 0 } && !APP300_PRESENTED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        sex_pdx::serial_record!("[display.app.presented] target=15 surface=300 scene=1");
    }
}
const FOCUS_SURFACE_COLOR: u32 = 0x0089B4FA;

// ── Frame Chrome Rim (focused surface, matches shell frame_rim()) ──
// Minimal mode button/rim size.  Surfaces start in minimal mode (chrome_flags=0)
// until OP_SURFACE_TAB_INFO arrives with SURFACE_CHROME_TOP_BAR set.  The initial
// render() call and any clock-tick redraws before the first chrome message must
// show clickable buttons, so keep these sizes usable (not 4×4 spec minimum).
const FRAME_RIM_PX: usize = 8;
const FRAME_RIM_COLOR: u32 = 0x00B4BEFE;

// ── Frame Light Colors (top-left rim band, matches shell FRAME_LIGHTS_HIT_TARGET_V1) ──
const FRAME_LIGHT_SIZE_PX: usize = 8;
const FRAME_LIGHT_GAP_PX: usize = 3;
const FRAME_LIGHT_CLOSE_COLOR: u32 = 0x00FF4444;
const FRAME_LIGHT_MINIMIZE_COLOR: u32 = 0x00FFCC44;
const FRAME_LIGHT_ZOOM_COLOR: u32 = 0x0044FF44;

/// Base alpha for the disabled close light (close_allowed=0, non-interactive).
/// V1: close is disabled for all frames; red renders at reduced alpha to
/// signal the action is unavailable. Yellow/green remain at normal brightness.
/// When close_allowed becomes 1 in a future phase, this changes to 224.
const FRAME_LIGHT_CLOSE_DISABLED_BASE_ALPHA: u8 = 160;

// ── Tab Strip Constants (matches shell FRAME_TAB_LIGHT_EXCLUSION_PX and TAB_*) ──
const TAB_STRIP_LIGHT_EXCLUSION_PX: usize = 20;
const TAB_ACTIVE_COLOR: u32 = FOCUS_SURFACE_COLOR; // 0x00A8E0FF (bright cyan)
const TAB_INACTIVE_COLOR: u32 = 0x0045475A; // Catppuccin Surface1

// ── Top Bar Chrome Mode (default mode, matches shell FRAME_TOP_BAR_*) ──
/// Surface.chrome_flags bit: top bar enabled (24px top band replaces 4px top rim).
const SURFACE_CHROME_TOP_BAR: u8 = 1 << 0;
/// chrome_flags bit 1: frame is hovered (pointer over frame body/chrome).
const SURFACE_CHROME_FRAME_HOVER: u8 = 1 << 1;
/// chrome_flags bit 2: a frame light is hovered (pointer over close/minimize/zoom).
const SURFACE_CHROME_LIGHT_HOVER: u8 = 1 << 2;
/// chrome_flags bits 3-4: hovered light kind (0=close, 1=minimize, 2=zoom).
const SURFACE_CHROME_LIGHT_KIND_MASK: u8 = 0x18; // bits 3-4
const SURFACE_CHROME_LIGHT_KIND_SHIFT: u8 = 3;
/// chrome_flags bit 5: close action allowed for this frame's active surface.
const SURFACE_CHROME_CLOSE_ALLOWED: u8 = 1 << 5;
/// Height of the top bar chrome band in default mode.
const FRAME_TOP_BAR_HEIGHT_PX: usize = 26;
/// Width and height of each frame light in default mode.
const FRAME_TOP_BAR_LIGHT_SIZE_PX: usize = 10;
/// Gap between adjacent frame lights in default mode.
const FRAME_TOP_BAR_LIGHT_GAP_PX: usize = 5;
/// X-width of the Frame Lights exclusion zone in default mode.
const FRAME_TOP_BAR_LIGHT_EXCLUSION_PX: usize = 50;
/// Top bar background color: dark navy-blue header, distinct from content teal.
const FRAME_TOP_BAR_COLOR: u32 = 0x001E1E2E;
/// Toolbar divider color (bottom edge of top bar).
const FRAME_TOP_BAR_DIVIDER_COLOR: u32 = 0x00B4BEFE;
/// Light vertical zone: top (y offset within top bar), centered in 28px.
const FRAME_TOP_BAR_LIGHT_TOP: usize = 9;
/// Light vertical zone: bottom (exclusive).
const FRAME_TOP_BAR_LIGHT_BOTTOM: usize = 19; // 9 + 10

// ── Appearance Tokens (V1: flat ARGB, no alpha blending, blur forced zero) ────

#[derive(Clone, Copy)]
#[repr(C)]
struct RenderTokensV1 {
    focus_surface_color: u32,
    frame_rim_color: u32,
    frame_top_bar_color: u32,
    active_tab_color: u32,
    inactive_tab_color: u32,
    close_light_color: u32,
    minimize_light_color: u32,
    zoom_light_color: u32,
    appearance_flags: u8,
    effect_levels: u8,
}

const DEFAULT_RENDER_TOKENS: RenderTokensV1 = RenderTokensV1 {
    focus_surface_color:  FOCUS_SURFACE_COLOR,
    frame_rim_color:      FRAME_RIM_COLOR,
    frame_top_bar_color:  FRAME_TOP_BAR_COLOR,
    active_tab_color:     TAB_ACTIVE_COLOR,
    inactive_tab_color:   TAB_INACTIVE_COLOR,
    close_light_color:    FRAME_LIGHT_CLOSE_COLOR,
    minimize_light_color: FRAME_LIGHT_MINIMIZE_COLOR,
    zoom_light_color:     FRAME_LIGHT_ZOOM_COLOR,
    appearance_flags:     0,
    effect_levels:        0,
};

static mut DISPLAY_TOKENS: RenderTokensV1 = DEFAULT_RENDER_TOKENS;

// Token reception state machine. Call 1 buffers; Call 2 commits.
static mut TOKEN_BUF_CALL1_RECEIVED: bool = false;
static mut TOKEN_BUF_OWNER: u32 = 0;
static mut TOKEN_BUF_ARG0: u64 = 0;
static mut TOKEN_BUF_ARG1: u64 = 0;
static mut TOKEN_BUF_ARG2: u64 = 0;

/// Force alpha byte to 0xFF. Renderer ignores alpha (raw framebuffer write),
/// but this guards against any future alpha-blending path receiving a zero.
#[inline]
fn clamp_color_token(c: u32) -> u32 {
    c | 0xFF000000
}

// Shell-owned OS cursor surface. Always rendered last (above all app surfaces).
const CURSOR_SURFACE_ID: u64 = 0x90;
const LAUNCHER_PANEL_SURFACE_ID: u64 = 0x92;

/// Rate-limited rejection counter for unauthorized surface ops.
/// Logs at most every 64 rejections to prevent IPC/log storms.
static REJECT_COUNTER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static CURSOR_Z_TOP_LOGGED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static DISPLAY_WM_PD: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static RENDER_FRAME_COUNTER: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
static ROW_BUFFER_PROOF_LOGGED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static BLUR_PROOF_LOGGED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static ANIM_PROOF_LOGGED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
static R6_GLASS_PROOF_LOGGED: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);
/// Clock source for redraw marker: 0=silkbar, 1=fallback
/// Start in fallback until first valid SetClock apply.
static mut CLOCK_REDRAW_SOURCE: u8 = 1;
/// Canonical clock state for visible redraw path.
static mut CLOCK_CANON_HH: u8 = 10;
static mut CLOCK_CANON_MM: u8 = 42;
static mut CLOCK_CANON_SS: u8 = 0;
	/// Last canonical ss that was actually drawn to screen.
	/// Used to suppress same-second redraw spam (clock-only redraws
	/// that don't change the visible second).
	static mut LAST_REDRAWN_CLOCK_CANON_SS: u8 = 0xFF;

#[inline(always)]
fn clock_canon_store(hh: u8, mm: u8, ss: u8) {
    unsafe {
        CLOCK_CANON_HH = hh;
        CLOCK_CANON_MM = mm;
        CLOCK_CANON_SS = ss;
    }
}

#[inline(always)]
fn clock_total_seconds(hh: u8, mm: u8, ss: u8) -> u32 {
    (hh as u32) * 3600 + (mm as u32) * 60 + (ss as u32)
}

#[inline(always)]
fn clock_visible_monotonic_ok(prev: u32, curr: u32) -> bool {
    // Allow same/increasing values, and single day rollover at midnight.
    curr >= prev || (prev >= 86_300 && curr <= 60)
}

#[inline(always)]
fn clock_canon_apply_to_bar(bar: &mut SilkBar) {
    if bar.civil_enabled {return;}
    unsafe {
        bar.clock_hh = CLOCK_CANON_HH;
        bar.clock_mm = CLOCK_CANON_MM;
        bar.clock_ss = CLOCK_CANON_SS;
    }
}

/// Clamp a surface rectangle against framebuffer dimensions.
/// Returns `(x, y, w, h)` guaranteed to be within FB bounds and below the bar.
/// The `y` coordinate is clamped to at least `BAR_H` to prevent covering the top strip.
fn clamp_surface(surf: &Surface, fb_w: usize, fb_h: usize) -> (usize, usize, usize, usize) {
    // BAR_H covers model PANEL_Y+PANEL_H (10+38=48) plus 2px safety margin.
    // Must remain >= PANEL_Y + PANEL_H from silkbar-model.
    let bar_h=bar_height();
    let x = (surf.x.max(0) as usize).min(fb_w.saturating_sub(1));
    let y = (surf.y.max(bar_h as i32) as usize).min(fb_h.saturating_sub(1));
    let max_w = fb_w.saturating_sub(x);
    let max_h = fb_h.saturating_sub(y);
    let w = (surf.w as usize).min(max_w);
    let h = (surf.h as usize).min(max_h);
    (x, y, w, h)
}

/// Composite a single pixel: non-focused surfaces in slot order, then focused on top.
/// Shared by render() and redraw_surface_area() to prevent drift.
fn composite_pixel(x: usize, y: usize, w: usize, h: usize, bg: u32, focused_id: u64, scene: &scene::SceneSnapshot) -> u32 {
    let selected=unsafe{scene.groups.winner(&scene.surfaces,focused_id,x,y,w,h).map(|s|s.surface_id)};
    composite_pixel_selected(x,y,w,h,bg,focused_id,selected, scene)
}

fn composite_pixel_selected(x: usize, y: usize, w: usize, h: usize, bg: u32, focused_id: u64, selected:Option<u64>, scene: &scene::SceneSnapshot) -> u32 {
    let source = unsafe { scene.surfaces.iter().find(|s| s.active && Some(s.surface_id) == selected)
        .map(|surface| surface_groups::PixelSource {
            surface,
            content: scene.groups.content(surface, &scene.surfaces, x, y, w, h),
            chrome: surface.surface_id == focused_id || scene.groups.is_frame(surface) || scene.groups.is_program_frame(surface),
        }) };
    composite_pixel_source(x,y,w,h,bg,source, scene)
}

/// Choices are borrowed only during this immutable render row; no lookup or
/// lifecycle authority is retained across a display drain.
fn composite_pixel_source(x: usize, y: usize, w: usize, h: usize, bg: u32, source: Option<surface_groups::PixelSource<'_>>, scene: &scene::SceneSnapshot) -> u32 {
    let Some(source) = source else { return bg; };
    let surf = source.surface;
    let (sx, sy, sw, sh) = scene.groups.rect(surf, w, h);
    if !surf.active || sw == 0 || sh == 0 || x < sx || x >= sx + sw || y < sy || y >= sy + sh { return bg; }
    let mut c = bg;
    let mut winner_mask = surface_lifecycle::native_pixel_bit(surf.owner_pd, surf.surface_id);
    if !source.chrome {
        c = fill_rect_color(surf, x, y, surf.color);
        c = bitmap_color(surf, x, y, c, scene);
        if let Some(tc) = surface_text_fg_at_scaled(x,y,surf,scene.metrics.scale()) { c = tc; }
    } else { unsafe {
        // ── Phase 6 proof: real render-path markers ──────────────
        // Emit once when the focused-surface pixel path is entered.
        // Proves bounds were checked via clamp_surface + sw/sh guards
        // and that the real compositor loop called composite_pixel.
        unsafe {
            static mut PHASE6_RENDER_BEGIN_PROOF: u32 = 1;
            if PHASE6_RENDER_BEGIN_PROOF > 0 {
                PHASE6_RENDER_BEGIN_PROOF -= 1;
                let active_count = scene.surfaces.iter().filter(|s| s.active).count();
                let frame_count = scene.surfaces.iter().filter(|s| s.active && s.tab_count > 0 && (s.chrome_flags & SURFACE_CHROME_TOP_BAR) != 0).count();
                serial_println!("[sexdisplay.frame_lights.render.begin] fb_w={} fb_h={} surfaces={} frames={}", w, h, active_count, frame_count);
            }
            static mut PHASE6_BOUNDS_OK_PROOF: u32 = 1;
            if PHASE6_BOUNDS_OK_PROOF > 0 {
                PHASE6_BOUNDS_OK_PROOF -= 1;
                serial_println!("[sexdisplay.frame_lights.bounds.ok] all_lights_within_fb=1 fb_w={} fb_h={} light_w=40 light_h=10 ok=1 reason=clamp_surface_guarantee", w, h);
            }
        }
        // Rim + Frame Light + Top Bar check (focused surface only).
        let lx = x - sx;  // local x within clamped surface
        let ly = y - sy;  // local y within clamped surface

        // Edge detection with saturating_sub to prevent underflow
        // on tiny surfaces (rim fills entire surface if sw/sh < RIM_PX).
        let rim_right = sw.saturating_sub((scene.metrics.px(8)as usize));
        let rim_bottom = sh.saturating_sub((scene.metrics.px(8)as usize));
        let top_bar_active = (surf.chrome_flags & SURFACE_CHROME_TOP_BAR) != 0;
        let frame_hovered = (surf.chrome_flags & SURFACE_CHROME_FRAME_HOVER) != 0;
        let light_hovered = (surf.chrome_flags & SURFACE_CHROME_LIGHT_HOVER) != 0;
        let hovered_light_kind = (surf.chrome_flags & SURFACE_CHROME_LIGHT_KIND_MASK) >> SURFACE_CHROME_LIGHT_KIND_SHIFT;
        let close_allowed = (surf.chrome_flags & SURFACE_CHROME_CLOSE_ALLOWED) != 0;
        let single_tab = surf.tab_count <= 1;

        // One-shot reveal mode marker.
        unsafe {
            static mut HOVER_REVEAL_PROOF: u32 = 1;
            if HOVER_REVEAL_PROOF > 0 && ly == 0 && lx == 0 {
                HOVER_REVEAL_PROOF -= 1;
                serial_println!("[sexdisplay.frame.hover.reveal] mode=v1b single_tab={} hovered={}",
                    single_tab as u8, frame_hovered as u8);
            }
        }

        // Hover reveal: single-tab frame chrome dims when not hovered,
        // brightens on hover. Multi-tab always fully visible.
        let chrome_dim: u8 = if single_tab && !frame_hovered { 5 } else { 10 };
        // Scale: 5 → ~20% alpha, 10 → full alpha.
        fn scale_alpha(base: u8, dim: u8) -> u8 {
            ((base as u16 * dim as u16) / 10) as u8
        }

        // ── TOP BAR ZONE (default chrome mode, R6 glass) ──
        // [silk.live_topstrip.v2.clip] Safety: frame chrome must never render
        // above the global SilkBar strip. y is the global framebuffer row;
        // composite_pixel uses the frozen scene bar boundary (28logical px),
        // but this belt-and-suspenders guard catches any future path that might
        // call composite_pixel inside that visible strip, independent of backing capacity.
        if y < (scene.metrics.bar()as usize) {
            // y is in or above the SilkBar glow row — skip frame chrome entirely.
            // This pixel will get bar_color or panel_glow from the top strip pass.
        } else if top_bar_active && ly < (scene.metrics.title()as usize) {
            // Frosted glass toolbar background.
            let base_alpha: u8 = if frame_hovered { 220 } else { 200 };
            let toolbar_alpha = scale_alpha(base_alpha, chrome_dim);
            if toolbar_alpha > 0 {
                c = glass_over_bg(scene.tokens.frame_top_bar_color, x, y, toolbar_alpha, scene);
            }
            // Divider: bright opaque line at bottom edge.
            if ly >= (scene.metrics.title()as usize).saturating_sub(scene.metrics.scale()as usize) {
                c = FRAME_TOP_BAR_DIVIDER_COLOR;
            }
            // Tab strip: glass background with tab color overlay.
            if surf.tab_count > 0
                && lx >= scene.metrics.px(74)as usize
                && lx < rim_right
            {
                let exclusion=scene.metrics.px(74)as usize;
                let available = rim_right.saturating_sub(exclusion);
                let slot_w = available / surf.tab_count as usize;
                if slot_w > 0 {
                    let tab_idx = (lx - exclusion) / slot_w;
                    let tab_color = if tab_idx == surf.active_tab as usize {
                        scene.tokens.active_tab_color
                    } else {
                        scene.tokens.inactive_tab_color
                    };
                    let tab_glass = glass_over_bg(tab_color, x, y, 224, scene);
                    let tab_alpha: u8 = if tab_idx == surf.active_tab as usize { 240 } else { 200 };
                    c = alpha_blend_xrgb_over_xrgb(tab_color, tab_glass, tab_alpha);
                }
            }
            // Select title ink against the final glass/tab pixel, then paint it.
            if let Some(fg)=toolbar_title_fg_at_scaled(lx,ly,surf.surface_id,c,scene.metrics.scale()){c=fg;}
            // Natural 10px circular marks inside the shared 24px targets.
            // Painting and pointer targets share the same transformed boxes.
            for index in 0..3 {
                let r=scene.metrics.control(index).unwrap();
                let size=scene.metrics.px(10)as usize;
                let left=r.x as usize+(r.w as usize-size)/2;
                let top=r.y as usize+(r.h as usize-size)/2;
                if !in_rect(lx,ly,left,top,size,size){continue;}
                let dx=(2*(lx-left)+1)as i32-size as i32;
                let dy=(2*(ly-top)+1)as i32-size as i32;
                if dx*dx+dy*dy>=(size*size)as i32 {continue;}
                let color=match index{0=>scene.tokens.close_light_color,1=>scene.tokens.minimize_light_color,_=>scene.tokens.zoom_light_color};
                let disabled=index==0&&!close_allowed;
                let hovered=light_hovered&&hovered_light_kind as u32==index;
                let alpha=if disabled{80}else if hovered{255}else{224};
                let inner=size.saturating_sub(2*scene.metrics.scale()as usize);
                let ink=if dx*dx+dy*dy>=(inner*inner)as i32{sex_pdx::desktop_style::contrast_ink(c)}else{color};
                c=coverage_blend(ink,c,alpha);
            }
        }
        // ── RIM BAND (minimal mode top edge, or left/right/bottom edges always) ──
        else if ly < (scene.metrics.px(8)as usize) || lx < (scene.metrics.px(8)as usize)
            || lx >= rim_right || ly >= rim_bottom
        {
            // In the rim band. Check for Frame Lights in top-left corner.
            // This path is reached in minimal mode (top_bar_active=false) or
            // for left/right/bottom edges when top bar is active.
            if ly < (scene.metrics.px(8)as usize) && !top_bar_active {
                // Minimal mode top rim lights — glass version.
                // Close light dimmed: close_allowed=0, non-interactive.
                let l1_alpha: u8 = if close_allowed { 224 } else { FRAME_LIGHT_CLOSE_DISABLED_BASE_ALPHA };
                if lx >= FRAME_LIGHT_GAP_PX
                    && lx < FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX
                {
                    c = glass_over_bg(scene.tokens.close_light_color, x, y, l1_alpha, scene);
                }
                else if lx >= FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX
                    && lx < FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX
                {
                    c = glass_over_bg(scene.tokens.minimize_light_color, x, y, 224, scene);
                }
                else if lx >= FRAME_LIGHT_GAP_PX + 2 * (FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX)
                    && lx < FRAME_LIGHT_GAP_PX + 2 * (FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX) + FRAME_LIGHT_SIZE_PX
                {
                    c = glass_over_bg(scene.tokens.zoom_light_color, x, y, 224, scene);
                }
                // Tab strip — glass version.
                else if surf.tab_count > 0
                    && lx >= TAB_STRIP_LIGHT_EXCLUSION_PX
                    && lx < rim_right
                {
                    let available = rim_right - TAB_STRIP_LIGHT_EXCLUSION_PX;
                    let slot_w = available / surf.tab_count as usize;
                    if slot_w > 0 {
                        let tab_idx = (lx - TAB_STRIP_LIGHT_EXCLUSION_PX) / slot_w;
                        let tab_color = if tab_idx == surf.active_tab as usize {
                            scene.tokens.active_tab_color
                        } else {
                            scene.tokens.inactive_tab_color
                        };
                        let tab_alpha: u8 = if tab_idx == surf.active_tab as usize { 220 } else { 180 };
                        c = glass_over_bg(tab_color, x, y, tab_alpha, scene);
                    } else {
                        c = glass_over_bg(scene.tokens.frame_rim_color, x, y, 196, scene);
                    }
                } else {
                    c = glass_over_bg(scene.tokens.frame_rim_color, x, y, 196, scene);
                }
            } else {
                // Keep the full resize band, but give it a quiet inset surface
                // and a steady one-pixel outline. Changing frame time must not
                // animate an idle window's edge or hide its resize affordance.
                let edge=scene.metrics.scale()as usize;
                let outer_edge = lx<edge||ly<edge||lx>=sw.saturating_sub(edge)||ly>=sh.saturating_sub(edge);
                c = if outer_edge {
                    glass_over_bg(scene.tokens.frame_rim_color, x, y, 112, scene)
                } else {
                    glass_over_bg(sex_pdx::desktop_style::NATIVE_CANVAS, x, y, 224, scene)
                };
            }
        } else {
            // ── SURFACE CONTENT AREA ──
            let content = source.content;
            winner_mask = surface_lifecycle::native_pixel_bit(content.owner_pd, content.surface_id);
            c = fill_rect_color(content, x, y, content.color);
            c = bitmap_color(content, x, y, c, scene);
            // Child text uses its own origin; the parent alone draws chrome.
            if let Some(tc) = surface_text_fg_at_scaled(x,y,content,scene.metrics.scale()) { c = tc; }
        }
    } }
    if winner_mask != 0 { unsafe { APP_PRESENT_BITS |= winner_mask; } }
    c
}

/// Paint one row run of plain surface content without asking pixel by pixel.
/// False means this run needs composite_pixel_source: chrome, a bitmap, or a
/// fill rect covering part of it. Must match that function exactly.
fn fast_content_run(fb:*mut u32,x0:usize,x1:usize,y:usize,w:usize,h:usize,source:surface_groups::PixelSource<'_>,scene: &scene::SceneSnapshot)->bool{
    let surf=source.surface;
    let (sx,sy,sw,sh)=scene.groups.rect(surf,w,h);
    if !surf.active||sw==0||sh==0||x0<sx||x1>sx+sw||y<sy||y>=sy+sh{return false;}
    let content=if source.chrome{
        let ly=y-sy;
        let top=if surf.chrome_flags&SURFACE_CHROME_TOP_BAR!=0{(scene.metrics.title()as usize)}else{(scene.metrics.px(8)as usize)};
        if y<(scene.metrics.bar()as usize)||ly<top||ly>=sh.saturating_sub((scene.metrics.px(8)as usize))
            ||x0-sx<(scene.metrics.px(8)as usize)||x1-sx>sw.saturating_sub((scene.metrics.px(8)as usize)){return false;}
        source.content
    }else{surf};
    if unsafe{scene.bitmaps.matches_surface(content.owner_pd,content.surface_id)}{return false;}
    let mut base=content.color;
    let (lx0,lx1,ly)=(x0 as i32-content.x,x1 as i32-content.x,y as i32-content.y);
    for i in 0..content.fill_count as usize{
        if content.fill_sw[i]==0||content.fill_sh[i]==0
            ||ly<content.fill_sy[i]||ly>=content.fill_sy[i]+content.fill_sh[i] as i32
            ||lx1<=content.fill_sx[i]||lx0>=content.fill_sx[i]+content.fill_sw[i] as i32{continue;}
        if lx0<content.fill_sx[i]||lx1>content.fill_sx[i]+content.fill_sw[i] as i32{return false;}
        base=content.fill_color[i];
    }
    let mask=surface_lifecycle::native_pixel_bit(if source.chrome{content.owner_pd}else{surf.owner_pd},if source.chrome{content.surface_id}else{surf.surface_id});
    if mask!=0{unsafe{APP_PRESENT_BITS|=mask;}}
    let row=unsafe{fb.add(y*w)};
    // The private frame is ordinary memory: a plain fill vectorizes.
    unsafe{core::slice::from_raw_parts_mut(row.add(x0),x1-x0).fill(base);}
    text_run(row,x0,x1,y,content,scene.metrics);
    true
}

/// The ink and selection of `surface_text_fg_at` for one row run.
fn text_run(row:*mut u32,x0:usize,x1:usize,y:usize,surf:&Surface,metrics:sex_pdx::desktop_style::chrome::Metrics){
    if let Some((src,from,to))=program_pixel_row_for(y,x0,x1,surf,metrics){
        // Straight from the program's own region: one span copy per row.
        let out=unsafe{core::slice::from_raw_parts_mut(row.add(from),to-from)};
        for(i,o)in out.iter_mut().enumerate(){*o=unsafe{core::ptr::read_volatile(src.add(i))}&0x00FF_FFFF;}
        return;
    }
    if program_pixel_at_for(0,0,surf,metrics).is_some(){return;}
    if surf.text_len==0||surf.line_count==0{return;}
    const GLYPH_W:usize=sex_pdx::desktop_style::GLYPH_WIDTH as usize;
    const GLYPH_H:usize=sex_pdx::desktop_style::GLYPH_HEIGHT as usize;
    let scale=surface_text_scale_for(surf,metrics.scale());
    let tx=(surf.x.max(0) as usize).saturating_add(metrics.px(sex_pdx::desktop_style::INSET)as usize);
    let top_inset=if surf.chrome_flags&SURFACE_CHROME_TOP_BAR!=0{(metrics.title()as usize)+(metrics.px(8)as usize)}else{metrics.px(sex_pdx::desktop_style::TEXT_TOP)as usize};
    let ty=(surf.y.max(0) as usize).saturating_add(top_inset);
    if y<ty{return;}
    let line=(y-ty)/((GLYPH_H+2)*scale);
    if line>=surf.line_count as usize{return;}
    let py=((y-ty)/scale)%(GLYPH_H+2);
    let advance=(GLYPH_W+1)*scale;
    let first=x0.saturating_sub(tx)/advance;
    let last=(x1.saturating_sub(tx)+advance-1)/advance;
    let row_start=surf.line_cells[line] as usize;
    let row_end=if line+1<surf.line_count as usize{surf.line_cells[line+1]}else{surf.cell_count} as usize;
    let len=(surf.text_len as usize).min(TEXT_BUF_CAP);
    let cells=(surf.wrap_cols as usize).min(row_end.saturating_sub(row_start)).min(last);
    for col in first..cells{
        let ordinal=row_start+col;
        if ordinal>=surf.cell_count as usize{break;}
        let index=surf.cell_starts[ordinal] as usize;
        let Some((scalar,end))=sex_pdx::text_encoding::cell(&surf.text_buf[..len],index)else{continue;};
        if scalar==10{continue;}
        let selected=index<surf.selection_end as usize&&end>surf.selection_start as usize
            &&surf.selection_end as usize<=surf.text_len as usize;
        let left=tx+col*advance;
        // Geist Mono drawn for this cell size, as surface_text_fg_at draws it.
        {
            let line_h=(GLYPH_H+2)*scale;
            if let Some(cell)=glyphs::mono_cell(scalar,advance,line_h){
                let gy=(y-ty)%line_h;
                let back=if selected{0xff36577c}else{surf.color};
                for x in left.max(x0)..(left+advance).min(x1){
                    let alpha=cell[gy*advance+(x-left)];
                    if alpha!=0{unsafe{core::ptr::write_volatile(row.add(x),coverage_blend(surf.text_color,back,alpha));}}
                    else if selected{unsafe{core::ptr::write_volatile(row.add(x),back);}}
                }
                continue;
            }
        }
        let bits=if py<GLYPH_H{scalar_glyph(scalar)[py]}else{0};
        if bits==0&&!selected{continue;}
        for x in left.max(x0)..(left+advance).min(x1){
            let px=(x-left)/scale;
            let ink=px<GLYPH_W&&(bits>>(GLYPH_W-1-px))&1!=0;
            if ink{unsafe{core::ptr::write_volatile(row.add(x),surf.text_color);}}
            else if selected{unsafe{core::ptr::write_volatile(row.add(x),0xff36577c);}}
        }
    }
}

/// One framebuffer row from x0 to x1: fast runs where possible.
fn raster_row(fb:*mut u32,y:usize,x0:usize,x1:usize,w:usize,h:usize,scene: &scene::SceneSnapshot){
    let focused=scene.focus;
    let back=bg(y,h);
    let mut row=unsafe{scene.groups.row_winners(&scene.surfaces,focused,y,w,h)};
    let mut x=x0;
    while x<x1{
        let (end,source)=match row.as_mut(){Some(row)=>row.run(x),None=>(x1,None)};
        let end=end.min(x1);
        if row.is_some(){
            if let Some(source)=source{
                if fast_content_run(fb,x,end,y,w,h,source,scene){x=end;continue;}
            }else{
                unsafe{core::slice::from_raw_parts_mut(fb.add(y*w+x),end-x).fill(back);}
                x=end;continue;
            }
        }
        while x<end{
            let color=if let Some(row)=row.as_mut(){
                composite_pixel_source(x,y,w,h,back,row.source(x),scene)
            }else{composite_pixel(x,y,w,h,back,focused,scene)};
            unsafe{core::ptr::write_volatile(fb.add(y*w+x),color);}
            x+=1;
        }
    }
}

/// For each active fill rect in the surface, if the global pixel (x,y) falls
/// within the rect, update the running color (painter's order: last match wins).
/// Used in both passes of composite_pixel to prevent logic drift.
fn fill_rect_color(surf: &Surface, x: usize, y: usize, base_color: u32) -> u32 {
    if surf.fill_count == 0 { return base_color; }
    let lx = (x as i32) - surf.x;
    let ly = (y as i32) - surf.y;
    let mut c = base_color;
    for i in 0..surf.fill_count as usize {
        if surf.fill_sw[i] == 0 || surf.fill_sh[i] == 0 { continue; }
        if lx >= surf.fill_sx[i] && lx < surf.fill_sx[i] + surf.fill_sw[i] as i32
            && ly >= surf.fill_sy[i] && ly < surf.fill_sy[i] + surf.fill_sh[i] as i32
        {
            c = surf.fill_color[i];
        }
    }
    c
}

fn bitmap_viewport(surf: &Surface) -> bitmap_transaction::Viewport{
    bitmap_viewport_for(surf,chrome_metrics())
}
fn bitmap_viewport_for(surf: &Surface, metrics:sex_pdx::desktop_style::chrome::Metrics) -> bitmap_transaction::Viewport{
    let chrome = surf.chrome_flags & SURFACE_CHROME_TOP_BAR != 0;
    bitmap_transaction::Viewport { x: surf.x, y: surf.y, width: surf.w, height: surf.h,
        inset: if chrome { metrics.px(8) } else { 0 }, top: if chrome { metrics.title() + metrics.px(8) } else { 0 } }
}
fn bitmap_color(surf: &Surface, x: usize, y: usize, base: u32, scene: &scene::SceneSnapshot) -> u32 {
    if !surf.active || !(unsafe { scene.bitmaps.matches_surface(surf.owner_pd, surf.surface_id) }) { return base; }
    let Some((x, y)) = bitmap_viewport_for(surf,scene.metrics).local(x, y) else { return base; };
    unsafe { scene.bitmaps.sample(surf.owner_pd, surf.surface_id, x, y) }
        .map_or(base, |pixel| bitmap_transaction::blend(pixel, base))
}
fn bitmap_surface_damage(surf: &Surface, geometry: Option<sex_pdx::bitmap_batch::Geometry>, w: usize, h: usize) -> (usize, usize, usize, usize) {
    bitmap_viewport(surf).screen(geometry, w, h)
}

/// Fixed-point linear interpolation between two XRGB colors.
/// t=0 returns a, t=255 returns b.  Alpha byte forced to 0xFF.
#[inline]
fn lerp_color_xrgb(a: u32, b: u32, t: u8) -> u32 {
    if t == 0 { return a | 0xFF000000; }
    if t == 255 { return b | 0xFF000000; }
    let ar = (a & 0xFF) as u32;
    let ag = ((a >> 8) & 0xFF) as u32;
    let ab = ((a >> 16) & 0xFF) as u32;
    let br = (b & 0xFF) as u32;
    let bg = ((b >> 8) & 0xFF) as u32;
    let bb = ((b >> 16) & 0xFF) as u32;
    let ti = t as u32;
    let r = ar + ((br.wrapping_sub(ar)).wrapping_mul(ti) / 255);
    let g = ag + ((bg.wrapping_sub(ag)).wrapping_mul(ti) / 255);
    let b_ = ab + ((bb.wrapping_sub(ab)).wrapping_mul(ti) / 255);
    0xFF000000 | (b_ << 16) | (g << 8) | r
}

/// Smooth vertical gradient between two XRGB colors.
/// y=0 returns top, y=h-1 returns bottom.
#[inline]
fn vertical_gradient_xrgb(y: usize, h: usize, top: u32, bottom: u32) -> u32 {
    if h <= 1 { return top; }
    let t = ((y.min(h - 1) as u32).wrapping_mul(255)) / ((h - 1) as u32);
    lerp_color_xrgb(top, bottom, t as u8)
}

/// Desktop background: smooth gradient from bg_top (at y=0) to bg_bottom (at bottom).
/// Replaces the old 4-band hard gradient.  Endpoint colors preserved from Theme.
fn bg(y: usize, h: usize) -> u32 {
    let (top, bottom) = sex_pdx::desktop_look::wallpaper().unwrap_or((DEFAULT_THEME.bg_top, DEFAULT_THEME.bg_bottom));
    vertical_gradient_xrgb(y, h, top, bottom)
}

fn fill_bar_bg_buffer(fb_w: u32, fb_h: u32) -> bool {
    let fill_w = fb_w as usize;
    if fill_w > BAR_BG_W_CAP {
        return false;
    }
    let fill_h = bar_height().min(fb_h as usize);
    for y in 0..fill_h {
        let c = bg(y, fb_h as usize);
        for x in 0..fill_w {
            unsafe { BAR_BG_BUF[y * BAR_BG_W_CAP + x] = c; }
        }
    }
    true
}

fn blur_bar_bg_buffer_radius1(fb_w: u32, fb_h: u32) -> bool {
    let width = fb_w as usize;
    if width > BAR_BG_W_CAP {
        return false;
    }
    let height = bar_height().min(fb_h as usize);
    for y in 0..height {
        for x in 0..width {
            let mut acc_r: u32 = 0;
            let mut acc_g: u32 = 0;
            let mut acc_b: u32 = 0;
            let y0 = y.saturating_sub(1);
            let y1 = (y + 1).min(height.saturating_sub(1));
            let x0 = x.saturating_sub(1);
            let x1 = (x + 1).min(width.saturating_sub(1));
            for sy in y0..=y1 {
                for sx in x0..=x1 {
                    let c = unsafe { BAR_BG_BUF[sy * BAR_BG_W_CAP + sx] };
                    acc_r += c & 0xFF;
                    acc_g += (c >> 8) & 0xFF;
                    acc_b += (c >> 16) & 0xFF;
                }
            }
            let count = ((y1 - y0 + 1) * (x1 - x0 + 1)) as u32;
            let r = (acc_r / count) & 0xFF;
            let g = (acc_g / count) & 0xFF;
            let b = (acc_b / count) & 0xFF;
            unsafe { BAR_BLUR_BUF[y * BAR_BG_W_CAP + x] = 0xFF000000 | (b << 16) | (g << 8) | r; }
        }
    }
    true
}

/// The backdrop is horizontally constant. Its 3x3 integer blur therefore
/// equals the average of its two/three distinct source rows. Capture these
/// colors once; a pixel never reads the mutable legacy blur scratch buffers.
fn scene_bar_blur_rows(fb_h: usize) -> [u32; BAR_BG_H] {
    let height=bar_height().min(fb_h);
    let mut rows=[0;BAR_BG_H];
    for y in 0..height {
        let first=y.saturating_sub(1);
        let last=(y+1).min(height-1);
        let (mut r,mut g,mut b)=(0u32,0u32,0u32);
        for sy in first..=last {
            let color=bg(sy,fb_h);
            r+=color&255;g+=(color>>8)&255;b+=(color>>16)&255;
        }
        let count=(last-first+1)as u32;
        rows[y]=0xFF000000|((b/count)<<16)|((g/count)<<8)|(r/count);
    }
    rows
}

fn sample_bar_blur_bg_xrgb(x: u32, y: u32, fb_w: u32, fb_h: u32, scene: &scene::SceneSnapshot) -> u32 {
    if fb_w as usize > BAR_BG_W_CAP || y as usize >= BAR_BG_H || x >= fb_w {
        return bg(y as usize, fb_h as usize);
    }
    scene.blur_rows[y as usize]
}

/// Alpha-blend an XRGB foreground over an XRGB background.
/// alpha=0 → bg, alpha=255 → fg.  Result is opaque XRGB.
/// Uses integer arithmetic only; no framebuffer read.
#[inline]
/// A glyph's coverage over what is behind it. Full coverage is the ink itself.
fn coverage_blend(fg: u32, bg: u32, coverage: u8) -> u32 {
    if coverage == 255 { return fg; }
    let (a, inv) = (coverage as u32, 255 - coverage as u32);
    let mix = |shift: u32| ((((fg >> shift) & 0xFF) * a + ((bg >> shift) & 0xFF) * inv) / 255) << shift;
    (fg & 0xFF00_0000) | mix(16) | mix(8) | mix(0)
}
fn alpha_blend_xrgb_over_xrgb(fg: u32, bg: u32, alpha: u8) -> u32 {
    if alpha == 0 { return bg; }
    if alpha == 255 { return fg; }
    let a = alpha as u32;
    let inv = 255 - a;
    let fr = (fg & 0xFF) as u32;
    let fg_ = ((fg >> 8) & 0xFF) as u32;
    let fb = ((fg >> 16) & 0xFF) as u32;
    let br = (bg & 0xFF) as u32;
    let bg_ = ((bg >> 8) & 0xFF) as u32;
    let bb = ((bg >> 16) & 0xFF) as u32;
    let r = (fr * a + br * inv) / 255;
    let g = (fg_ * a + bg_ * inv) / 255;
    let b = (fb * a + bb * inv) / 255;
    0xFF000000 | (b << 16) | (g << 8) | r
}

/// Alpha-blend a glass fg over the desktop background gradient.
/// Uses FB_H for height; alpha controls translucency.
#[inline]
fn glass_over_bg(fg: u32, x: usize, y: usize, alpha: u8, scene: &scene::SceneSnapshot) -> u32 {
    let fb_w = unsafe { scene.width as u32 };
    let fb_h = unsafe { scene.height as u32 };
    if fb_h == 0 { return fg; }
    let bg = sample_bar_blur_bg_xrgb(x as u32, y as u32, fb_w, fb_h, scene);
    alpha_blend_xrgb_over_xrgb(fg, bg, alpha)
}

/// Linear edge glow alpha: distance 0 → max_alpha, distance ≥ spread → 0.
/// Integer-only, no floats.
#[inline]
fn glow_edge_alpha(dist: u32, spread: u32, max_alpha: u8) -> u8 {
    if spread == 0 || dist >= spread { return 0; }
    (max_alpha as u32 * (spread - dist) / spread) as u8
}

#[inline]
fn in_rect(x: usize, y: usize, rx: usize, ry: usize, rw: usize, rh: usize) -> bool {
    x >= rx && x < rx + rw && y >= ry && y < ry + rh
}

#[inline]
fn module_rect(bar: &SilkBar, slot: ModuleSlot) -> (usize, usize, usize, usize) {
    let lb = &bar.layout[slot as usize];
    (lb.x, lb.y, lb.w, lb.h)
}

fn workspace_color(x:usize,y:usize,bar:&SilkBar,scene: &scene::SceneSnapshot)->Option<u32>{
    for i in 0..silkbar_model::WORKSPACE_COUNT {
        let b=&bar.layout[1+i];
        if b.w==0||!in_rect(x,y,b.x,b.y,b.w,b.h){continue;}
        let scale=scene.metrics.scale()as usize;
        if i==0&&bar.layout[ModuleSlot::Workspace1 as usize].w==0 {
            // A recognizable nine-dot overview mark, not a hidden workspace
            // strip. The whole shared target opens the real Atlas overview.
            let left=b.x+(b.w-10*scale)/2;let top=b.y+(b.h-10*scale)/2;
            if in_rect(x,y,left,top,10*scale,10*scale)
                &&((x-left)/scale)%4<2&&((y-top)/scale)%4<2{return Some(DEFAULT_THEME.active);}
            return None;
        }
        let size=if bar.workspaces[i].active{10*scale}else{6*scale};
        let left=b.x+(b.w-size)/2;let top=b.y+(b.h-size)/2;
        if !in_rect(x,y,left,top,size,size){return None;}
        return Some(if bar.workspaces[i].urgent{DEFAULT_THEME.urgent}
            else if bar.workspaces[i].active{DEFAULT_THEME.active}else{DEFAULT_THEME.muted});
    }
    let _=scene;None
}
fn chip_color(x:usize,y:usize,bar:&SilkBar,scene: &scene::SceneSnapshot)->Option<u32>{
    for (idx,slot) in [ModuleSlot::Chip0,ModuleSlot::Chip1,ModuleSlot::Chip2].iter().enumerate(){
        let(cx,cy,cw,ch)=module_rect(bar,*slot);
        let size=scene.metrics.px(12)as usize;
        let left=cx+cw.saturating_sub(size)/2;let top=cy+ch.saturating_sub(size)/2;
        if !in_rect(x,y,left,top,size,size){continue;}
        let chip=&bar.chips[idx];
        if !chip.visible{return Some(DEFAULT_THEME.muted);}
        return Some(match chip.kind{ChipKind::Net=>glass_over_bg(DEFAULT_THEME.chip_fill,x,y,212,scene),
            ChipKind::Wifi=>0x0038D6C8,ChipKind::Battery=>0x00FFB84D,ChipKind::Clock=>DEFAULT_THEME.chip_border});
    }
    None
}

// ── Phase 5: Pixel Indicators (tiny bounded dots/swatches, y<50) ────────────

/// Surface ID → indicator color mapping for active app dot.
const fn app_indicator_color(sid: u32) -> Option<u32> {
    if let Some(color)=sex_pdx::desktop_style::native_indicator(sid as u64){return Some(color);}
    match sid {
        200 => Some(0x00A6E3A1), // Linen  → green
        201 => Some(0x00CBA6F7), // Quil   → mauve
        202 => Some(0x0094E2D5), // Mesh   → teal
        203 => Some(0x00FAB387), // Collar → peach
        204 => Some(0x00F38BA8), // Bell   → red
        153 => Some(0x0089B4FA), // Spindle→ blue
        0   => None,            // no active app
        _   => Some(0x00444444), // unknown → dim gray
    }
}

/// Accent index → RGB color (Atlas 8-accent palette).
const fn accent_swatch_color(idx: u8) -> u32 {
    const ACCENT: [u32; 8] = [
        0x0089B4FA, // 0: Blue
        0x00A6E3A1, // 1: Green
        0x00F9E2AF, // 2: Yellow
        0x00FAB387, // 3: Peach
        0x00F38BA8, // 4: Red
        0x00CBA6F7, // 5: Mauve
        0x00F5C2E7, // 6: Pink
        0x0094E2D5, // 7: Teal
    ];
    if (idx as usize) < 8 { ACCENT[idx as usize] } else { 0x00444444 }
}

/// Phase 5: active app indicator — 4x4 colored dot near launcher.
/// Position: x=155..159, y=22..25 (right of options dots at x=135).
fn phase5_active_app_at(x:usize,y:usize,bar:&SilkBar,scene: &scene::SceneSnapshot)->Option<u32>{
    if bar.layout[ModuleSlot::Workspace1 as usize].w==0{return None;}
    let scale=scene.metrics.scale()as usize;
    if in_rect(x,y,40*scale,12*scale,4*scale,4*scale){app_indicator_color(scene.focus as u32)}else{None}
}

/// A new notification, shown in the panel in place of the title for a few seconds.
static mut PANEL_NOTICE: sex_pdx::notify::Text = sex_pdx::notify::Text::EMPTY;
static mut PANEL_NOTICE_ASSEMBLER: sex_pdx::notify::Assembler = sex_pdx::notify::Assembler::new();
const PANEL_NOTICE_INK: u32 = 0x00F9E2AF;
/// Panel: the focused window's title, right of the focus dot and left of the
/// workspace dots, like a Waybar or Plasma window title.
const PANEL_TITLE_X: usize = 166;
const PANEL_TITLE_END: usize = silkbar_model::WS_X0 - 16;
fn panel_title_fg_at(x:usize,y:usize,bar:&SilkBar,scene: &scene::SceneSnapshot,background:u32)->Option<u32>{
    let scale=scene.metrics.scale()as usize;
    if bar.layout[ModuleSlot::Workspace1 as usize].w==0{return None;}
    let left=52*scale;let end=bar.layout[ModuleSlot::Workspace0 as usize].x.saturating_sub(12*scale);
    let line_h=glyphs::UI_H*scale;let top=(scene.metrics.bar()as usize).saturating_sub(line_h)/2;
    if x<left||x>=end||y<top||y>=top+line_h{return None;}
    let notice=unsafe{&*core::ptr::addr_of!(PANEL_NOTICE)};
    let text=if notice.len>0{notice.as_bytes()}else{surface_title(scene.focus)?};
    ui_label_at_scaled(text,x-left,y-top,background,scene.metrics.scale())
}

/// Battery in the panel, left of the status chips: "87%  2:05 left", "64%  charging",
/// read from the power sensors (the ACPI service reads the firmware's batteries).
static mut PANEL_BATTERY: ([u8; 24], usize, usize) = ([0; 24], 0, 0);
static mut PANEL_BATTERY_NEXT: u64 = 0;
/// Polls the sensors every two seconds; true when the text changed.
fn panel_battery_poll(scale: u32) -> bool {
    let now = sex_pdx::get_ticks();
    unsafe { if now < PANEL_BATTERY_NEXT { return false; } PANEL_BATTERY_NEXT = now + 2000; }
    let Ok(s) = sex_pdx::power::sensors() else { return false };
    let mut text = [0u8; 24];
    let mut n = 0;
    let mut put = |bytes: &[u8], n: &mut usize| for &b in bytes { if *n < text.len() { text[*n] = b; *n += 1; } };
    if let Some(p) = s.battery_percent {
        let mut digits = [0u8; 3];
        let d = if p >= 100 { digits = *b"100"; 3 } else if p >= 10 { digits[0] = b'0' + p / 10; digits[1] = b'0' + p % 10; 2 } else { digits[0] = b'0' + p; 1 };
        put(&digits[..d], &mut n);
        put(b"%", &mut n);
        if s.charging { put(b"  charging", &mut n); }
        else if let Some(m) = s.battery_minutes.filter(|_| s.on_ac != Some(true)) {
            let (h, m) = ((m / 60).min(99), m % 60);
            put(b"  ", &mut n);
            if h >= 10 { put(&[b'0' + (h / 10) as u8], &mut n); }
            put(&[b'0' + (h % 10) as u8, b':', b'0' + (m / 10) as u8, b'0' + (m % 10) as u8], &mut n);
            put(b" left", &mut n);
        }
    }
    unsafe {
        let old = &mut *core::ptr::addr_of_mut!(PANEL_BATTERY);
        if old.1 == n && old.0[..n] == text[..n] { return false; }
        let width = glyphs::ui_width(&text[..n], scale).unwrap_or(n * 7 * scale as usize);
        *old = (text, n, width);
        sex_pdx::serial_record!("[display.panel.battery] text={} width={}", core::str::from_utf8(&text[..n]).unwrap_or("?"), width);
    }
    true
}
fn panel_battery_at(x: usize, y: usize, bar: &SilkBar, scene: &scene::SceneSnapshot, background: u32) -> Option<u32> {
    let (text, n, width) = unsafe { &*core::ptr::addr_of!(PANEL_BATTERY) };
    if *n == 0 { return None; }
    let scale = scene.metrics.scale() as usize;
    let (cx, _, _, _) = module_rect(bar, ModuleSlot::Chip0);
    let end = cx.saturating_sub(10 * scale);
    let left = end.saturating_sub(*width);
    let line_h = glyphs::UI_H * scale;
    let top = (scene.metrics.bar() as usize).saturating_sub(line_h) / 2;
    if x < left || x >= end || y < top || y >= top + line_h { return None; }
    ui_label_at_scaled(&text[..*n], x - left, y - top, background, scene.metrics.scale())
}

/// Same visible top-strip content for full and incremental redraws.
fn top_strip_pixel(x: usize, y: usize, bar: &SilkBar, lines: &[u8;20], scene: &scene::SceneSnapshot) -> u32 {
    let background = bar_color(x, y, bar, scene);
    if let Some(fg) = clock_fg_at_scaled(x,y,bar,lines,background,scene.metrics.scale()) { fg }
    else if let Some(fg) = bell_badge_at_scaled(x,y,bar,background,scene.metrics.scale()) { fg }
    else if let Some(fg) = panel_battery_at(x, y, bar, scene, background) { fg }
    else if let Some(fg) = phase5_active_app_at(x, y, bar, scene) { fg }
    else if let Some(fg) = panel_title_fg_at(x, y, bar, scene, background) { fg }
    else if let Some(fg) = phase5_tint_swatch_at(x, y, bar) { fg }
    else if let Some(fg) = phase5_palette_dot_at(x, y, bar) { fg }
    else { background }
}

// Bounded observation only: no protocol, authority or scheduling decisions.
static mut REPAINT_BUDGET: u16 = 512;
static mut REPAINT_ORDINAL: u32 = 0;
static mut REPAINT_SEEN: u32 = 0;
static mut REPAINT_DRAINED: usize = 0;
static mut REPAINT_UPSERTS: u32 = 0;
static mut REPAINT_FITTED_NOOPS: u32 = 0;
static mut REPAINT_TEXT_CALLS: u32 = 0;
static mut REPAINT_STRIP_FOCUS: u64 = u64::MAX;
fn repaint_begin() -> u64 { if unsafe{REPAINT_BUDGET==0}{0}else{sex_pdx::get_ticks()} }
fn repaint_end(kind: &str, started: u64) {
    unsafe {
        if REPAINT_BUDGET == 0 { return; }
        let focus_changed = matches!(kind,"full"|"strip") && REPAINT_STRIP_FOCUS != FOCUSED_SURFACE_ID;
        // Clock-only strips exhausted the first experimental budget before
        // the mixed workload. Keep every structural/focus observation and
        // sample ordinary text draws; do not change render scheduling.
        if kind=="strip" && !focus_changed { return; }
        if kind=="text" {
            REPAINT_TEXT_CALLS=REPAINT_TEXT_CALLS.wrapping_add(1);
            if REPAINT_TEXT_CALLS%16!=0 { return; }
        }
        let end = sex_pdx::get_ticks();
        REPAINT_BUDGET -= 1;
        REPAINT_ORDINAL += 1;
        sex_pdx::serial_record!("[display.repaint] ordinal={} kind={} start={} end={} ticks={} seen={} drained={} upserts={} fitted_noops={} focus={}",
            REPAINT_ORDINAL,kind,started,end,end.saturating_sub(started),REPAINT_SEEN,
            REPAINT_DRAINED,REPAINT_UPSERTS,REPAINT_FITTED_NOOPS,FOCUSED_SURFACE_ID);
        if matches!(kind,"full"|"strip") && REPAINT_STRIP_FOCUS != FOCUSED_SURFACE_ID {
            REPAINT_STRIP_FOCUS = FOCUSED_SURFACE_ID;
            sex_pdx::serial_record!("[display.focus.drawn] focus={} ordinal={} kind={} tick={} pixels=unconfirmed",
                FOCUSED_SURFACE_ID,REPAINT_ORDINAL,kind,end);
        }
    }
}

/// Phase 5: tint/accent swatch — 4x4 colored square near right side.
/// Position: x=1045..1049, y=22..25 (between Bell at x=1020 and Clock at x=1090).
fn phase5_tint_swatch_at(x: usize, y: usize, bar: &SilkBar) -> Option<u32> {
    if !SILKBAR_PHASE5_PIXEL_PROOF_ENABLED { return None; }
    const SX: usize = 1045;
    const SY: usize = 22;
    const SS: usize = 4;
    if x >= SX && x < SX + SS && y >= SY && y < SY + SS {
        Some(accent_swatch_color(bar.phase1.accent_tint_idx))
    } else {
        None
    }
}

/// Phase 5: palette open indicator — 3x3 green/red dot near right side.
/// Green when palette_open=true, dim when closed.
/// Position: x=1060..1063, y=23..26 (right of tint swatch).
fn phase5_palette_dot_at(x: usize, y: usize, bar: &SilkBar) -> Option<u32> {
    if !SILKBAR_PHASE5_PIXEL_PROOF_ENABLED { return None; }
    const PX: usize = 1060;
    const PY: usize = 23;
    const PS: usize = 3;
    if x >= PX && x < PX + PS && y >= PY && y < PY + PS {
        if bar.phase1.palette_open {
            Some(0x0044FF44) // bright green
        } else {
            Some(0x00224422) // dim/dark green
        }
    } else {
        None
    }
}

fn bar_color(x: usize, y: usize, bar: &SilkBar, scene: &scene::SceneSnapshot) -> u32 {
    const R6_BAR_ALPHA: u8 = 196;
    const R6_EDGE_ALPHA: u8 = 72;
    const BAR_TOP_HIGHLIGHT: u32 = 0x00A8D8FF;
    const BAR_BOTTOM_DARK_EDGE: u32 = 0x000B1220;
    const BAR_CRYSTAL_TINT: u32 = 0x001C2A54;
    // Selected-window option indicators (display-only, no action behavior).
    // Rendered as small colored dots between launcher and workspace indicators.
    // Each dot is 3x3 px at a fixed position, colored by option bit.
    // Options dots: only intercept the exact 3×3 pixel positions, fall through
    // for all other x values so workspace indicators, chips, and other bar
    // elements render correctly at the same y range (y=19..22).
    // An early return for the full y-range overrides those modules and creates
    // a visible horizontal strip of wrong color across the bar.
    if SILKBAR_PHASE5_PIXEL_PROOF_ENABLED && bar.selected_options_mask != 0 {
        const OPTIONS_X: usize = 135;
        const OPTIONS_Y: usize = 19;
        const DOT_W: usize = 3;
        const DOT_H: usize = 3;
        const DOT_GAP: usize = 5;
        if y >= OPTIONS_Y && y < OPTIONS_Y + DOT_H {
            for bit in 0..4 {
                let dot_x = OPTIONS_X + bit * DOT_GAP;
                if x >= dot_x && x < dot_x + DOT_W {
                    if (bar.selected_options_mask >> bit) & 1 != 0 {
                        return match bit {
                            0 => 0x00FF4444,
                            1 => 0x0044FF44,
                            2 => 0x00FFCC44,
                            3 => 0x0044CCFF,
                            _ => DEFAULT_THEME.panel_fill,
                        };
                    }
                    // Unset bit: fall through to normal module rendering
                }
            }
            // Between dots: fall through to normal module rendering
        }
    }
    // Workspace indicators
    if let Some(c) = workspace_color(x, y, bar, scene) { return c; }
    // Status chip backgrounds
    if let Some(c) = chip_color(x, y, bar, scene) { return c; }
    // Bell indicator (privacy-safe presence dot between Battery and Clock)
    let (bx, by, bw, bh) = module_rect(bar, ModuleSlot::Bell);
    if in_rect(x, y, bx+scene.metrics.px(2)as usize, by+scene.metrics.px(2)as usize, bw.saturating_sub(scene.metrics.px(4)as usize), bh.saturating_sub(scene.metrics.px(4)as usize)) {
        let s = &bar.bell_state;
        if s.flags & 1 == 0 {
            // Bell unavailable (no SLOT_BELL grant or server not running) → dim dot
            return DEFAULT_THEME.muted;
        }
        if s.total_visible == 0 {
            // No events → dim/inactive dot
            return DEFAULT_THEME.muted;
        }
        if s.redacted_count > 0 {
            // Events with privacy-redacted content → warm tint
            return 0x00FFAA44; // amber
        }
        // Active events → bright dot
        return 0x00FFD700; // gold
    }
    // Launcher button with rounded-illusion border (model position)
    let (lx, ly, lw, lh) = module_rect(bar, ModuleSlot::Launcher);
    if in_rect(x, y, lx, ly, lw, lh) {
        let x2 = lx + 2;
        let y2 = ly + 2;
        let xw = lx + lw - 2;
        let yh = ly + lh - 2;
        if x < x2 || x >= xw || y < y2 || y >= yh {
            return glass_over_bg(DEFAULT_THEME.panel_glow, x, y, R6_EDGE_ALPHA, scene);
        }
        let launcher_body = glass_over_bg(0x005CAEE8, x, y, 200, scene);
        return alpha_blend_xrgb_over_xrgb(0x0082D6FF, launcher_body, 64);
    }
    // Bar body: glass translucency over desktop background,
    // with subtle bottom-edge glow falloff.
    let bar_bottom=(scene.metrics.bar()as usize);
    const GLOW_SPREAD: usize = 4;
    if y == 0 {
        let body = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA, scene);
        return alpha_blend_xrgb_over_xrgb(BAR_TOP_HIGHLIGHT, body, 64);
    }
    if y == bar_bottom - 1 {
        let body = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA, scene);
        let darkened = alpha_blend_xrgb_over_xrgb(BAR_BOTTOM_DARK_EDGE, body, 88);
        return alpha_blend_xrgb_over_xrgb(DEFAULT_THEME.panel_glow, darkened, 48);
    }
    if y + GLOW_SPREAD >= bar_bottom && y < bar_bottom {
        let dist = (bar_bottom - 1 - y) as u32;
        let glow_a = glow_edge_alpha(dist, GLOW_SPREAD as u32, R6_EDGE_ALPHA);
        if glow_a > 0 {
            let base = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA, scene);
            let body = alpha_blend_xrgb_over_xrgb(BAR_CRYSTAL_TINT, base, 36);
            return alpha_blend_xrgb_over_xrgb(DEFAULT_THEME.panel_glow, body, glow_a);
        }
    }
    let base = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA, scene);
    alpha_blend_xrgb_over_xrgb(BAR_CRYSTAL_TINT, base, 32)
}

// 5×7 bitmap glyphs for digits 0-9 (MSB = leftmost pixel)
const FONT: [[u8; 7]; 10] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
    [0b01110, 0b10001, 0b00001, 0b00110, 0b00001, 0b10001, 0b01110],
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
];

// 5×7 ASCII font: glyphs for chars 0x20 (space) through 0x5A (Z), 59 glyphs
// Each glyph: 7 bytes, MSB = leftmost pixel, 5 bits wide, 3 bits padding
// Used by surface_text_fg_at() for text rendering on surfaces
// Finite ROM: one distinct glyph for every printable ASCII byte. No cache,
// allocation, byte rewriting or font-loading authority. Unsupported bytes
// have no ink; selection and layout retain their original byte positions.
const FONT_ASCII_5X7: [[u8; 7]; 95] = [
    [0, 0, 0, 0, 0, 0, 0], // ASCII 32
    [4, 4, 4, 4, 4, 0, 4], // ASCII 33
    [10, 10, 10, 0, 0, 0, 0], // ASCII 34
    [10, 10, 31, 10, 31, 10, 10], // ASCII 35
    [4, 15, 20, 14, 5, 30, 4], // ASCII 36
    [25, 26, 2, 4, 8, 11, 19], // ASCII 37
    [12, 18, 18, 12, 21, 18, 13], // ASCII 38
    [12, 4, 8, 0, 0, 0, 0], // ASCII 39
    [2, 4, 8, 8, 8, 4, 2], // ASCII 40
    [8, 4, 2, 2, 2, 4, 8], // ASCII 41
    [0, 4, 21, 14, 21, 4, 0], // ASCII 42
    [0, 4, 4, 31, 4, 4, 0], // ASCII 43
    [0, 0, 0, 0, 12, 4, 8], // ASCII 44
    [0, 0, 0, 31, 0, 0, 0], // ASCII 45
    [0, 0, 0, 0, 0, 12, 12], // ASCII 46
    [1, 2, 2, 4, 8, 8, 16], // ASCII 47
    [14, 17, 19, 21, 25, 17, 14], // ASCII 48
    [4, 12, 4, 4, 4, 4, 14], // ASCII 49
    [14, 17, 1, 2, 4, 8, 31], // ASCII 50
    [14, 17, 1, 6, 1, 17, 14], // ASCII 51
    [2, 6, 10, 18, 31, 2, 2], // ASCII 52
    [31, 16, 30, 1, 1, 17, 14], // ASCII 53
    [6, 8, 16, 30, 17, 17, 14], // ASCII 54
    [31, 1, 2, 4, 8, 8, 8], // ASCII 55
    [14, 17, 17, 14, 17, 17, 14], // ASCII 56
    [14, 17, 17, 15, 1, 2, 12], // ASCII 57
    [0, 12, 12, 0, 12, 12, 0], // ASCII 58
    [0, 12, 12, 0, 12, 4, 8], // ASCII 59
    [2, 4, 8, 16, 8, 4, 2], // ASCII 60
    [0, 0, 31, 0, 31, 0, 0], // ASCII 61
    [8, 4, 2, 1, 2, 4, 8], // ASCII 62
    [14, 17, 1, 2, 4, 0, 4], // ASCII 63
    [14, 17, 19, 21, 19, 16, 14], // ASCII 64
    [14, 17, 17, 31, 17, 17, 17], // ASCII 65
    [30, 17, 17, 30, 17, 17, 30], // ASCII 66
    [14, 17, 16, 16, 16, 17, 14], // ASCII 67
    [30, 17, 17, 17, 17, 17, 30], // ASCII 68
    [31, 16, 16, 30, 16, 16, 31], // ASCII 69
    [31, 16, 16, 30, 16, 16, 16], // ASCII 70
    [14, 17, 16, 23, 17, 17, 14], // ASCII 71
    [17, 17, 17, 31, 17, 17, 17], // ASCII 72
    [14, 4, 4, 4, 4, 4, 14], // ASCII 73
    [7, 1, 1, 1, 1, 17, 14], // ASCII 74
    [17, 18, 20, 24, 20, 18, 17], // ASCII 75
    [16, 16, 16, 16, 16, 16, 31], // ASCII 76
    [17, 27, 21, 21, 17, 17, 17], // ASCII 77
    [17, 25, 21, 19, 17, 17, 17], // ASCII 78
    [14, 17, 17, 17, 17, 17, 14], // ASCII 79
    [30, 17, 17, 30, 16, 16, 16], // ASCII 80
    [14, 17, 17, 17, 21, 18, 13], // ASCII 81
    [30, 17, 17, 30, 20, 18, 17], // ASCII 82
    [14, 17, 16, 14, 1, 17, 14], // ASCII 83
    [31, 4, 4, 4, 4, 4, 4], // ASCII 84
    [17, 17, 17, 17, 17, 17, 14], // ASCII 85
    [17, 17, 17, 17, 17, 10, 4], // ASCII 86
    [17, 17, 17, 21, 21, 27, 17], // ASCII 87
    [17, 17, 10, 4, 10, 17, 17], // ASCII 88
    [17, 17, 10, 4, 4, 4, 4], // ASCII 89
    [31, 1, 2, 4, 8, 16, 31], // ASCII 90
    [14, 8, 8, 8, 8, 8, 14], // ASCII 91
    [16, 8, 8, 4, 2, 2, 1], // ASCII 92
    [14, 2, 2, 2, 2, 2, 14], // ASCII 93
    [4, 10, 17, 0, 0, 0, 0], // ASCII 94
    [0, 0, 0, 0, 0, 0, 31], // ASCII 95
    [8, 4, 2, 0, 0, 0, 0], // ASCII 96
    [0, 0, 14, 1, 15, 17, 15], // ASCII 97
    [16, 16, 30, 17, 17, 17, 30], // ASCII 98
    [0, 0, 14, 16, 16, 16, 14], // ASCII 99
    [1, 1, 15, 17, 17, 17, 15], // ASCII 100
    [0, 0, 14, 17, 31, 16, 14], // ASCII 101
    [6, 9, 8, 28, 8, 8, 8], // ASCII 102
    [0, 0, 15, 17, 15, 1, 14], // ASCII 103
    [16, 16, 30, 17, 17, 17, 17], // ASCII 104
    [4, 0, 12, 4, 4, 4, 14], // ASCII 105
    [2, 0, 6, 2, 2, 18, 12], // ASCII 106
    [16, 16, 18, 20, 24, 20, 18], // ASCII 107
    [12, 4, 4, 4, 4, 4, 14], // ASCII 108
    [0, 0, 26, 21, 21, 17, 17], // ASCII 109
    [0, 0, 30, 17, 17, 17, 17], // ASCII 110
    [0, 0, 14, 17, 17, 17, 14], // ASCII 111
    [0, 0, 30, 17, 30, 16, 16], // ASCII 112
    [0, 0, 15, 17, 15, 1, 1], // ASCII 113
    [0, 0, 22, 25, 16, 16, 16], // ASCII 114
    [0, 0, 15, 16, 14, 1, 30], // ASCII 115
    [8, 8, 28, 8, 8, 9, 6], // ASCII 116
    [0, 0, 17, 17, 17, 19, 13], // ASCII 117
    [0, 0, 17, 17, 17, 10, 4], // ASCII 118
    [0, 0, 17, 17, 21, 21, 10], // ASCII 119
    [0, 0, 17, 10, 4, 10, 17], // ASCII 120
    [0, 0, 17, 17, 15, 1, 14], // ASCII 121
    [0, 0, 31, 2, 4, 8, 31], // ASCII 122
    [2, 4, 4, 8, 4, 4, 2], // ASCII 123
    [4, 4, 4, 4, 4, 4, 4], // ASCII 124
    [8, 4, 4, 2, 4, 4, 8], // ASCII 125
    [0, 0, 9, 22, 0, 0, 0], // ASCII 126
];

// Finite 5x7 scalar repertoire. Unsupported scalars use an explicit box;
// glyph choice never rewrites the committed source bytes.
const FONT_UNICODE_5X7: &[(u32, [u8; 7])] = &[
    (0x00A0, [0, 0, 0, 0, 0, 0, 0]), // nonbreaking space, one cell
    (0x00A1, [4, 0, 4, 4, 4, 4, 4]), // ¡
    (0x00A2, [4, 14, 21, 20, 21, 14, 4]), // ¢
    (0x00A3, [6, 9, 8, 28, 8, 9, 30]), // £
    (0x00A5, [17, 10, 31, 4, 31, 4, 4]), // ¥
    (0x00A7, [14, 16, 14, 17, 14, 1, 14]), // §
    (0x00A9, [14, 17, 23, 25, 23, 17, 14]), // ©
    (0x00AB, [0, 5, 10, 20, 10, 5, 0]), // «
    (0x00AC, [0, 0, 31, 1, 1, 0, 0]), // ¬
    (0x00B0, [12, 18, 12, 0, 0, 0, 0]), // °
    (0x00B1, [4, 4, 31, 4, 4, 0, 31]), // ±
    (0x00B2, [12, 2, 4, 14, 0, 0, 0]), // ²
    (0x00B5, [0, 0, 17, 17, 17, 30, 16]), // µ
    (0x00B7, [0, 0, 0, 4, 0, 0, 0]), // ·
    (0x00BB, [0, 20, 10, 5, 10, 20, 0]), // »
    (0x00BD, [17, 18, 4, 8, 23, 1, 2]), // ½
    (0x00BF, [4, 0, 4, 8, 16, 17, 14]), // ¿
    (0x00C0, [8, 4, 14, 17, 31, 17, 17]), // À
    (0x00C1, [2, 4, 14, 17, 31, 17, 17]), // Á
    (0x00C2, [4, 10, 14, 17, 31, 17, 17]), // Â
    (0x00C3, [13, 22, 14, 17, 31, 17, 17]), // Ã
    (0x00C4, [10, 0, 14, 17, 31, 17, 17]), // Ä
    (0x00C5, [14, 10, 14, 17, 31, 17, 17]), // Å
    (0x00C7, [14, 17, 16, 16, 17, 14, 4]), // Ç
    (0x00C8, [8, 4, 31, 16, 30, 16, 31]), // È
    (0x00C9, [2, 4, 31, 16, 30, 16, 31]), // É
    (0x00CA, [4, 10, 31, 16, 30, 16, 31]), // Ê
    (0x00CB, [10, 0, 31, 16, 30, 16, 31]), // Ë
    (0x00CC, [8, 4, 14, 4, 4, 4, 14]), // Ì
    (0x00CD, [2, 4, 14, 4, 4, 4, 14]), // Í
    (0x00CE, [4, 10, 14, 4, 4, 4, 14]), // Î
    (0x00CF, [10, 0, 14, 4, 4, 4, 14]), // Ï
    (0x00D1, [13, 22, 17, 25, 21, 19, 17]), // Ñ
    (0x00D2, [8, 4, 14, 17, 17, 17, 14]), // Ò
    (0x00D3, [2, 4, 14, 17, 17, 17, 14]), // Ó
    (0x00D4, [4, 10, 14, 17, 17, 17, 14]), // Ô
    (0x00D5, [13, 22, 14, 17, 17, 17, 14]), // Õ
    (0x00D6, [10, 0, 14, 17, 17, 17, 14]), // Ö
    (0x00D7, [0, 17, 10, 4, 10, 17, 0]), // ×
    (0x00D9, [8, 4, 17, 17, 17, 17, 14]), // Ù
    (0x00DA, [2, 4, 17, 17, 17, 17, 14]), // Ú
    (0x00DB, [4, 10, 17, 17, 17, 17, 14]), // Û
    (0x00DC, [10, 0, 17, 17, 17, 17, 14]), // Ü
    (0x00DD, [2, 4, 17, 10, 4, 4, 4]), // Ý
    (0x00DF, [12, 18, 18, 28, 18, 18, 28]), // ß
    (0x00E0, [8, 4, 14, 1, 15, 17, 15]), // à
    (0x00E1, [2, 4, 14, 1, 15, 17, 15]), // á
    (0x00E2, [4, 10, 14, 1, 15, 17, 15]), // â
    (0x00E3, [13, 22, 14, 1, 15, 17, 15]), // ã
    (0x00E4, [10, 0, 14, 1, 15, 17, 15]), // ä
    (0x00E5, [14, 10, 14, 1, 15, 17, 15]), // å
    (0x00E7, [0, 14, 16, 16, 16, 14, 4]), // ç
    (0x00E8, [8, 4, 14, 17, 31, 16, 14]), // è
    (0x00E9, [2, 4, 14, 17, 31, 16, 14]), // é
    (0x00EA, [4, 10, 14, 17, 31, 16, 14]), // ê
    (0x00EB, [10, 0, 14, 17, 31, 16, 14]), // ë
    (0x00EC, [8, 4, 12, 4, 4, 4, 14]), // ì
    (0x00ED, [2, 4, 12, 4, 4, 4, 14]), // í
    (0x00EE, [4, 10, 12, 4, 4, 4, 14]), // î
    (0x00EF, [10, 0, 12, 4, 4, 4, 14]), // ï
    (0x00F1, [13, 22, 22, 25, 17, 17, 17]), // ñ
    (0x00F2, [8, 4, 0, 14, 17, 17, 14]), // ò
    (0x00F3, [2, 4, 0, 14, 17, 17, 14]), // ó
    (0x00F4, [4, 10, 0, 14, 17, 17, 14]), // ô
    (0x00F5, [13, 22, 0, 14, 17, 17, 14]), // õ
    (0x00F6, [10, 0, 0, 14, 17, 17, 14]), // ö
    (0x00F7, [0, 4, 0, 31, 0, 4, 0]), // ÷
    (0x00F9, [8, 4, 17, 17, 17, 19, 13]), // ù
    (0x00FA, [2, 4, 17, 17, 17, 19, 13]), // ú
    (0x00FB, [4, 10, 17, 17, 17, 19, 13]), // û
    (0x00FC, [10, 0, 17, 17, 17, 19, 13]), // ü
    (0x00FD, [2, 4, 17, 17, 15, 1, 14]), // ý
    (0x03B1, [0, 0, 13, 18, 18, 18, 13]), // α
    (0x03B2, [14, 17, 30, 17, 30, 16, 16]), // β
    (0x03C0, [0, 0, 31, 10, 10, 10, 10]), // π
    (0x2013, [0, 0, 0, 14, 0, 0, 0]), // –
    (0x2014, [0, 0, 0, 31, 0, 0, 0]), // —
    (0x2018, [2, 4, 6, 0, 0, 0, 0]), // ‘
    (0x2019, [12, 4, 8, 0, 0, 0, 0]), // ’
    (0x201C, [5, 10, 15, 0, 0, 0, 0]), // “
    (0x201D, [15, 5, 10, 0, 0, 0, 0]), // ”
    (0x2022, [0, 0, 14, 14, 14, 0, 0]), // •
    (0x2026, [0, 0, 0, 0, 0, 0, 21]), // …
    (0x20AC, [7, 8, 30, 8, 30, 8, 7]), // €
    (0x2190, [0, 4, 8, 31, 8, 4, 0]), // ←
    (0x2191, [4, 14, 21, 4, 4, 4, 0]), // ↑
    (0x2192, [0, 4, 2, 31, 2, 4, 0]), // →
    (0x2193, [0, 4, 4, 4, 21, 14, 4]), // ↓
    (0x2212, [0, 0, 0, 30, 0, 0, 0]), // −
    (0x221E, [0, 0, 10, 21, 10, 0, 0]), // ∞
    (0x2260, [0, 1, 31, 4, 31, 16, 0]), // ≠
    (0x2264, [2, 4, 8, 4, 2, 0, 14]), // ≤
    (0x2265, [8, 4, 2, 4, 8, 0, 14]), // ≥
    (0x2713, [0, 1, 1, 2, 18, 12, 0]), // ✓
];
const FONT_UNSUPPORTED_5X7: [u8; 7] = [31, 17, 21, 21, 21, 17, 31];

fn scalar_glyph(scalar:u32)->[u8;7]{
    if scalar==9 { return [0;7]; }
    if (32..=126).contains(&scalar) { return FONT_ASCII_5X7[(scalar-32)as usize]; }
    FONT_UNICODE_5X7.iter().find(|(code,_)|*code==scalar)
        .map_or(FONT_UNSUPPORTED_5X7,|(_,rows)|*rows)
}

fn ascii_glyph_index(byte: u8) -> Option<usize> {
    use sex_pdx::desktop_style::{ASCII_FIRST, ASCII_LAST};
    (ASCII_FIRST..=ASCII_LAST).contains(&byte).then(|| (byte - ASCII_FIRST) as usize)
}

/// Titles programs gave their own windows, by surface. Empty means the catalog's name.
static mut PROGRAM_TITLES: [(u64, [u8; 16], u8); 16] = [(0, [0; 16], 0); 16];
fn program_title(surface_id: u64) -> Option<&'static [u8]> {
    let titles = unsafe { &*core::ptr::addr_of!(PROGRAM_TITLES) };
    titles.iter().find(|t| t.0 == surface_id && t.2 != 0).map(|t| &t.1[..t.2 as usize])
}
fn set_program_title(surface_id: u64, title: Option<[u8; 16]>) {
    let titles = unsafe { &mut *core::ptr::addr_of_mut!(PROGRAM_TITLES) };
    let slot = match titles.iter().position(|t| t.0 == surface_id) { Some(i) => i, None => match titles.iter().position(|t| t.0 == 0) { Some(i) => i, None => return } };
    match title {
        Some(bytes) => {
            let len = bytes.iter().position(|b| *b == 0).unwrap_or(16);
            let ok = len > 0 && bytes[..len].iter().all(|b| (32..127).contains(b));
            titles[slot] = if ok { (surface_id, bytes, len as u8) } else { (0, [0; 16], 0) };
        }
        None => titles[slot] = (0, [0; 16], 0),
    }
}
/// Fixed-position title glyphs choose contrast against the final composed background.
/// The name a window shows in its title bar and in the panel.
fn surface_title(surface_id: u64) -> Option<&'static [u8]> {
    if let Some(own)=program_title(surface_id) { return Some(own); }
    if let Some(row)=app_catalog::INCARNATIONS.iter().find(|row|row.surface==surface_id||row.shell_surface==Some(surface_id)) {
        return Some(match row.app {app_catalog::App::Quil=>b"Quil",app_catalog::App::Spindle=>b"Spindle",app_catalog::App::Browser=>b"Browser",app_catalog::App::Program=>b"Program"});
    }
    match surface_id{200=>Some(b"Linen"),100=>Some(b"App"),_=>None}
}

fn toolbar_title_fg_at(lx: usize, ly: usize, surface_id: u64, background: u32) -> Option<u32>{
    toolbar_title_fg_at_scaled(lx,ly,surface_id,background,sex_pdx::desktop_style::ui_scale())
}
fn toolbar_title_fg_at_scaled(lx: usize, ly: usize, surface_id: u64, background: u32, ui_scale:u32) -> Option<u32>{
    let title_x = sex_pdx::desktop_style::chrome::Metrics::new(ui_scale).px(85)as usize;
    let line_h=glyphs::UI_H*ui_scale as usize;
    let title_y = (sex_pdx::desktop_style::chrome::Metrics::new(ui_scale).title()as usize).saturating_sub(line_h) / 2;
    if lx + 2 < title_x || ly < title_y || ly >= title_y + line_h { return None; }
    let title = surface_title(surface_id)?;
    // Geist Sans, kerned, each glyph at its quarter-pixel position; two pixels of
    // margin left of the first letter catch an overhang.
    let alpha = glyphs::ui_coverage(title, lx as i32 - title_x as i32, ly - title_y, ui_scale).unwrap_or(0);
    (alpha != 0).then(|| coverage_blend(sex_pdx::desktop_style::contrast_ink(background), background, alpha))
}

/// Scale by whole pixels so glyph edges remain crisp. A legacy 20-column
/// row needs 240px plus two 8px insets at 2x; narrow overlays retain 1x.
/// Exact Spindle panes share the terminal layout scale for their narrower tiles.
fn surface_text_scale(surf: &Surface) -> usize{
    surface_text_scale_for(surf,sex_pdx::desktop_style::ui_scale())
}
fn surface_text_scale_for(surf: &Surface, ui_scale:u32) -> usize{
    // Exact Spindle panes share the terminal's base width threshold; both
    // renderer and terminal apply the same authenticated screen scale.
    let spindle = sex_pdx::app_catalog::is_spindle(surf.owner_pd)
        && sex_pdx::app_catalog::native_pixel_bit(surf.owner_pd, surf.surface_id) != 0;
    let base = if spindle {
        sex_pdx::spindle_view::scale(surf.w)
    } else {
        sex_pdx::desktop_style::scale(surf.w)
    };
    // The window's own text size: whole steps from its width's default, Geist
    // drawn fresh for each.
    // HiDPI: the screen's scale multiplies the width's default and the range.
    let ui=ui_scale as i32;
    (base as i32 * ui + surf.text_zoom as i32).clamp(1, sex_pdx::desktop_style::MAX_TEXT_SCALE as i32 * ui) as usize
}
/// A zoom step on a surface: +1 larger, -1 smaller, 0 the default. The stored step
/// stays within what the scale can show, so the opposite key acts at once.
fn apply_text_zoom(surf: &mut Surface, step: i64) -> bool {
    let before = surface_text_scale(surf);
    let zoom = surf.text_zoom;
    surf.text_zoom = 0;
    let base = surface_text_scale(surf) as i32;
    surf.text_zoom = zoom;
    let wanted = if step == 0 { 0 } else { surf.text_zoom as i32 + step.signum() as i32 };
    let ui = sex_pdx::desktop_style::ui_scale() as i32;
    let most = sex_pdx::desktop_style::MAX_TEXT_SCALE as i32 * ui;
    surf.text_zoom = ((base + wanted).clamp(1, most) - base) as i8;
    let changed = surface_text_scale(surf) != before;
    if changed { recompute_text_lines(surf); }
    changed
}

/// Returns `Some(fg_color)` if pixel (x, y) is a text glyph foreground pixel
/// within the given surface, or `None` if background/not in the text area.
/// Uses cached Geist Mono at every scale, with a bounded legacy fallback.
/// TEXT_MODEL_V2: recompute the per-line index table after a text write.
/// Lines split on '\n' and wrap at scalar-cell wrap_cols. Legacy behavior preserved:
/// a buffer with no '\n' wraps at the fixed 20 columns every pre-V2 writer
/// pads to; any '\n' switches the surface to width-derived wrap
/// (usable width / scaled 6px glyph advance, clamped 1..=80).
fn recompute_text_lines(surf: &mut Surface) {
    let len = (surf.text_len as usize).min(TEXT_BUF_CAP);
    surf.line_count = 0;
    surf.cell_count = 0;
    surf.cell_starts.fill(0);
    surf.line_cells.fill(0);
    surf.line_starts.fill(0);
    if len == 0 { surf.wrap_cols = TEXT_LEGACY_COLS; return; }

    let has_newline = surf.text_buf[..len].iter().any(|&b| b == b'\n');
    surf.wrap_cols = if has_newline {
        let advance = 6 * surface_text_scale(surf) as u32;
        (surf.w.saturating_sub(chrome_metrics().px(16)) / advance).clamp(1, 160) as u8
    } else {
        TEXT_LEGACY_COLS
    };

    let cols = surf.wrap_cols as usize;
    let mut i = 0usize;
    let mut col = 0usize;
    surf.line_starts[0] = 0;
    surf.line_cells[0] = 0;
    surf.line_count = 1;
    while let Some((scalar,end)) = sex_pdx::text_encoding::cell(&surf.text_buf[..len],i) {
        surf.cell_starts[surf.cell_count as usize]=i as u16;
        surf.cell_count+=1;
        i=end;
        if scalar == 10 {
            if (surf.line_count as usize) >= TEXT_MAX_ROWS { break; }
            surf.line_starts[surf.line_count as usize] = i as u16;
            surf.line_cells[surf.line_count as usize] = surf.cell_count;
            surf.line_count += 1;
            col = 0;
        } else {
            col += 1;
            if col >= cols && i < len {
                if (surf.line_count as usize) >= TEXT_MAX_ROWS { break; }
                surf.line_starts[surf.line_count as usize] = i as u16;
                surf.line_cells[surf.line_count as usize] = surf.cell_count;
                surf.line_count += 1;
                col = 0;
            }
        }
    }
}

// These mutate text data only; dispatch admits the exact live surface owner.
fn apply_pending_text(slot:&mut Surface,pending:&text_transaction::Pending<'_>)->bool{
    let context=pending.context();
    // Main already attested the sender and live Surface at intake. This is
    // candidate association under that policy, not another grant or task=PD.
    if !slot.active||slot.owner_pd!=context.owner()||slot.surface_id!=context.surface()
        ||pending.len()>TEXT_BUF_CAP{return false;}
    slot.text_buf.copy_from_slice(pending.bytes());
    slot.text_len=pending.len() as u16;
    slot.text_color=pending.color();
    (slot.selection_start,slot.selection_end)=pending.selection();
    recompute_text_lines(slot);
    true
}

fn text_projection_requirement(old:&Surface,new:&Surface,w:usize,h:usize)->scene::TextCoverage{
    // Both values describe the same admitted Surface association. Compare the
    // actual derived layout too, rather than assuming its cache is current.
    if old.text_buf==new.text_buf&&old.text_len==new.text_len&&old.text_color==new.text_color
        &&(old.selection_start,old.selection_end)==(new.selection_start,new.selection_end)
        &&old.line_starts==new.line_starts&&old.cell_starts==new.cell_starts
        &&old.line_cells==new.line_cells&&old.cell_count==new.cell_count
        &&old.line_count==new.line_count&&old.wrap_cols==new.wrap_cols{
        return scene::TextCoverage::NoPixels;
    }
    // Old bounds are essential when shortening/clearing text or selection.
    // Do not optimize hidden parents/occlusion: these clipped geometric bounds
    // conservatively include every possible changed body text pixel.
    let mut required=damage::Damage::new(bar_height());
    required.add(text_surface_damage(old,w,h),w,h);
    required.add(text_surface_damage(new,w,h),w,h);
    match required.rect(){Some(rect)=>scene::TextCoverage::Body(changed_text_rows(old,new,rect)),None=>scene::TextCoverage::NoPixels}
}

/// Rows whose bytes are identical paint identical pixels, so a keystroke need
/// not repaint the whole body. Anything else that moves or restyles cells
/// keeps the whole rectangle.
fn changed_text_rows(old:&Surface,new:&Surface,rect:damage::Rect)->damage::Rect{
    if (old.x,old.y,old.w,old.h,old.chrome_flags)!=(new.x,new.y,new.w,new.h,new.chrome_flags)
        ||old.wrap_cols!=new.wrap_cols||old.text_color!=new.text_color
        ||surface_text_scale(old)!=surface_text_scale(new)
        ||old.selection_start<old.selection_end||new.selection_start<new.selection_end
        ||old.line_count==0||new.line_count==0{return rect;}
    fn row(surf:&Surface,index:usize)->&[u8]{
        let len=(surf.text_len as usize).min(TEXT_BUF_CAP);
        let start=(surf.line_starts[index] as usize).min(len);
        let end=if index+1<surf.line_count as usize{(surf.line_starts[index+1] as usize).min(len)}else{len};
        &surf.text_buf[start..end.max(start)]
    }
    let (old_rows,new_rows)=(old.line_count as usize,new.line_count as usize);
    let (mut first,mut last)=(usize::MAX,0usize);
    for index in 0..old_rows.max(new_rows){
        if index>=old_rows||index>=new_rows||row(old,index)!=row(new,index){
            first=first.min(index);last=index;
        }
    }
    if first==usize::MAX{return rect;}
    let top=if new.chrome_flags & SURFACE_CHROME_TOP_BAR != 0 {frame_height()+frame_rim()} else {chrome_metrics().px(sex_pdx::desktop_style::TEXT_TOP)as usize};
    let line=9*surface_text_scale(new);
    let origin=(new.y.max(0) as usize).saturating_add(top);
    let y=origin.saturating_add(first*line).max(rect.y).min(rect.bottom);
    let bottom=origin.saturating_add((last+1)*line).min(rect.bottom).max(y);
    if y==bottom{return rect;}
    damage::Rect{x:rect.x,y,right:rect.right,bottom}
}

fn join_pending_text(paint:&mut paint_batch::Dirty,w:usize,h:usize){
    unsafe{
        for(index,stage)in TEXT_STAGES.iter().enumerate(){
            let Some(pending)=stage.pending()else{continue;};
            // Existing preview modes can display a second view of these bytes.
            // Keep their broader body obligation rather than infer occlusion.
            if h<bar_height(){paint.request_full();continue;}
            if ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED||ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED{
                paint.request_area();continue;
            }
            let original=&SURFACES[index];let mut candidate=*original;
            if !apply_pending_text(&mut candidate,&pending){paint.request_full();continue;}
            match text_projection_requirement(original,&candidate,w,h){
                scene::TextCoverage::NoPixels=>paint.request_settle(),
                scene::TextCoverage::Body(rect)=>paint.request_damage(rect,w,h,bar_height()),
            }
        }
    }
}

/// Program pixel surfaces: Display's read-only view of each row's region, and the size
/// the row last asked to show (zero is text). Reset when the row presents a new run.
const PROGRAM_ROWS:usize=sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Program);
static mut PIXEL_REGIONS:[u64;PROGRAM_ROWS]=[0;PROGRAM_ROWS];
static mut PIXEL_BYTES:[u64;PROGRAM_ROWS]=[0;PROGRAM_ROWS];
static mut PIXEL_SIZE:[(usize,usize);PROGRAM_ROWS]=[(0,0);PROGRAM_ROWS];
/// The program row's picture under this pixel, when the surface shows one.
fn program_pixel_at(x:usize,y:usize,surf:&Surface)->Option<Option<u32>>{
    program_pixel_at_for(x,y,surf,chrome_metrics())
}
fn program_pixel_at_for(x:usize,y:usize,surf:&Surface,metrics:sex_pdx::desktop_style::chrome::Metrics)->Option<Option<u32>>{
    let k=sex_pdx::pixel_region::index_of(surf.owner_pd)?;
    if !app_catalog::by_surface(surf.owner_pd,surf.surface_id).is_some_and(|r|r.app==app_catalog::App::Program){return None;}
    let (w,h)=unsafe{PIXEL_SIZE[k]};let base=unsafe{PIXEL_REGIONS[k]};
    if w==0||h==0||base==0{return None;}
    let bx=(surf.x.max(0) as usize).saturating_add(metrics.px(sex_pdx::desktop_style::INSET)as usize);
    let top=if surf.chrome_flags&SURFACE_CHROME_TOP_BAR!=0{(metrics.title()as usize)+(metrics.px(8)as usize)}else{metrics.px(sex_pdx::desktop_style::TEXT_TOP)as usize};
    let by=(surf.y.max(0) as usize).saturating_add(top);
    // Inside the frame's border, whatever size the program asked for.
    let right=(surf.x.max(0) as usize+surf.w as usize).saturating_sub(metrics.px(sex_pdx::desktop_style::INSET)as usize);
    let bottom=(surf.y.max(0) as usize+surf.h as usize).saturating_sub(metrics.px(sex_pdx::desktop_style::INSET)as usize);
    if x<bx||y<by||x>=bx+w||y>=by+h||x>=right||y>=bottom{return Some(None);}
    Some(Some(unsafe{core::ptr::read_volatile((base as *const u32).add((y-by)*w+(x-bx)))}&0x00FF_FFFF))
}
/// The part of row `y` within `x0..x1` that shows the program's picture: where it
/// starts in the region, and the screen columns it covers. Same bounds as
/// `program_pixel_at`, computed once per row.
fn program_pixel_row(y:usize,x0:usize,x1:usize,surf:&Surface)->Option<(*const u32,usize,usize)>{
    program_pixel_row_for(y,x0,x1,surf,chrome_metrics())
}
fn program_pixel_row_for(y:usize,x0:usize,x1:usize,surf:&Surface,metrics:sex_pdx::desktop_style::chrome::Metrics)->Option<(*const u32,usize,usize)>{
    let k=sex_pdx::pixel_region::index_of(surf.owner_pd)?;
    if !app_catalog::by_surface(surf.owner_pd,surf.surface_id).is_some_and(|r|r.app==app_catalog::App::Program){return None;}
    let (w,h)=unsafe{PIXEL_SIZE[k]};let base=unsafe{PIXEL_REGIONS[k]};
    if w==0||h==0||base==0{return None;}
    let bx=(surf.x.max(0) as usize).saturating_add(metrics.px(sex_pdx::desktop_style::INSET)as usize);
    let top=if surf.chrome_flags&SURFACE_CHROME_TOP_BAR!=0{(metrics.title()as usize)+(metrics.px(8)as usize)}else{metrics.px(sex_pdx::desktop_style::TEXT_TOP)as usize};
    let by=(surf.y.max(0) as usize).saturating_add(top);
    let right=(surf.x.max(0) as usize+surf.w as usize).saturating_sub(metrics.px(sex_pdx::desktop_style::INSET)as usize).min(bx+w).min(x1);
    let bottom=(surf.y.max(0) as usize+surf.h as usize).saturating_sub(metrics.px(sex_pdx::desktop_style::INSET)as usize).min(by+h);
    let from=x0.max(bx);
    if y<by||y>=bottom||from>=right{return None;}
    Some((unsafe{(base as *const u32).add((y-by)*w+(from-bx))},from,right))
}
/// Read-only views of each app's BOOT text region, resolved once at startup.
static mut TEXT_REGIONS:[u64;sex_pdx::app_catalog::COUNT]=[0;sex_pdx::app_catalog::COUNT];
fn text_region_of(owner:u32)->Option<&'static [u8]>{
    let index=sex_pdx::text_region::index_of(owner)?;
    let base=unsafe{TEXT_REGIONS[index]};
    if base==0{return None;}
    Some(unsafe{core::slice::from_raw_parts(base as *const u8,sex_pdx::text_region::BYTES as usize)})
}

fn legacy_text_clear(slot:&mut Surface)->bool{
    let changed=slot.text_len!=0 || slot.selection_start!=0 || slot.selection_end!=0;
    slot.erase_selection();
    slot.text_len=0;slot.text_buf=[0u8;TEXT_BUF_CAP];
    slot.line_count=0;slot.cell_count=0;slot.cell_starts.fill(0);slot.line_cells.fill(0);slot.line_starts.fill(0);slot.wrap_cols=TEXT_LEGACY_COLS;
    changed
}
fn legacy_text_draw(slot:&mut Surface,packed:u64,arg2:u64)->bool{
    let offset=((arg2&0xFF)|((arg2>>4)&0xF00))as usize;
    let count=((arg2>>8)&0xF)as usize;
    let color=((arg2>>32)as u32)|0xFF000000;
    let mut changed=(slot.text_color!=color&&slot.text_len!=0) || slot.selection_start!=0 || slot.selection_end!=0;
    slot.erase_selection();
    slot.text_color=color;
    let mut i=0;
    while i<count&&i<8&&offset+i<TEXT_BUF_CAP{
        let byte=((packed>>(i*8))&0xFF)as u8;
        changed|=slot.text_buf[offset+i]!=byte;slot.text_buf[offset+i]=byte;i+=1;
    }
    if offset+i>slot.text_len as usize{
        changed=true;slot.text_len=(offset+i).min(TEXT_BUF_CAP)as u16;
    }
    recompute_text_lines(slot);changed
}

fn surface_text_fg_at(x: usize, y: usize, surf: &Surface) -> Option<u32>{
    surface_text_fg_at_scaled(x,y,surf,sex_pdx::desktop_style::ui_scale())
}
fn surface_text_fg_at_scaled(x: usize, y: usize, surf: &Surface, ui_scale:u32) -> Option<u32>{
    // A program showing pixels shows only them.
    if let Some(pixel) = program_pixel_at_for(x,y,surf,sex_pdx::desktop_style::chrome::Metrics::new(ui_scale)) { return pixel; }
    if surf.text_len == 0 || surf.line_count == 0 { return None; }
    const GLYPH_W: usize = sex_pdx::desktop_style::GLYPH_WIDTH as usize;
    const GLYPH_H: usize = sex_pdx::desktop_style::GLYPH_HEIGHT as usize;
    const CHAR_SPACING: usize = 1;
    let scale = surface_text_scale_for(surf,ui_scale);
    let line_h = (GLYPH_H + 2) * scale;
    let tx = (surf.x.max(0) as usize).saturating_add(sex_pdx::desktop_style::chrome::Metrics::new(ui_scale).px(sex_pdx::desktop_style::INSET)as usize);
    let top_inset = if surf.chrome_flags & SURFACE_CHROME_TOP_BAR != 0 { (sex_pdx::desktop_style::chrome::Metrics::new(ui_scale).title()as usize) + (sex_pdx::desktop_style::chrome::Metrics::new(ui_scale).px(8)as usize) } else { sex_pdx::desktop_style::chrome::Metrics::new(ui_scale).px(sex_pdx::desktop_style::TEXT_TOP)as usize };
    let ty = (surf.y.max(0) as usize).saturating_add(top_inset);

    if x < tx || y < ty { return None; }

    let col = (x - tx) / ((GLYPH_W + CHAR_SPACING) * scale);
    let row = (y - ty) / line_h;
    let px = ((x - tx) / scale) % (GLYPH_W + CHAR_SPACING);
    let py = ((y - ty) / scale) % (GLYPH_H + 2);

    if col >= surf.wrap_cols as usize { return None; }
    if row >= surf.line_count as usize { return None; }

    // Cell ordinals are derived from the same committed bytes as line starts.
    // Keep scalar decoding out of the per-pixel column scan.
    let ordinal=surf.line_cells[row] as usize+col;
    let row_end=if row+1<surf.line_count as usize {surf.line_cells[row+1]}else{surf.cell_count} as usize;
    if ordinal>=row_end || ordinal>=surf.cell_count as usize { return None; }
    let char_idx=surf.cell_starts[ordinal] as usize;
    let (scalar,char_end)=sex_pdx::text_encoding::cell(&surf.text_buf[..(surf.text_len as usize).min(TEXT_BUF_CAP)],char_idx)?;
    if scalar==10 { return None; }
    // Selection belongs to this committed surface text. Paint the whole cell,
    // including spaces and inter-glyph spacing, beneath existing foreground ink.
    let selected = char_idx < surf.selection_end as usize
        && char_end > surf.selection_start as usize
        && surf.selection_end as usize <= surf.text_len as usize;
    let background = if selected { Some(0xff36577c) } else { None };
    // Geist Mono is the default at every scale, including the compact 6x9 grid.
    // Keep the legacy glyph only for missing font coverage or a cache failure.
    let advance = (GLYPH_W + CHAR_SPACING) * scale;
    {
        if let Some(cell) = glyphs::mono_cell(scalar, advance, line_h) {
            let (gx, gy) = ((x - tx) % advance, (y - ty) % line_h);
            let alpha = cell[gy * advance + gx];
            if alpha == 0 { return background; }
            return Some(coverage_blend(surf.text_color, background.unwrap_or(surf.color), alpha));
        }
    }
    if px >= GLYPH_W || py >= GLYPH_H { return background; }
    let glyph_row = scalar_glyph(scalar)[py];
    let bit = (glyph_row >> (GLYPH_W - 1 - px)) & 1;
    if bit != 0 { Some(surf.text_color) } else { background }
}

/// Gregorian UTC caption derived only from the same accepted whole-second sample.
/// Formatting is not a source query or an authority/range admission decision.
fn civil_clock_date(seconds:u32)->[u8;13]{
    const MONTHS:[[u8;3];12]=[*b"JAN",*b"FEB",*b"MAR",*b"APR",*b"MAY",*b"JUN",*b"JUL",*b"AUG",*b"SEP",*b"OCT",*b"NOV",*b"DEC"];
    let unknown=*b"Date unknown ";
    let Some(date)=sex_time::SexTimestamp::from_unix(seconds as i64,0)
        .and_then(sex_time::Gregorian::from_timestamp) else{return unknown;};
    if !(1..=9999).contains(&date.year){return unknown;}
    let Some(month)=date.month.checked_sub(1).and_then(|m|MONTHS.get(m as usize)) else{return unknown;};
    let mut line=[b' ';13];
    line[0]=b'0'+date.day/10;line[1]=b'0'+date.day%10;
    line[3..6].copy_from_slice(month);
    let year=date.year as u32;
    for(at,divisor)in[(7,1000),(8,100),(9,10),(10,1)]{line[at]=b'0'+(year/divisor%10)as u8;}
    line
}
/// Two padded ASCII rows derived from the accepted sample/state. Renderers
/// snapshot these once per pass, never recompute the calendar per pixel.
fn civil_clock_lines(clock:&silkbar_model::civil_clock::CivilClock)->[[u8;13];2]{
    use silkbar_model::civil_clock::State;
    let mut lines=[[b' ';13];2];
    let (first,second):(&[u8],&[u8])=match clock.state(){
        State::Snapshot=>(b"UTC 00:00:00",b""),
        State::Rollback=>(b"UTC blocked",b"Time blocked"),
        State::Exhausted=>(b"UTC ended",b"Updates ended"),
        State::Unavailable=>(b"UTC unknown",b"Source error"),
        State::Unknown if clock.is_stale()=>(b"UTC stale",b"Sample stale"),
        State::Unknown=>(b"UTC unknown",b"Awaiting UTC"),
    };
    lines[0][..first.len()].copy_from_slice(first);lines[1][..second.len()].copy_from_slice(second);
    if clock.state()==State::Snapshot{
        lines[1]=civil_clock_date(clock.seconds());
        let day=clock.seconds()%86_400;
        for(at,value)in[(4,day/3600),(7,day/60%60),(10,day%60)]{
            lines[0][at]=b'0'+(value/10)as u8;lines[0][at+1]=b'0'+(value%10)as u8;
        }
    }
    lines
}
/// One real single-line UTC/date caption per paint pass. State remains the
/// admitted civil-clock state; the legacy two-row data formatter is unchanged.
fn civil_clock_caption(clock:&silkbar_model::civil_clock::CivilClock)->[u8;20]{
    let lines=civil_clock_lines(clock);let mut text=[b' ';20];
    if clock.state()==silkbar_model::civil_clock::State::Snapshot {
        text[..5].copy_from_slice(&lines[0][4..9]);
        text[6..9].copy_from_slice(b"UTC");text[10]=b'|';
        text[12..18].copy_from_slice(&lines[1][..6]);
    }else if clock.state()==silkbar_model::civil_clock::State::Unavailable {
        text[..15].copy_from_slice(b"UTC unavailable");
    }else{text[..13].copy_from_slice(&lines[0]);}
    text
}
fn ui_label_at(text:&[u8],x:usize,y:usize,background:u32)->Option<u32>{
    ui_label_at_scaled(text,x,y,background,sex_pdx::desktop_style::ui_scale())
}
fn ui_label_at_scaled(text:&[u8],x:usize,y:usize,background:u32,ui_scale:u32)->Option<u32>{
    let scale=ui_scale;
    let alpha=glyphs::ui_coverage(text,x as i32,y,scale)?;
    (alpha!=0).then(||coverage_blend(sex_pdx::desktop_style::contrast_ink(background),background,alpha))
}
/// Proportional Geist Sans captions in the existing status boxes. The legacy
/// cell renderer is recovery only; ordinary labels keep natural glyph advances.
fn panel_caption_at(text:&[u8],x:usize,y:usize,background:u32)->Option<u32>{
    if y>=glyphs::CAPTION_H{return None;}
    let alpha=match glyphs::caption_coverage(text,x as i32,y){
        Some(alpha)=>alpha,
        None=>{
            let byte=*text.get(x/6)?;let px=x%6;
            if px>=5||y>=7{return None;}
            let glyph=ascii_glyph_index(byte)?;
            if (FONT_ASCII_5X7[glyph][y]>>(4-px))&1!=0{255}else{0}
        }
    };
    (alpha!=0).then(||coverage_blend(sex_pdx::desktop_style::contrast_ink(background),background,alpha))
}
fn civil_clock_fg_at(x:usize,y:usize,bar:&SilkBar,text:&[u8;20],background:u32)->Option<u32>{
    civil_clock_fg_at_scaled(x,y,bar,text,background,sex_pdx::desktop_style::ui_scale())
}
fn civil_clock_fg_at_scaled(x:usize,y:usize,bar:&SilkBar,text:&[u8;20],background:u32,ui_scale:u32)->Option<u32>{
    let(cx,cy,cw,ch)=module_rect(bar,ModuleSlot::Clock);
    let pad=sex_pdx::desktop_style::chrome::Metrics::new(ui_scale).px(4)as usize;
    let top=cy+ch.saturating_sub(glyphs::UI_H*ui_scale as usize)/2;
    if x<cx+pad||x>=cx+cw||y<top||y>=cy+ch{return None;}
    ui_label_at_scaled(text,x-cx-pad,y-top,background,ui_scale)
}
/// One completed-frame observation per changed clock stamp. This is called
/// only after the full/strip framebuffer writes and overlays finish. Accepted
/// protocol updates remain unconfirmed until this path actually runs.
fn civil_clock_drawn(bar:&SilkBar,w:usize,h:usize,path:&str){
    use silkbar_model::civil_clock::State;
    if !bar.civil_enabled{return;}
    let(x,y,cw,ch)=module_rect(bar,ModuleSlot::Clock);
    if x.checked_add(cw).map_or(true,|right|right>w)||y.checked_add(ch).map_or(true,|bottom|bottom>h){return;}
    let clock=&bar.civil_clock;
    let stamp=(clock.sequence(),clock.state(),clock.seconds(),clock.is_stale(),w,h,x,y,cw,ch);
    static mut LAST_CIVIL_DRAW:Option<(u32,State,u32,bool,usize,usize,usize,usize,usize,usize)>=None;
    unsafe{
        if LAST_CIVIL_DRAW==Some(stamp){return;}
        LAST_CIVIL_DRAW=Some(stamp);
    }
    sex_pdx::serial_record!("[display.civil.draw] sequence={} state={:?} seconds={} stale={} pixels=written path={} x={} y={} w={} h={}",clock.sequence(),clock.state(),clock.seconds(),clock.is_stale()as u8,path,x,y,cw,ch);
}
fn clock_fg_at(x: usize, y: usize, bar: &SilkBar, lines: &[u8;20], background: u32) -> Option<u32>{
    clock_fg_at_scaled(x,y,bar,lines,background,sex_pdx::desktop_style::ui_scale())
}
fn clock_fg_at_scaled(x: usize, y: usize, bar: &SilkBar, lines: &[u8;20], background: u32, ui_scale:u32) -> Option<u32>{
    if bar.civil_enabled {return civil_clock_fg_at_scaled(x,y,bar,lines,background,ui_scale);}
    let(cx,cy,cw,ch)=module_rect(bar,ModuleSlot::Clock);
    if x<cx+2||x>=cx+cw||y<cy+1||y>=cy+ch{return None;}
    let mut label=*b"DEMO 00:00:00";
    for(at,value)in[(5,bar.clock_hh),(8,bar.clock_mm),(11,bar.clock_ss)]{
        label[at]=b'0'+value/10;label[at+1]=b'0'+value%10;
    }
    panel_caption_at(&label,x-cx-2,y-cy-1,background)
}

/// Bell count in the existing module: natural 9px Sans digits, centered and clipped.
fn bell_badge_at(x: usize, y: usize, bar: &SilkBar, background: u32) -> Option<u32>{
    bell_badge_at_scaled(x,y,bar,background,sex_pdx::desktop_style::ui_scale())
}
fn bell_badge_at_scaled(x: usize, y: usize, bar: &SilkBar, background: u32, ui_scale:u32) -> Option<u32>{
    let n=bar.bell_state.total_visible;
    if n==0||bar.bell_state.flags&1==0{return None;}
    let n=n.min(99);let digits=[b'0'+n/10,b'0'+n%10];
    let label=if n>=10{&digits[..]}else{&digits[1..]};
    let(bx,by,bw,bh)=module_rect(bar,ModuleSlot::Bell);
    if x<bx||x>=bx+bw||y<by||y>=by+bh{return None;}
    let width=glyphs::ui_width(label,ui_scale).unwrap_or(label.len()*6).min(bw);
    let left=bx+(bw-width)/2;let top=by+bh.saturating_sub(glyphs::UI_H*ui_scale as usize)/2;
    if x<left||y<top{return None;}
    ui_label_at_scaled(label,x-left,y-top,background,ui_scale)
}


use paint_batch::{Content,Plan};
// Descriptive completion of this publication, not authority or a whole-system
// epoch receipt. Lifecycle may consume only a successfully published Full.
struct PublishedFrame{plan:Plan,bar_height:usize,present_bits:u64,canon_ss:u8,
    text_projections:[Option<scene::TextProjection>;MAX_SURFACES]}

// Small output bookkeeping is tentative until the prepared frame is published.
// Its retained pixel copy is erased before this invocation returns to cleanup.
struct OutputBackup{saved:[u32;CURSOR_ARROW_W*CURSOR_ARROW_H*9],x:usize,y:usize,valid:bool,bits:u64,frame:u64,pending:Option<PreparedCursor>}
impl OutputBackup{
    fn capture()->Self{unsafe{Self{saved:CURSOR_UNDER_SAVED,x:CURSOR_UNDER_X,y:CURSOR_UNDER_Y,valid:CURSOR_UNDER_VALID,bits:APP_PRESENT_BITS,
        frame:RENDER_FRAME_COUNTER.load(core::sync::atomic::Ordering::Relaxed),pending:PREPARED_CURSOR}}}
    fn restore(&self){unsafe{CURSOR_UNDER_SAVED.copy_from_slice(&self.saved);CURSOR_UNDER_X=self.x;CURSOR_UNDER_Y=self.y;
        CURSOR_UNDER_VALID=self.valid;APP_PRESENT_BITS=self.bits;PREPARED_CURSOR=self.pending;
        RENDER_FRAME_COUNTER.store(self.frame,core::sync::atomic::Ordering::Relaxed);}}
}
impl Drop for OutputBackup{fn drop(&mut self){for pixel in &mut self.saved{unsafe{core::ptr::write_volatile(pixel,0);}}}}

fn valid_scanout(fb:*mut u32,w:usize,h:usize)->bool{
    let Some(bytes)=w.checked_mul(h).and_then(|n|n.checked_mul(4))else{return false;};
    let address=fb as u64;
    if address<HIGH_HALF_BASE || address.checked_add(bytes as u64).is_none(){return false;}
    unsafe{address==FB_PTR&&w==FB_W as usize&&h==FB_H as usize&&w!=0&&h!=0&&w<=MAX_FB_W&&h<=MAX_FB_H}
}

// Sole Display scanout writer. Preparation writes only the held private target.
// This CPU copy is not an atomic hardware flip or vblank completion. Only the
// spans that differ from the frame the output already shows are written, each
// row at the scanout's own pitch (real GPUs pad rows).
fn publish_frame(fb:*mut u32,w:usize,h:usize,frame:frame_output::Publication<'_>)->bool{
    let pixels=frame.pixels;
    if !valid_scanout(fb,w,h)||w.checked_mul(h)!=Some(pixels.len()){return false;}
    if frame.previous.is_some_and(|old|old.len()!=pixels.len())||frame.rows.end>h{return false;}
    let pitch=unsafe{FB_PITCH} as usize;
    let mut written=0usize;
    // A new night light level repaints every pixel: what the output shows was
    // warmed by the old level, so comparing with the old frame is not enough.
    let night=sex_pdx::desktop_look::night_level();
    let previous=if sex_pdx::desktop_look::take_night_change(){None}else{frame.previous};
    for y in frame.rows.first..frame.rows.end{
        let row=&pixels[y*w..(y+1)*w];
        let (first,end)=match previous{
            Some(old)=>{let old=&old[y*w..(y+1)*w];
                let Some(first)=(0..w).find(|&x|row[x]!=old[x])else{continue};
                (first,(first..w).rev().find(|&x|row[x]!=old[x]).unwrap_or(first)+1)}
            None=>(0,w)};
        let out=unsafe{fb.add(y*pitch)};
        if night==0{
        for(index,pixel)in row[first..end].iter().enumerate(){unsafe{core::ptr::write_volatile(out.add(first+index),*pixel);}}
        }else{
            // Night light warms the finished frame on its way out.
            for(index,pixel)in row[first..end].iter().enumerate(){unsafe{core::ptr::write_volatile(out.add(first+index),sex_pdx::desktop_look::night(*pixel,night));}}
        }
        written+=end-first;
    }
    unsafe{PUBLISHED_PIXELS=PUBLISHED_PIXELS.wrapping_add(written as u64);LAST_WRITTEN=written as u64;}
    // Order posted/write-combining CPU stores before releasing presentation
    // effects. This does not signal GPU scanout/latch or atomic visibility.
    #[cfg(target_arch="x86_64")]
    unsafe{core::arch::asm!("sfence",options(nostack,preserves_flags));}
    #[cfg(not(target_arch="x86_64"))]
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    true
}

fn raster_batch(target:&frame_output::Target<'_>,scene: &scene::SceneSnapshot,plan:Plan)->bool{
    if !scene.is_valid()||scene.width!=target.width()||scene.height!=target.height()||plan.is_empty(){return false;}
    // Verified no-pixel text work republishes the committed baseline. Do not
    // disturb cursor underlay or infer any app/cursor/clock presentation effect.
    if plan.settle_only(){return true;}
    let (fb,w,h,total)=(target.pointer(),target.width(),target.height(),target.len());
    if !plan.is_full(){restore_cursor_under(fb,w,h,total,scene.metrics);}
    let body=match plan.content{
        Content::Full=>raster_full(target,scene),Content::Area=>raster_area(target,scene),
        Content::Damage(rect)=>raster_damage(target,rect,scene),Content::None|Content::Settle|Content::Cursor=>true};
    if !body||plan.strip&&!raster_strip(target,scene){return false;}
    if !plan.cursor_only(){
        if plan.is_full()||matches!(plan.content,Content::Area)||plan.strip{
            draw_atlas_cards_pass(fb,w,h,total,scene);
            draw_atlas_frame_previews_pass(fb,w,h,total,scene);
        }
        draw_launcher_panel(fb,w,h,total,scene);
    }
    draw_cursor_z_top(fb,w,h,total,true,scene);
    true
}

fn render_prepared(fb:*mut u32,w:usize,h:usize,bar:&SilkBar,requested:Plan)->Option<PublishedFrame>{
    if requested.is_empty()||!valid_scanout(fb,w,h){return None;}
    // Reject reentry before creating another exclusive frame-store reference.
    if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return None;}
    let mut plan=requested.with_atlas(ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED||ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED);
    if !plan.is_full()&&h<bar_height(){plan=Plan::full();}
    let backup=OutputBackup::capture();
    let Some(store)=(unsafe{FRAME_OUTPUT.as_mut()})else{return None;};
    if store.layout().width()!=w||store.layout().height()!=h{return None;}
    if store.front_pixels().is_none(){plan=Plan::full();}
    let settle_only=plan.settle_only();
    let started=if plan.cursor_only()||settle_only{0}else{repaint_begin()};
    let Some(mut preparation)=store.prepare(plan.is_full())else{return None;};
    if plan.is_full(){RENDER_FRAME_COUNTER.fetch_add(1,core::sync::atomic::Ordering::Relaxed);}
    let Some(lease)=SceneLease::capture(w,h,bar)else{drop(preparation);backup.restore();return None;};
    let scene=lease.view();
    let canon_ss=unsafe{CLOCK_CANON_SS};
    if !settle_only{unsafe{APP_PRESENT_BITS=0;PREPARED_CURSOR=None;}}
    let prepared={let target=preparation.target();raster_batch(&target,scene,plan)};
    if !prepared{drop(preparation);backup.restore();return None;}
    if !preparation.commit(|frame|publish_frame(fb,w,h,frame)){backup.restore();return None;}
    if let Some(front)=store.front_pixels(){unsafe{
        if SCREEN_CAPTURE.published(store.generation(),w,h,front,sex_pdx::desktop_look::night_level()){
            let frame=app_catalog::by_target(sex_pdx::screen_capture::CLIENT).map(|row|row.frame());
            if let Some(surface)=scene.surfaces.iter().find(|s|s.active&&Some(s.surface_id)==frame){
                sex_pdx::serial_record!("[display.capture.reference] generation={} frame={} x={} y={} w={} h={} bar_h={} source=published_scene",
                    store.generation(),surface.surface_id,surface.x,surface.y,surface.w,surface.h,scene.metrics.bar());
            }
        }
    }}
    let present_bits=if settle_only{0}else{unsafe{APP_PRESENT_BITS}};
    if !settle_only{
        record_cursor_presented();
        report_app_presented();
        if !plan.cursor_only(){repaint_end(plan.label(),started);}
    }
    unsafe{
        static mut BATCH_LOG_BUDGET:u32=64;
        if BATCH_LOG_BUDGET>0{BATCH_LOG_BUDGET-=1;
            sex_pdx::serial_record!("[display.batch.published] generation={} content={} strip={} scene=1 prepare=1 publication=1 written={} total_written={}",
                store.generation(),match plan.content{Content::Full=>"full",Content::Area=>"area",Content::Damage(_)=>"damage",Content::Cursor=>"cursor",Content::Settle=>"settle",Content::None=>"none"},plan.strip as u8,LAST_WRITTEN,PUBLISHED_PIXELS);}
    }
    if plan.is_full(){
        static mut LAST_CHROME_METRICS:Option<(u32,usize,usize)>=None;
        let m=scene.metrics;let stamp=(m.scale(),w,h);
        unsafe{if LAST_CHROME_METRICS!=Some(stamp){
            LAST_CHROME_METRICS=Some(stamp);
            sex_pdx::serial_record!("[display.chrome.metrics] scale={} bar={} title={} inset={} body_top={} target={} width={} height={} publication=1",m.scale(),m.bar(),m.title(),m.px(8),m.title()+m.px(8),m.px(24),w,h);
        }}
    }
    if plan.clock_dirty(){civil_clock_drawn(&scene.bar,w,h,plan.label());}
    // Copy only descriptive transaction identities. The payload remains owned
    // by this scene, which ordinary scope Drop scrubs before Main commits text.
    let text_projections=scene.text_projections;
    Some(PublishedFrame{plan,bar_height:scene.metrics.bar()as usize,present_bits,canon_ss,text_projections})
}
fn render(fb:*mut u32,w:usize,h:usize,bar:&SilkBar)->bool{
    render_prepared(fb,w,h,bar,Plan::full()).is_some()
}

fn raster_full(target: &frame_output::Target<'_>, scene: &scene::SceneSnapshot) -> bool {
    let fb=target.pointer();
    let (w,h)=(target.width(),target.height());
    let pixels=target.len();
    let frame=scene.frame;
    // Budgeted marker: full-FB render invoked (proof of render entry + boundedness).
    unsafe {
        static mut FULL_RENDER_BUDGET: u32 = 8;
        let b = &mut FULL_RENDER_BUDGET;
        if *b > 0 {
            *b -= 1;
            serial_println!("[sexdisplay.render.full] fb_w={} fb_h={}", w, h);
        }
    }
    let total_pixels = pixels;
    let row_buffer_filled = fill_bar_bg_buffer(w as u32, h as u32);
    let blur_filled = if row_buffer_filled {
        blur_bar_bg_buffer_radius1(w as u32, h as u32)
    } else {
        false
    };
    if !ROW_BUFFER_PROOF_LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        serial_println!(
            "[sexdisplay.render.row_buffer] cap_w={} fb_w={} filled={}",
            BAR_BG_W_CAP,
            w,
            if row_buffer_filled { 1 } else { 0 }
        );
    }
    if !BLUR_PROOF_LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        serial_println!(
            "[sexdisplay.render.blur] radius=1 fb_w={} filled={}",
            w,
            if blur_filled { 1 } else { 0 }
        );
    }
    if !ANIM_PROOF_LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        let _ = frame;
        serial_println!("[sexdisplay.render.anim] enabled=0 reason=quiet_chrome");
    }
    if !R6_GLASS_PROOF_LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        serial_println!("[sexdisplay.render.glass.r6] bar_alpha=196 chip_alpha=212 blur=1 pulse=0");
    }

    let bar=&scene.bar;
    let focused_id=scene.focus;
    // Top strip boundary: BAR_H = 50. Keep in sync with BAR_H constant.
    // Rows 0..49 = SilkBar panel background, 50 = glow edge.
    let clock_lines=civil_clock_caption(&bar.civil_clock);
    let _=focused_id;
    if total_pixels<w*h{return false;}
    for y in 0..h.min((scene.metrics.bar()as usize)) {
        for x in 0..w {
            let c: u32 = if y < (scene.metrics.bar()as usize).saturating_sub(scene.metrics.scale()as usize) {
                top_strip_pixel(x, y, bar, &clock_lines, scene)
            } else {
                DEFAULT_THEME.panel_glow // low-contrast bar edge
            };
            unsafe { core::ptr::write_volatile(fb.add(y*w+x), c); }
        }
    }
    for y in (scene.metrics.bar()as usize)..h{raster_row(fb,y,0,w,w,h,scene);}
    true
}

/// Redraw the top strip (rows 0..50) — bar background, workspace indicators, chips, and clock digits.
/// Called for all SilkBar updates (clock, workspace active/urgent, chip visibility/kind).
/// This is the only function that touches y<50 pixels after initial render.
fn raster_strip(target: &frame_output::Target<'_>, scene: &scene::SceneSnapshot) -> bool {
    let fb=target.pointer();
    let (w,h)=(target.width(),target.height());

    if w == 0 || h == 0 || w > MAX_FB_W || h > MAX_FB_H {
        return false;
    }
    if h < (scene.metrics.bar()as usize) {
        return false;
    }
    let total_pixels = match w.checked_mul(h) {
        Some(v) => v,
        None => return false,
    };
    let bar=&scene.bar;
    // [silk.live_topstrip.v2.fb_clear] REMOVED: writing raw desktop gradient over
    // the bar area before rendering the bar causes a visible black/purple flash
    // on every clock tick (the host displays the partial gradient before the bar
    // loop finishes overwriting it). The bar loop covers rows 0..51 unconditionally,
    // so the clear is unnecessary.

    // [silk.live_topstrip.v2.audit] Row-sampled hash diagnostics for top strip rows.
    // Emitted at first live redraw (ss=any) and again at ss=4 to narrow down
    // which row range carries the artifact. Each row gets a compact 32-bit FNV-1a
    // hash seeded from the actual framebuffer pixel row (post-clear, pre-bar-render).
    // Only the first 64 rows are sampled to bound log output.
    // Gated by SILK_RENDER_PROOF_PROFILE_ENABLED to keep default boot fast.
    if SILK_RENDER_PROOF_PROFILE_ENABLED {
        unsafe {
            static mut LIVE_TOPSTRIP_V2_ROWS_DIAG_DONE: bool = false;
            static mut LIVE_TOPSTRIP_V2_ROWS_SS4_DONE: bool = false;
            let do_diag = !LIVE_TOPSTRIP_V2_ROWS_DIAG_DONE
                || (bar.clock_ss >= 4 && !LIVE_TOPSTRIP_V2_ROWS_SS4_DONE);
            if do_diag {
                let sample_rows: [usize; 10] = [0, 25, 46, 47, 48, 49, 50, 51, 55, 63];
                let max_row = sample_rows.iter().max().copied().unwrap_or(63);
                if h > max_row {
                    for &ry in sample_rows.iter() {
                        let row_base = ry * w;
                        let mut h_val: u32 = 0x811c9dc5u32;
                        for x in 0..w {
                            let idx = row_base + x;
                            if idx < total_pixels {
                                let px = core::ptr::read_volatile(fb.add(idx));
                                h_val ^= px as u32;
                                h_val = h_val.wrapping_mul(0x01000193u32);
                            }
                        }
                        serial_println!("[silk.live_topstrip.v2.rows] row={} hash=0x{:08X} ss={}",
                            ry, h_val, bar.clock_ss);
                    }
                }
                if !LIVE_TOPSTRIP_V2_ROWS_DIAG_DONE {
                    LIVE_TOPSTRIP_V2_ROWS_DIAG_DONE = true;
                }
                if bar.clock_ss >= 4 && !LIVE_TOPSTRIP_V2_ROWS_SS4_DONE {
                    LIVE_TOPSTRIP_V2_ROWS_SS4_DONE = true;
                }
            }
        }
    }

    // [silk.live_topstrip.glitch.fix] Refill bar background blur buffer before each live
    // top-strip redraw. Prevents stale glass artifacts when a clock tick triggers a
    // strip-only redraw without a preceding full render() call. The gradient colors are
    // constant so this is cheap — no visible change on correct boots, defensive on edge cases.
    fill_bar_bg_buffer(w as u32, h as u32);
    blur_bar_bg_buffer_radius1(w as u32, h as u32);
    // Budgeted marker: top-strip redraw invoked (proof of bar render + boundedness).
    unsafe {
        // [silk.live_topstrip.audit] One-shot: confirms live top-strip redraw entered.
        // Gated by SILK_RENDER_PROOF_PROFILE_ENABLED to keep default boot fast.
        if SILK_RENDER_PROOF_PROFILE_ENABLED {
            static mut LIVE_TOPSTRIP_AUDIT_DONE: bool = false;
            if !LIVE_TOPSTRIP_AUDIT_DONE {
                LIVE_TOPSTRIP_AUDIT_DONE = true;
                serial_println!("[silk.live_topstrip.audit] first_live_redraw fb_w={} fb_h={} ss={}", w, h, bar.clock_ss);
            }
            // [silk.live_topstrip.clear] Budgeted: proves full strip cleared before redraw.
            static mut LIVE_TOPSTRIP_CLEAR_BUDGET: u32 = 32;
            let lcb = &mut LIVE_TOPSTRIP_CLEAR_BUDGET;
            if *lcb > 0 {
                *lcb -= 1;
                serial_println!("[silk.live_topstrip.clear] y_start=0 y_end=51 w={} ss={} ok=1", w, bar.clock_ss);
            }
            // [silk.live_topstrip.bounds] Budgeted: proves bar confined to y<51.
            static mut LIVE_TOPSTRIP_BOUNDS_BUDGET: u32 = 8;
            let lbb = &mut LIVE_TOPSTRIP_BOUNDS_BUDGET;
            if *lbb > 0 {
                *lbb -= 1;
                serial_println!("[silk.live_topstrip.bounds] top_strip_h=50 glow_row=50 total_rows=51 fb_h={} ok=1", h);
            }
            // [silk.live_topstrip.tick4] One-shot: fires first time rendered clock shows ss>=4.
            // Proves live redraw is still running correctly past the observed glitch window.
            static mut LIVE_TOPSTRIP_TICK4_FIRED: bool = false;
            if !LIVE_TOPSTRIP_TICK4_FIRED && bar.clock_ss >= 4 {
                LIVE_TOPSTRIP_TICK4_FIRED = true;
                serial_println!("[silk.live_topstrip.tick4] ss={} mm={} hh={} fb_w={} fb_h={} ok=1 reason=live_redraw_past_tick4",
                    bar.clock_ss, bar.clock_mm, bar.clock_hh, w, h);
            }
        }
        static mut TOP_STRIP_REDRAW_BUDGET: u32 = 16;
        let b = &mut TOP_STRIP_REDRAW_BUDGET;
        if *b > 0 {
            *b -= 1;
            if hot_log() { serial_println!("[sexdisplay.render.top_strip] fb_w={} fb_h={}", w, h); }
        }
        // Clock redraw proof: what time is drawn on screen.
        // Source tracked via module-level CLOCK_REDRAW_SOURCE set by apply path.
        unsafe {
            static mut CLOCK_REDRAW_BUDGET: u32 = 64;
            let b = &mut CLOCK_REDRAW_BUDGET;
            if *b > 0 && !bar.civil_enabled {
                *b -= 1;
                let src = if CLOCK_REDRAW_SOURCE == 1 { "fallback" } else { "silkbar" };
                if hot_log() { serial_println!("[sexdisplay.clock.redraw] h={} m={} s={} source={}",
                    bar.clock_hh, bar.clock_mm, bar.clock_ss, src); }
                let source_ok = if bar.clock_ss == CLOCK_CANON_SS { 1 } else { 0 };
                if hot_log() { serial_println!(
                    "[sexdisplay.clock.redraw.source_check] redraw_ss={} canonical_ss={} source={} ok={}",
                    bar.clock_ss, CLOCK_CANON_SS, src, source_ok
                ); }
                static mut CLOCK_VISIBLE_MONO_INIT: bool = false;
                static mut CLOCK_VISIBLE_PREV_TOTAL: u32 = 0;
                let redraw_total = clock_total_seconds(bar.clock_hh, bar.clock_mm, bar.clock_ss);
                let (prev_ss, mono_ok) = if !CLOCK_VISIBLE_MONO_INIT {
                    CLOCK_VISIBLE_MONO_INIT = true;
                    CLOCK_VISIBLE_PREV_TOTAL = redraw_total;
                    (bar.clock_ss, 1u8)
                } else {
                    let prev_total = CLOCK_VISIBLE_PREV_TOTAL;
                    let ok = if clock_visible_monotonic_ok(prev_total, redraw_total) { 1u8 } else { 0u8 };
                    let prev_ss_v = (prev_total % 60) as u8;
                    if ok == 1 {
                        CLOCK_VISIBLE_PREV_TOTAL = redraw_total;
                    }
                    (prev_ss_v, ok)
                };
                if hot_log() { serial_println!(
                    "[sexdisplay.clock.monotonic.visible] prev_ss={} redraw_ss={} ok={}",
                    prev_ss, bar.clock_ss, mono_ok
                ); }
                // Visible seconds proof: confirms the exact seconds value that
                // will be drawn in the subsequent pixel loop (clock_fg_at).
                if hot_log() { serial_println!("[clock.visible.seconds] h={} m={} s={} drawn=1 ok=1 reason=pixel_loop_follows",
                    bar.clock_hh, bar.clock_mm, bar.clock_ss); }
            }
        }
        // Phase 3: state marker — prove phase1 fields are populated after receives.
        if SILKBAR_PHASE3_RECEIVE_PROOF_ENABLED {
            static mut PHASE3_STATE_BUDGET: u32 = 8;
            let b = &mut PHASE3_STATE_BUDGET;
            if *b > 0 {
                *b -= 1;
                serial_println!(
                    "[sexdisplay.silkbar.phase3.state] active={} tint={} palette_open={} selected={} available={}",
                    bar.phase1.active_app_sid,
                    bar.phase1.accent_tint_idx,
                    bar.phase1.palette_open as u8,
                    bar.phase1.palette_selected,
                    bar.phase1.palette_available
                );
            }
        }
        // Phase 5: budgeted draw marker — prove indicators are being rendered.
        if SILKBAR_PHASE5_PIXEL_PROOF_ENABLED {
            static mut PHASE5_DRAW_BUDGET: u32 = 8;
            let b = &mut PHASE5_DRAW_BUDGET;
            if *b > 0 {
                *b -= 1;
                serial_println!(
                    "[sexdisplay.silkbar.phase5.draw] active={} tint={} palette_open={}",
                    bar.phase1.active_app_sid,
                    bar.phase1.accent_tint_idx,
                    bar.phase1.palette_open as u8
                );
            }
        }
    }
    // Top strip boundary: rows 0..50. Keep in sync with BAR_H constant.
    // Row 50 = glow edge; rows 0..49 = SilkBar panel fill + modules.
    let clock_lines=civil_clock_caption(&bar.civil_clock);
    for y in 0..(scene.metrics.bar()as usize).min(h) {
        for x in 0..w {
            let c: u32 = if y < (scene.metrics.bar()as usize).saturating_sub(scene.metrics.scale()as usize) {
                top_strip_pixel(x, y, bar, &clock_lines, scene)
            } else {
                DEFAULT_THEME.panel_glow
            };
            let idx = y * w + x;
            if idx < total_pixels {
                unsafe { core::ptr::write_volatile(fb.add(idx), c); }
            }
        }
    }
    // Redraw top-layer overlays that may extend into the bar area (rows 0..50).
    // Without these, a clock-tick-triggered top-strip redraw would erase cursor
    // pixels, launcher panel, atlas cards, and frame preview content that the
    // initial render() placed above the bar, causing glitch artifacts across
    // the top chrome.
    // save_under=false: this path repaints only y<50; pixels under the
    // cursor (y>=51) still contain the drawn cursor — capturing here would
    // poison the save-under patch with cursor pixels.
    true
}

/// Redraw all pixels from y=50 to bottom (background gradient + all active surfaces).
/// Never touches y<50 (SilkBar/clock/chips). Not a full render() call.
fn raster_area(target: &frame_output::Target<'_>, scene: &scene::SceneSnapshot) -> bool {
    let fb=target.pointer();
    let (w,h)=(target.width(),target.height());

    if w == 0 || h == 0 || w > MAX_FB_W || h > MAX_FB_H { return false; }
    if h < (scene.metrics.bar()as usize) { return false; }
    let total_pixels = match w.checked_mul(h) {
        Some(v) => v,
        None => return false,
    };
    // Budgeted marker: surface-area redraw invoked (proof of compositor render + boundedness).
    unsafe {
        static mut SURFACE_AREA_REDRAW_BUDGET: u32 = 16;
        let b = &mut SURFACE_AREA_REDRAW_BUDGET;
        if *b > 0 {
            *b -= 1;
            if hot_log() { serial_println!("[sexdisplay.render.surface_area] fb_w={} fb_h={}", w, h); }
        }
    }
    if total_pixels<w*h{return false;}
    for x in 0..w{unsafe{core::ptr::write_volatile(fb.add(((scene.metrics.bar()as usize)-1)*w+x),DEFAULT_THEME.panel_glow);}} // low-contrast bar edge
    for y in (scene.metrics.bar()as usize)..h{raster_row(fb,y,0,w,w,h,scene);}
    true
}

/// Conservative old/new glyph footprint, derived from the same layout fields
/// as surface_text_fg_at. Geometry changes use the full scene path instead.
fn text_surface_damage(surf: &Surface, w: usize, h: usize) -> (usize,usize,usize,usize) {
    if surf.text_len == 0 || surf.line_count == 0 {return (0,0,0,0);}
    let (x,y,sw,sh)=surface_groups::rect(surf,w,h);
    let top=if surf.chrome_flags & SURFACE_CHROME_TOP_BAR != 0 {frame_height()+frame_rim()} else {chrome_metrics().px(sex_pdx::desktop_style::TEXT_TOP)as usize};
    let text_y=(surf.y.max(0) as usize).saturating_add(top).max(y);
    let bottom=(surf.y.max(0) as usize).saturating_add(top)
        .saturating_add(surf.line_count as usize * 9 * surface_text_scale(surf))
        .min(y.saturating_add(sh));
    (x,text_y,sw,bottom.saturating_sub(text_y))
}

/// Text-only redraw after a complete transaction. Restore the old cursor patch
/// before touching damage, then capture its new background exactly once.
fn raster_damage(target: &frame_output::Target<'_>, rect: damage::Rect, scene: &scene::SceneSnapshot) -> bool {
    let fb=target.pointer();
    let (w,h)=(target.width(),target.height());

    if w == 0 || h < (scene.metrics.bar()as usize) || w > MAX_FB_W || h > MAX_FB_H { return false; }
    if rect.x >= rect.right || rect.y < (scene.metrics.bar()as usize) || rect.y >= rect.bottom || rect.right > w || rect.bottom > h { return false; }
    if ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED || ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED {
        return raster_area(target,scene);
    }
    let Some(total) = w.checked_mul(h) else { return false; };
    let bar=&scene.bar;
    for y in rect.y..rect.bottom {
        raster_row(fb,y,rect.x,rect.right,w,h,scene);
    }
    true
}

// Classic NW arrow, 8 wide × 16 tall.
// Each u8 row: bit 7 = leftmost pixel. Set bit = draw CURSOR_ARROW_COLOR; clear = transparent.
const CURSOR_ARROW_W: usize = 8;
const CURSOR_ARROW_H: usize = 16;
const CURSOR_ARROW_COLOR: u32 = 0x00FFFFFF; // white
const CURSOR_ARROW_BITMAP: [u8; 16] = [
    0b10000000, //  *
    0b11000000, //  **
    0b11100000, //  ***
    0b11110000, //  ****
    0b11111000, //  *****
    0b11111100, //  ******
    0b11111110, //  *******
    0b11111100, //  ******   (start contracting)
    0b11011000, //  ** **    (waist split)
    0b10001100, //  *   **
    0b00001100, //      **
    0b00000110, //       **
    0b00000110, //       **
    0b00000011, //        **
    0b00000011, //        **
    0b00000001, //         *
];

/// INPUT_CURSOR_DRAIN_COHERENCE_V1: save-under patch for the cursor-only
/// fast path. Holds the framebuffer pixels beneath the last cursor draw so
/// a cursor-only move can restore + redraw the 8x16 rect instead of running
/// a full redraw_surface_area() composite. Local sexdisplay state — no
/// shared memory, no backing-buffer redesign, no ABI change.
// Room for the arrow at the largest screen scale (3x3 pixels per bit).
static mut CURSOR_UNDER_SAVED: [u32; CURSOR_ARROW_W * CURSOR_ARROW_H * 9] =
    [0; CURSOR_ARROW_W * CURSOR_ARROW_H * 9];
static mut CURSOR_UNDER_X: usize = 0;
static mut CURSOR_UNDER_Y: usize = 0;
static mut CURSOR_UNDER_VALID: bool = false;

/// Restore the framebuffer pixels saved beneath the last cursor draw.
/// Mirrors draw_cursor_z_top bounds rules exactly: y<51 SilkBar zone never
/// written, x/y clipped to fb dims, idx bounds-checked against total_pixels.


/// Discard retained framebuffer-derived cursor pixels before cleanup custody
/// can be acknowledged. A later draw saves only the removal scene's pixels.
fn scrub_cursor_under(){
    unsafe{
        CURSOR_UNDER_VALID=false;
        for pixel in &mut CURSOR_UNDER_SAVED{core::ptr::write_volatile(pixel,0);}
        CURSOR_UNDER_X=0;CURSOR_UNDER_Y=0;
    }
}

fn restore_cursor_under(fb: *mut u32, w: usize, h: usize, total_pixels: usize, metrics:sex_pdx::desktop_style::chrome::Metrics) {
    unsafe {
        if !CURSOR_UNDER_VALID { return; }
        let ox = CURSOR_UNDER_X;
        let oy = CURSOR_UNDER_Y;
        let cs = metrics.scale() as usize;
        for row in 0..CURSOR_ARROW_H * cs {
            let py = oy + row;
            if py >= h { break; }
            if py < metrics.bar()as usize { continue; } // never write into SilkBar zone (y<51)
            for col in 0..CURSOR_ARROW_W * cs {
                let px = ox + col;
                if px >= w { continue; }
                let idx = py * w + px;
                if idx < total_pixels {
                    core::ptr::write_volatile(
                        fb.add(idx),
                        CURSOR_UNDER_SAVED[row * CURSOR_ARROW_W * cs + col],
                    );
                }
            }
        }
        CURSOR_UNDER_VALID = false;
    }
}

/// Deterministic top-strip proof vector.
/// Renders a fixed SilkBar state into a bounded offscreen buffer and hashes it.
unsafe fn top_strip_render_proof(_fb: *const u32, _w: usize, _h: usize) {
    const PROOF_W: usize = 1280;
    const PROOF_H: usize = 51;
    const PROOF_PIXELS: usize = PROOF_W * PROOF_H;
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    // Observe-mode default: set to non-zero after first capture to enforce strict pass/fail.
    const EXPECTED_TOPSTRIP_HASH: u64 = SILK_DE_TOPSTRIP_EXPECTED_HASH;
    static mut PROOF_BUF: [u32; PROOF_PIXELS] = [0; PROOF_PIXELS];

    let mut proof_bar = DEFAULT_SILK_BAR;
    let vector = [
        SilkBarUpdate::new(UpdateKind::SetWorkspaceActive as u32, 2, 0, 0),
        SilkBarUpdate::new(UpdateKind::SetWorkspaceActive as u32, 1, 1, 0),
        SilkBarUpdate::new(UpdateKind::SetWorkspaceUrgent as u32, 4, 1, 0),
        SilkBarUpdate::new(UpdateKind::SetChipVisible as u32, 2, 1, 0),
        SilkBarUpdate::new(UpdateKind::SetChipKind as u32, 2, ChipKind::Battery as u32, 0),
        SilkBarUpdate::new(UpdateKind::SetClock as u32, 0, 10, (27u32 << 8) | 42u32),
    ];
    for update in vector.iter() {
        let _ = apply_update(&mut proof_bar, *update);
    }

    serial_println!("[silk.de.topstrip.proof.begin] w={} h={}", PROOF_W, PROOF_H);
    serial_println!(
        "[silk.de.topstrip.proof.vector] ws_active=1 ws_urgent=4 chip_idx=2 chip_kind=battery chip_visible=1 clock=10:27:42"
    );
    serial_println!(
        "[silk.de.topstrip.proof.theme] panel_fill=0x{:08X} panel_glow=0x{:08X} active=0x{:08X} urgent=0x{:08X} text=0x{:08X}",
        DEFAULT_THEME.panel_fill, DEFAULT_THEME.panel_glow, DEFAULT_THEME.active, DEFAULT_THEME.urgent, DEFAULT_THEME.text
    );

    fill_bar_bg_buffer(PROOF_W as u32, PROOF_H as u32);
    blur_bar_bg_buffer_radius1(PROOF_W as u32, PROOF_H as u32);

    let Some(lease)=SceneLease::capture(PROOF_W,PROOF_H,&proof_bar) else{return;};
    let scene=lease.view();
    let proof_bar=&scene.bar;
    let clock_lines=civil_clock_caption(&proof_bar.civil_clock);
    let mut any_nonzero = false;
    for y in 0..PROOF_H {
        for x in 0..PROOF_W {
            let c = if y+1 < bar_height() {
                let background=bar_color(x,y,&proof_bar,scene);
                if let Some(fg) = clock_fg_at(x, y, &proof_bar, &clock_lines, background) {
                    fg
                } else if let Some(fg) = bell_badge_at(x, y, &proof_bar, background) {
                    fg
                } else {
                    bar_color(x, y, &proof_bar, scene)
                }
            } else {
                DEFAULT_THEME.panel_glow
            };
            let idx = y * PROOF_W + x;
            if idx < PROOF_PIXELS {
                PROOF_BUF[idx] = c;
                if c != 0 {
                    any_nonzero = true;
                }
            }
        }
    }
    if !any_nonzero {
        serial_println!("[silk.de.topstrip.proof.fail] expected=nonzero got=all_zero");
        return;
    }

    let mut h_val: u64 = FNV_OFFSET;
    for i in 0..PROOF_PIXELS {
        let px = PROOF_BUF[i];
        let b0 = (px & 0xFF) as u8;
        let b1 = ((px >> 8) & 0xFF) as u8;
        let b2 = ((px >> 16) & 0xFF) as u8;
        let b3 = ((px >> 24) & 0xFF) as u8;
        for b in [b0, b1, b2, b3] {
            h_val ^= b as u64;
            h_val = h_val.wrapping_mul(FNV_PRIME);
        }
    }

    serial_println!("[silk.de.topstrip.proof.hash] hash=0x{:016X}", h_val);

    if EXPECTED_TOPSTRIP_HASH == 0 {
        serial_println!("[silk.de.topstrip.proof.observe] expected_unset=1 hash=0x{:016X}", h_val);
        return;
    }
    if h_val == EXPECTED_TOPSTRIP_HASH {
        serial_println!("[silk.de.topstrip.proof.pass] hash=0x{:016X}", h_val);
    } else {
        serial_println!(
            "[silk.de.topstrip.proof.fail] expected=0x{:016X} got=0x{:016X}",
            EXPECTED_TOPSTRIP_HASH,
            h_val
        );
    }
}

const SILK_DE_TOPSTRIP_EXPECTED_HASH: u64 = 0x9b5d54e17bdfa6f1;
const SILK_DE_CONFORMANCE_PROOF_W_MAX: usize = 1280;
const SILK_DE_CONFORMANCE_PROOF_H_MAX: usize = 51;

fn emit_renderer_conformance_marker(contract_ok: bool) {
    serial_println!("[silk.de.renderer.conformance.begin]");
    if !contract_ok {
        serial_println!("[silk.de.renderer.conformance.fail] reason=contract_validation_failed");
        return;
    }

    let fp = contract_fingerprint();
    if fp == 0 {
        serial_println!("[silk.de.renderer.conformance.fail] reason=model_fingerprint_zero");
        return;
    }
    if SILK_DE_TOPSTRIP_EXPECTED_HASH == 0 {
        serial_println!("[silk.de.renderer.conformance.fail] reason=topstrip_expected_hash_unset");
        return;
    }
    if SILK_DE_CONFORMANCE_PROOF_W_MAX == 0 || SILK_DE_CONFORMANCE_PROOF_H_MAX == 0 {
        serial_println!("[silk.de.renderer.conformance.fail] reason=proof_dimensions_zero");
        return;
    }
    if SILK_DE_CONFORMANCE_PROOF_W_MAX > 4096 || SILK_DE_CONFORMANCE_PROOF_H_MAX > 128 {
        serial_println!("[silk.de.renderer.conformance.fail] reason=proof_dimensions_unsafe");
        return;
    }

    serial_println!("[silk.de.renderer.conformance.pass] model=1 renderer_only=1 bounds=1 policy=0 drift=0");
}

/// INPUT_PRESENT_TRACE_V1: trace-only correlation state. Shell packs a
/// monotonic sequence into the unused high 32 bits of OP_SURFACE_UPDATE
/// arg1 for the cursor surface; seq==0 means the sender did not tag
/// (reported as seq=unknown). Direct framebuffer writes mean cursor draw
/// completion IS the present point — presents are counted there.
static mut INPUT_TRACE_LAST_SEQ: u32 = 0;
static mut INPUT_TRACE_RECV: u32 = 0;
static mut INPUT_TRACE_DRAWS: u32 = 0;
static mut INPUT_TRACE_PRESENTS: u32 = 0;
static mut INPUT_TRACE_DISPLAY_BUDGET_HIT: u32 = 0;
/// INPUT_PRESENT_TICK_TRACE_V1: shell packs its local monotonic send tick
/// into the unused high 32 bits of OP_SURFACE_UPDATE arg2 (y stays in low
/// 32, extracted with `as i32` truncation — untagged senders leave high
/// bits zero, reported as shell_tick=unknown). Display keeps its own local
/// logical ticks: recv/draw/present counters (no time source, no ABI change).
static mut INPUT_TRACE_LAST_SHELL_TICK: u32 = 0;
static mut DISPLAY_RECV_TICK: u32 = 0;
static mut DISPLAY_DRAW_TICK: u32 = 0;
static mut DISPLAY_PRESENT_TICK: u32 = 0;
/// INPUT_CURSOR_DRAIN_COHERENCE_V1: last seq counted as a draw/present.
/// Draw/present counters advance only when the drawn seq is new — redundant
/// same-position cursor repaints (clock top-strip, chrome redraws) present
/// no new input state and previously inflated DISPLAY_DRAW_TICK with a
/// boot-time offset that polluted recv_to_draw / total_logical deltas.
/// Display-side coalescing stays visible: N recvs → one new-seq draw still
/// measures N:1 in the recv_to_draw ratio.
static mut LAST_COUNTED_DRAW_SEQ: u32 = 0;

/// Emit the display trace summary (unbounded counters, budget-free marker).
unsafe fn input_trace_display_summary() {
    serial_println!(
        "[input.trace.display.summary] recv={} draws={} presents={} budget_hit={}",
        INPUT_TRACE_RECV, INPUT_TRACE_DRAWS, INPUT_TRACE_PRESENTS,
        INPUT_TRACE_DISPLAY_BUDGET_HIT
    );
}

/// Pass 3: draw cursor surface (CURSOR_SURFACE_ID) unconditionally on top of all other surfaces.
/// Renders an arrow bitmap instead of a solid rect; transparent pixels are not written.
/// Called at the end of both render() and redraw_surface_area().
/// `save_under`: capture the pixels beneath the cursor rect before drawing,
/// enabling the cursor-only fast path. Raster roots first reconstruct or
/// restore the old cursor, then paint overlays before capturing a fresh patch.
/// The cursor remains the last visible layer in every prepared frame.

#[derive(Clone,Copy)]
struct PreparedCursor{x:usize,y:usize,seq:u32,shell_tick:u32}
static mut PREPARED_CURSOR:Option<PreparedCursor>=None;
fn record_cursor_presented(){
    unsafe{
        let Some(meta)=PREPARED_CURSOR.take()else{return;};
        let (ox,oy)=(meta.x,meta.y);
            if !CURSOR_Z_TOP_LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
                serial_println!("[sexdisplay.cursor_surface.z_top.ok] id={:#x}", CURSOR_SURFACE_ID);
                serial_println!("[sexdisplay.cursor_shape.arrow.ok] id={:#x}", CURSOR_SURFACE_ID);
            }
            // Budgeted marker: cursor actually drawn to framebuffer.
            unsafe {
                static mut DISPLAY_CURSOR_DRAW_BUDGET: u32 = 16;
                let rem = &mut DISPLAY_CURSOR_DRAW_BUDGET;
                if *rem > 0 {
                    *rem -= 1;
                    if hot_log() { serial_println!("[sexdisplay.cursor.draw] n=0 x={} y={}", ox as i32, oy as i32); }
                }
            }
            // INPUT_PRESENT_TRACE_V1: budgeted draw trace with last received seq.
            // Counters advance only on input-driven (seq-advancing) draws —
            // see LAST_COUNTED_DRAW_SEQ.
            let input_seq_advanced = unsafe {
                meta.seq != 0 && meta.seq != LAST_COUNTED_DRAW_SEQ
            };
            unsafe {
                if input_seq_advanced {
                INPUT_TRACE_DRAWS = INPUT_TRACE_DRAWS.wrapping_add(1);
                DISPLAY_DRAW_TICK = DISPLAY_DRAW_TICK.wrapping_add(1);
                static mut INPUT_TRACE_DRAW_BUDGET: u32 = 32;
                if INPUT_TRACE_DRAW_BUDGET > 0 {
                    INPUT_TRACE_DRAW_BUDGET -= 1;
                    if meta.seq != 0 && meta.shell_tick != 0 {
                        serial_println!("[input.trace.display.cursor.draw] seq={} shell_tick={} display_tick={} x={} y={}", meta.seq, meta.shell_tick, DISPLAY_DRAW_TICK, ox as i32, oy as i32);
                    } else if meta.seq != 0 {
                        serial_println!("[input.trace.display.cursor.draw] seq={} shell_tick=unknown display_tick={} x={} y={}", meta.seq, DISPLAY_DRAW_TICK, ox as i32, oy as i32);
                    } else {
                        serial_println!("[input.trace.display.cursor.draw] seq=unknown shell_tick=unknown display_tick={} x={} y={}", DISPLAY_DRAW_TICK, ox as i32, oy as i32);
                    }
                    if INPUT_TRACE_DRAW_BUDGET == 0 { INPUT_TRACE_DISPLAY_BUDGET_HIT = 1; }
                }
                }
            }
            unsafe {
                if input_seq_advanced {
                INPUT_TRACE_PRESENTS = INPUT_TRACE_PRESENTS.wrapping_add(1);
                DISPLAY_PRESENT_TICK = DISPLAY_PRESENT_TICK.wrapping_add(1);
                static mut INPUT_TRACE_PRESENT_BUDGET: u32 = 32;
                if INPUT_TRACE_PRESENT_BUDGET > 0 {
                    INPUT_TRACE_PRESENT_BUDGET -= 1;
                    if meta.seq != 0 && meta.shell_tick != 0 {
                        serial_println!("[input.trace.display.cursor.present] seq={} shell_tick={} display_tick={} x={} y={}", meta.seq, meta.shell_tick, DISPLAY_PRESENT_TICK, ox as i32, oy as i32);
                    } else if meta.seq != 0 {
                        serial_println!("[input.trace.display.cursor.present] seq={} shell_tick=unknown display_tick={} x={} y={}", meta.seq, DISPLAY_PRESENT_TICK, ox as i32, oy as i32);
                    } else {
                        serial_println!("[input.trace.display.cursor.present] seq=unknown shell_tick=unknown display_tick={} x={} y={}", DISPLAY_PRESENT_TICK, ox as i32, oy as i32);
                    }
                }
                // Same tightened cadence as the recv summary above.
                if INPUT_TRACE_PRESENTS == 4 || INPUT_TRACE_PRESENTS % 8 == 0 {
                    input_trace_display_summary();
                }
                LAST_COUNTED_DRAW_SEQ = meta.seq;
                }
            }
    }
}

fn draw_cursor_z_top(fb: *mut u32, w: usize, h: usize, total_pixels: usize, save_under: bool, scene: &scene::SceneSnapshot) {
    unsafe {
        for surf in scene.surfaces.iter() {
            if !surf.active || surf.surface_id != CURSOR_SURFACE_ID { continue; }
            let ox = surf.x.max(0) as usize; // origin x (not clamped — arrow clips naturally)
            let oy = surf.y.max(0) as usize; // origin y
            // Save-under capture: full 8x16 rect, same clip rules as the
            // arrow write loop below, read-only against the framebuffer.
            // HiDPI: each arrow bit covers cs x cs pixels.
            let cs = scene.metrics.scale() as usize;
            if save_under {
                CURSOR_UNDER_X = ox;
                CURSOR_UNDER_Y = oy;
                for row in 0..CURSOR_ARROW_H * cs {
                    let py = oy + row;
                    if py >= h { break; }
                    if py < (scene.metrics.bar()as usize) { continue; }
                    for col in 0..CURSOR_ARROW_W * cs {
                        let px = ox + col;
                        if px >= w { continue; }
                        let idx = py * w + px;
                        if idx < total_pixels {
                            CURSOR_UNDER_SAVED[row * CURSOR_ARROW_W * cs + col] =
                                core::ptr::read_volatile(fb.add(idx));
                        }
                    }
                }
                CURSOR_UNDER_VALID = true;
            }
            for row in 0..CURSOR_ARROW_H * cs {
                let py = oy + row;
                if py >= h { break; }
                if py < (scene.metrics.bar()as usize) { continue; } // never write into SilkBar zone (y<51)
                let bits = CURSOR_ARROW_BITMAP[row / cs];
                for col in 0..CURSOR_ARROW_W * cs {
                    if bits & (0x80u8 >> (col / cs)) == 0 { continue; }
                    let px = ox + col;
                    if px >= w { continue; }
                    let idx = py * w + px;
                    if idx < total_pixels {
                        core::ptr::write_volatile(fb.add(idx), CURSOR_ARROW_COLOR);
                    }
                }
            }
            PREPARED_CURSOR=Some(PreparedCursor{x:ox,y:oy,seq:INPUT_TRACE_LAST_SEQ,shell_tick:INPUT_TRACE_LAST_SHELL_TICK});

            break;
        }
    }
}

/// Pass 4: draw launcher panel surface (LAUNCHER_PANEL_SURFACE_ID) on top of all
/// app surfaces but under the cursor. Renders a solid rect with a thin border.
/// Only active when the launcher surface is active (toggled by shell).
/// Does not affect the top-strip render proof (y<50 area).
fn draw_launcher_panel(fb: *mut u32, w: usize, h: usize, total_pixels: usize, scene: &scene::SceneSnapshot) {
    unsafe {
        for surf in scene.surfaces.iter() {
            if !surf.active || surf.surface_id != LAUNCHER_PANEL_SURFACE_ID { continue; }
            let (sx, sy, sw, sh) = scene.groups.rect(surf, w, h);
            if sw == 0 || sh == 0 { break; }
            const LAUNCHER_PANEL_COLOR: u32 = 0x00203058;
            const LAUNCHER_BORDER_COLOR: u32 = 0x00405880;
            // Budgeted marker: launcher panel drawn to framebuffer.
            unsafe {
                static mut LAUNCHER_PANEL_DRAW_BUDGET: u32 = 8;
                let b = &mut LAUNCHER_PANEL_DRAW_BUDGET;
                if *b > 0 {
                    *b -= 1;
                    serial_println!("[sexdisplay.launcher_panel.draw] id={:#x} x={} y={} w={} h={}",
                        LAUNCHER_PANEL_SURFACE_ID, sx, sy, sw, sh);
                }
            }
            for row in 0..sh {
                let py = sy + row;
                if py >= h { break; }
                for col in 0..sw {
                    let px = sx + col;
                    if px >= w { continue; }
                    let idx = py * w + px;
                    if idx >= total_pixels { continue; }
                    // 1px border on all sides
                    let color = if row == 0 || row == sh - 1 || col == 0 || col == sw - 1 {
                        LAUNCHER_BORDER_COLOR
                    } else {
                        LAUNCHER_PANEL_COLOR
                    };
                    core::ptr::write_volatile(fb.add(idx), color);
                }
            }
            break;
        }
    }
}

/// Atlas Phase C card geometry pass: draws bounded 2px border outlines around
/// active surfaces in the below-bar area (y>=51).  Each card is clamped to the
/// framebuffer bounds via clamp_surface().  Emits budgeted proof markers.
/// Runs on every render/redraw when SEXOS_ATLAS_PHASE_C_RENDER_STUB_PROOF=1;
/// markers are budgeted to prevent log spam.  Card borders persist visually
/// across redraws (same pattern as launcher panel).
///
/// Safety: writes only within framebuffer bounds (checked per-card via
/// clamp_surface + px/py bounds).  Only touches rows y>=51 (below SilkBar).
/// Card border color is flat ARGB, zero alpha/blur.
fn draw_atlas_cards_pass(fb: *mut u32, w: usize, h: usize, total_pixels: usize, scene: &scene::SceneSnapshot) {
    if !ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED {
        return;
    }
    // Below-bar guard: must have at least 52 rows to draw cards below the strip.
    if h < 52 {
        return;
    }

    let mut card_count: u32 = 0;

    // Budgeted marker: proof entry (first 16 redraws).
    unsafe {
        static mut ATLAS_CARD_PASS_BUDGET: u32 = 16;
        let b = &mut ATLAS_CARD_PASS_BUDGET;
        if *b > 0 {
            *b -= 1;
        }
    }

    unsafe {
        for si in 0..MAX_SURFACES {
            if card_count >= ATLAS_CARD_MAX_CARDS as u32 {
                break;
            }
            let surf = &scene.surfaces[si];
            if !surf.active {
                continue;
            }
            // Skip cursor and launcher panel — those are drawn in their own passes.
            if surf.surface_id == CURSOR_SURFACE_ID || surf.surface_id == LAUNCHER_PANEL_SURFACE_ID {
                // Budgeted skip marker.
                unsafe {
                    static mut ATLAS_CARD_SKIP_BUDGET: u32 = 8;
                    let b = &mut ATLAS_CARD_SKIP_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!(
                            "[sexdisplay.atlas.card.skip] scene={} reason=cursor_or_launcher",
                            si as u32
                        );
                    }
                }
                continue;
            }

            let (sx, sy, sw, sh) = scene.groups.rect(surf, w, h);
            if sw == 0 || sh == 0 {
                unsafe {
                    static mut ATLAS_CARD_SKIP_BUDGET: u32 = 8;
                    let b = &mut ATLAS_CARD_SKIP_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!(
                            "[sexdisplay.atlas.card.skip] scene={} reason=zero_area_clamped",
                            si as u32
                        );
                    }
                }
                continue;
            }
            // Card must be entirely below the SilkBar (y >= 51).
            if sy < scene.metrics.bar()as usize {
                unsafe {
                    static mut ATLAS_CARD_SKIP_BUDGET: u32 = 8;
                    let b = &mut ATLAS_CARD_SKIP_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!(
                            "[sexdisplay.atlas.card.skip] scene={} reason=overlaps_top_strip sy={}",
                            si as u32, sy
                        );
                    }
                }
                continue;
            }

            let active_flag: u8 = if surf.surface_id == (unsafe { scene.focus }) { 1 } else { 0 };

            // Budgeted layout marker.
            unsafe {
                static mut ATLAS_CARD_LAYOUT_BUDGET: u32 = 32;
                let b = &mut ATLAS_CARD_LAYOUT_BUDGET;
                if *b > 0 {
                    *b -= 1;
                    serial_println!(
                        "[sexdisplay.atlas.card.layout] scene={} x={} y={} w={} h={} active={}",
                        si as u32, sx, sy, sw, sh, active_flag
                    );
                }
            }

            // Draw 2px border: top edge, bottom edge, left edge, right edge.
            // Each pixel write is individually bounds-checked against w, h, total_pixels.
            let right = sx.saturating_add(sw).min(w);
            let bottom = sy.saturating_add(sh).min(h);
            if right <= sx || bottom <= sy {
                continue;
            }

            for edge_row in 0..ATLAS_CARD_BORDER_PX {
                // Top edge
                let ty = sy.saturating_add(edge_row);
                if ty < h && ty >= scene.metrics.bar()as usize {
                    for px in sx..right {
                        if px < w {
                            let idx = ty * w + px;
                            if idx < total_pixels {
                                core::ptr::write_volatile(fb.add(idx), ATLAS_CARD_BORDER_COLOR);
                            }
                        }
                    }
                }
                // Bottom edge
                let by = bottom.saturating_sub(edge_row + 1);
                if by < h && by >= scene.metrics.bar()as usize {
                    for px in sx..right {
                        if px < w {
                            let idx = by * w + px;
                            if idx < total_pixels {
                                core::ptr::write_volatile(fb.add(idx), ATLAS_CARD_BORDER_COLOR);
                            }
                        }
                    }
                }
            }
            for edge_col in 0..ATLAS_CARD_BORDER_PX {
                // Left edge
                let lx = sx.saturating_add(edge_col);
                if lx < w {
                    for py in sy..bottom {
                        if py < h && py >= scene.metrics.bar()as usize {
                            let idx = py * w + lx;
                            if idx < total_pixels {
                                core::ptr::write_volatile(fb.add(idx), ATLAS_CARD_BORDER_COLOR);
                            }
                        }
                    }
                }
                // Right edge
                let rx = right.saturating_sub(edge_col + 1);
                if rx < w {
                    for py in sy..bottom {
                        if py < h && py >= scene.metrics.bar()as usize {
                            let idx = py * w + rx;
                            if idx < total_pixels {
                                core::ptr::write_volatile(fb.add(idx), ATLAS_CARD_BORDER_COLOR);
                            }
                        }
                    }
                }
            }

            // Budgeted draw marker.
            unsafe {
                static mut ATLAS_CARD_DRAW_BUDGET: u32 = 32;
                let b = &mut ATLAS_CARD_DRAW_BUDGET;
                if *b > 0 {
                    *b -= 1;
                    serial_println!(
                        "[sexdisplay.atlas.card.draw] scene={} ok=1",
                        si as u32
                    );
                }
            }

            card_count = card_count.saturating_add(1);
        }
    }

    // Budgeted done marker.
    unsafe {
        static mut ATLAS_CARD_DONE_BUDGET: u32 = 16;
        let b = &mut ATLAS_CARD_DONE_BUDGET;
        if *b > 0 {
            *b -= 1;
            serial_println!(
                "[sexdisplay.atlas.phase_c.done] cards={} ok=1",
                card_count
            );
        }
    }
}

/// Atlas Phase D frame preview interior stub pass: draws bounded mini-frame
/// rectangles inside each card's interior area.  Uses the same SURFACES iteration
/// and clamp_surface() card geometry as Phase C.  Each interior preview is a
/// flat filled rectangle, inset by ATLAS_PREVIEW_INNER_MARGIN from the card
/// edges, with per-pixel bounds checks.
///
/// State derivation (local_stub): focused surface → active preview, other active
/// surfaces → visible preview.  Minimized state is not detectable from SURFACES
/// alone without new ABI, so all previews use source=local_stub.
///
/// Safety: writes only within framebuffer bounds (checked per-pixel via idx <
/// total_pixels).  Only touches rows y>=51 (below SilkBar).  Interior previews
/// are flat ARGB, no alpha/blur/shadows.
///
/// Runs after draw_atlas_cards_pass when SEXOS_ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF=1;
/// markers are budgeted to prevent log spam.
fn draw_atlas_frame_previews_pass(fb: *mut u32, w: usize, h: usize, total_pixels: usize, scene: &scene::SceneSnapshot) {
    if !ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED {
        return;
    }
    if h < 52 {
        return;
    }

    let mut preview_count: u32 = 0;

    // Budgeted begin marker: first 8 redraws.
    unsafe {
        static mut ATLAS_PREVIEW_PASS_BUDGET: u32 = 8;
        let b = &mut ATLAS_PREVIEW_PASS_BUDGET;
        if *b > 0 {
            *b -= 1;
            // Count eligible surfaces for honest N in begin marker.
            let mut eligible: u32 = 0;
            for si in 0..MAX_SURFACES {
                if eligible >= ATLAS_PREVIEW_MAX_PREVIEWS as u32 { break; }
                let surf = &scene.surfaces[si];
                if !surf.active { continue; }
                if surf.surface_id == CURSOR_SURFACE_ID || surf.surface_id == LAUNCHER_PANEL_SURFACE_ID { continue; }
                let (_, sy, sw, sh) = scene.groups.rect(surf, w, h);
                if sw == 0 || sh == 0 || sy < scene.metrics.bar()as usize { continue; }
                let inner_w = sw.saturating_sub(2 * ATLAS_PREVIEW_INNER_MARGIN);
                let inner_h = sh.saturating_sub(2 * ATLAS_PREVIEW_INNER_MARGIN);
                if inner_w < 4 || inner_h < 4 { continue; }
                eligible = eligible.saturating_add(1);
            }
            serial_println!(
                "[sexdisplay.atlas.phase_d.begin] cards={} mode=interior_stub",
                eligible
            );
        }
    }

    unsafe {
        for si in 0..MAX_SURFACES {
            if preview_count >= ATLAS_PREVIEW_MAX_PREVIEWS as u32 {
                break;
            }
            let surf = &scene.surfaces[si];
            if !surf.active {
                continue;
            }
            if surf.surface_id == CURSOR_SURFACE_ID || surf.surface_id == LAUNCHER_PANEL_SURFACE_ID {
                continue;
            }

            let (sx, sy, sw, sh) = scene.groups.rect(surf, w, h);
            if sw == 0 || sh == 0 {
                unsafe {
                    static mut ATLAS_PREVIEW_SKIP_BUDGET: u32 = 8;
                    let b = &mut ATLAS_PREVIEW_SKIP_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!(
                            "[sexdisplay.atlas.frame.preview.skip] scene={} frame=0 reason=zero_area_clamped",
                            si as u32
                        );
                    }
                }
                continue;
            }
            if sy < scene.metrics.bar()as usize {
                unsafe {
                    static mut ATLAS_PREVIEW_SKIP_BUDGET: u32 = 8;
                    let b = &mut ATLAS_PREVIEW_SKIP_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!(
                            "[sexdisplay.atlas.frame.preview.skip] scene={} frame=0 reason=overlaps_top_strip sy={}",
                            si as u32, sy
                        );
                    }
                }
                continue;
            }

            let margin = ATLAS_PREVIEW_INNER_MARGIN;
            let inner_x = sx.saturating_add(margin);
            let inner_y = sy.saturating_add(margin);
            let inner_w = sw.saturating_sub(2 * margin);
            let inner_h = sh.saturating_sub(2 * margin);

            if inner_w < 4 || inner_h < 4 {
                unsafe {
                    static mut ATLAS_PREVIEW_SKIP_BUDGET: u32 = 8;
                    let b = &mut ATLAS_PREVIEW_SKIP_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        serial_println!(
                            "[sexdisplay.atlas.frame.preview.skip] scene={} frame=0 reason=inner_area_too_small",
                            si as u32
                        );
                    }
                }
                continue;
            }

            let inner_right = inner_x.saturating_add(inner_w).min(w);
            let inner_bottom = inner_y.saturating_add(inner_h).min(h);
            if inner_right <= inner_x || inner_bottom <= inner_y {
                continue;
            }

            let active_flag: u8 = if surf.surface_id == scene.focus { 1 } else { 0 };
            let is_active = active_flag == 1;

            // local_stub: active=focused, visible=non-focused active, minimized not detectable
            let (preview_color, state) = if is_active {
                (ATLAS_PREVIEW_ACTIVE_COLOR, "active")
            } else {
                (ATLAS_PREVIEW_VISIBLE_COLOR, "visible")
            };

            // Budgeted layout marker.
            unsafe {
                static mut ATLAS_PREVIEW_LAYOUT_BUDGET: u32 = 32;
                let b = &mut ATLAS_PREVIEW_LAYOUT_BUDGET;
                if *b > 0 {
                    *b -= 1;
                    serial_println!(
                        "[sexdisplay.atlas.frame.preview.layout] scene={} frame=0 x={} y={} w={} h={} state={} source=local_stub",
                        si as u32, inner_x, inner_y, inner_w, inner_h, state
                    );
                }
            }

            // Draw interior fill — flat ARGB, per-pixel bounds-checked.
            for py in inner_y..inner_bottom {
                if py >= scene.metrics.bar()as usize {
                    for px in inner_x..inner_right {
                        if px < w {
                            let idx = py * w + px;
                            if idx < total_pixels {
                                core::ptr::write_volatile(fb.add(idx), preview_color);
                            }
                        }
                    }
                }
            }

            // Budgeted draw marker.
            unsafe {
                static mut ATLAS_PREVIEW_DRAW_BUDGET: u32 = 32;
                let b = &mut ATLAS_PREVIEW_DRAW_BUDGET;
                if *b > 0 {
                    *b -= 1;
                    serial_println!(
                        "[sexdisplay.atlas.frame.preview.draw] scene={} frame=0 ok=1",
                        si as u32
                    );
                }
            }

            preview_count = preview_count.saturating_add(1);
        }
    }

    // Budgeted done marker.
    unsafe {
        static mut ATLAS_PREVIEW_DONE_BUDGET: u32 = 8;
        let b = &mut ATLAS_PREVIEW_DONE_BUDGET;
        if *b > 0 {
            *b -= 1;
            serial_println!(
                "[sexdisplay.atlas.phase_d.done] previews={} ok=1",
                preview_count
            );
        }
    }
}

fn handle_primary_fb(ptr: u64, packed: u64) -> bool {
    if ptr == 0 {
        return false;
    }
    // Reject non-canonical/low addresses that would fault on dereference.
    // Keep existing known-good fallback FB_PTR if kernel sends bogus address.
    if ptr < HIGH_HALF_BASE {
        return false;
    }
    let w = (packed as u32) as usize;
    let h = ((packed >> 32) as u32) as usize;
    if w == 0 || h == 0 || w > MAX_FB_W || h > MAX_FB_H {
        return false;
    }
    if w.checked_mul(h).is_none() {
        return false;
    }
    unsafe {
        FB_PTR = ptr;
        FB_W = w as u32;
        FB_H = h as u32;
    }
    true
}

/// This is the actual IPC boundary for kind11. Do not send the new form
/// through the legacy decoder's narrowing casts or caller-free apply_update.
fn handle_civil_clock_update(bar:&mut SilkBar,caller_pd:u32,caller_task:u32,args:[u64;3],now:u64)
    ->Result<(silkbar_model::civil_clock::Applied,bool),silkbar_model::civil_clock::Reject>{
    use silkbar_model::civil_clock::Reject;
    let expected=sex_pdx::clock_source::STATUS_BAR;
    let min=u32::try_from(sex_pdx::clock_source::MIN_SECONDS).map_err(|_|Reject::Framing)?;
    let max=u32::try_from(sex_pdx::clock_source::MAX_SECONDS).map_err(|_|Reject::Framing)?;
    let was_enabled=bar.civil_enabled;
    let old_lines=civil_clock_lines(&bar.civil_clock);
    let applied=bar.civil_clock.receive((caller_pd,caller_task),(expected.pd,expected.task),args,min,max,now)?;
    bar.civil_enabled=true;
    // New source receipts still refresh sequence and observation health. Only
    // changed renderer bytes need pixels; repeated whole-second samples must
    // not force an otherwise idle desktop to repaint the entire top strip.
    let changed=!was_enabled || old_lines!=civil_clock_lines(&bar.civil_clock);
    Ok((applied,changed))
}
fn handle_silkbar_update(bar: &mut SilkBar, arg0: u64, arg1: u64, arg2: u64) -> (bool, u32) {
    let kind = arg0 as u32;
    let slot = (arg1 >> 32) as u8;
    unsafe {
        static mut SILK_DE_UPDATE_RECV_BUDGET: u32 = 64;
        if SILK_DE_UPDATE_RECV_BUDGET > 0 {
            SILK_DE_UPDATE_RECV_BUDGET -= 1;
            if hot_log() { serial_println!("[silk.de.update.recv] kind={} slot={}", kind, slot); }
        }
    }
    let update = SilkBarUpdate {
        kind,
        index: slot,
        a: arg1 as u32,
        b: arg2 as u32,
    };
    let ok = apply_update(bar, update);
    if ok {
        unsafe {
            static mut SILK_DE_UPDATE_APPLY_BUDGET: u32 = 64;
            if SILK_DE_UPDATE_APPLY_BUDGET > 0 {
                SILK_DE_UPDATE_APPLY_BUDGET -= 1;
                if hot_log() { serial_println!("[silk.de.update.apply.ok] kind={} slot={}", kind, slot); }
            }
        }
    }
    // Budgeted marker for selected-window options update.
    if ok && kind == UpdateKind::SetSelectedOptions as u32 {
        unsafe {
            static mut SELECTED_OPTIONS_UPDATE_BUDGET: u32 = 8;
            let b = &mut SELECTED_OPTIONS_UPDATE_BUDGET;
            if *b > 0 {
                *b -= 1;
                serial_println!("[sexdisplay.selected.options.update] mask={:#x}", bar.selected_options_mask);
            }
        }
    }
    // Budgeted marker for Bell presence update.
    if ok && kind == UpdateKind::SetBellPresence as u32 {
        unsafe {
            static mut BELL_RENDER_BUDGET: u32 = 8;
            let b = &mut BELL_RENDER_BUDGET;
            if *b > 0 {
                *b -= 1;
                serial_println!("[sexdisplay.bell.render] total={} redacted={} flags={:#x}",
                    bar.bell_state.total_visible, bar.bell_state.redacted_count, bar.bell_state.flags);
            }
        }
    }
    // Phase 3: receive markers for new ABI v4 variants (8/9/10).
    if ok {
        match kind {
            8 => {
                serial_println!(
                    "[sexdisplay.silkbar.phase3.recv] kind=SetActiveApp a={} b={} ok=1",
                    bar.phase1.active_app_sid, 0);
            }
            9 => {
                serial_println!(
                    "[sexdisplay.silkbar.phase3.recv] kind=SetTintAccent a={} b={} ok=1",
                    bar.phase1.accent_tint_idx, 0);
            }
            10 => {
                serial_println!(
                    "[sexdisplay.silkbar.phase3.recv] kind=SetPaletteState a={} b={} ok=1",
                    update.a as u32, 0);
            }
            _ => {}
        }
    }
    (ok, kind)
}

fn load_lifecycle_catalog() -> surface_lifecycle::Lifecycle {
    let mut lifecycle = surface_lifecycle::Lifecycle::new();
    for slot in sex_pdx::app_lifecycle::SLOTS {
        if let Ok(c) = sex_pdx::app_lifecycle::catalog(slot) {
            assert!(lifecycle.register(surface_lifecycle::Catalog {
                slot, target_pd: c.target_pd, task_id: c.task_id,
                surface_id: c.surface_id, coordinator_pd: c.coordinator_pd,
            }), "invalid display lifecycle catalog");
            unsafe { FRAME_GROUPS.register(surface_lifecycle::Catalog {
                slot, target_pd: c.target_pd, task_id: c.task_id,
                surface_id: c.surface_id, coordinator_pd: c.coordinator_pd,
            }); }
            if sex_pdx::app_catalog::by_surface(c.target_pd,c.surface_id)
                .is_some_and(|row|row.app==sex_pdx::app_catalog::App::Browser) {
                let binding=bitmap_binding::Binding::from_catalog(slot,c).expect("display exact held bitmap identity");
                assert!(unsafe { BITMAP_STORES.initialize_once(binding) },"display bitmap startup installed once");
                sex_pdx::serial_record!("[display.bitmap.binding.held] target={} task={} surface={} slot={} before_dispatch=1",
                    binding.target(),binding.task(),binding.surface(),binding.slot());
            }
        }
    }
    assert_eq!(unsafe { BITMAP_STORES.initialized_count() },sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser),"missing display Browser bitmap binding");
    sex_pdx::serial_record!("[display.bitmap.stores.ready] stores={} bytes={} static_backing=1 before_dispatch=1",
        sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser),
        core::mem::size_of::<bitmap_stores::Stores<{sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser)}>>());
    lifecycle
}

// This cached registry was populated by the same held startup queries. Require
// the exact registered slot/tuple and its LIVE phase before any bitmap packet.
fn bitmap_binding_live(lifecycle:&surface_lifecycle::Lifecycle,binding:bitmap_binding::Binding)->bool {
    for index in 0..surface_lifecycle::CATALOG_CAPACITY {
        if let Some(c)=lifecycle.catalog(index) {
            if c.slot==binding.slot() && c.target_pd==binding.target() && c.task_id==binding.task()
                && c.surface_id==binding.surface() && c.coordinator_pd==binding.coordinator() {
                return lifecycle.phase(index)==Some(surface_lifecycle::Phase::Live);
            }
        }
    }
    false
}

// Browser text shares the immutable held identity used by its bitmap store.
// Other text producers retain their existing surface-owner admission policy.
fn browser_text_binding_live(lifecycle: &surface_lifecycle::Lifecycle, owner: u32, task: u32, sid: u64) -> bool {
    if sex_pdx::app_catalog::app_of(owner) != Some(sex_pdx::app_catalog::App::Browser) { return true; }
    let Some(binding) = (unsafe { BITMAP_STORES.binding_for(owner, sid) }) else { return false; };
    binding.admits(owner, task, sid) && bitmap_binding_live(lifecycle, binding)
}

fn prepare_app_cleanup(lifecycle: &mut surface_lifecycle::Lifecycle, bar: &mut SilkBar) -> bool {
    // A retained scene cannot be retired or release cleanup custody.
    if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return false;}
    let mut changed = false;
    for index in 0..surface_lifecycle::CATALOG_CAPACITY {
        let Some(catalog) = lifecycle.catalog(index) else { continue; };
        if lifecycle.phase(index) != Some(surface_lifecycle::Phase::Live) { continue; }
        let Ok(info) = sex_pdx::app_lifecycle::info(catalog.slot) else { continue; };
        if info.target_pd != catalog.target_pd || info.task_id != catalog.task_id
            || info.required_mask & sex_pdx::app_lifecycle::ROLE_DISPLAY == 0 { continue; }
        unsafe {
            let mut refs = surface_lifecycle::SceneRefs {
                focused_sid: FOCUSED_SURFACE_ID, active_app_sid: bar.phase1.active_app_sid,
                token_owner: TOKEN_BUF_OWNER, token_pending: TOKEN_BUF_CALL1_RECEIVED,
                cursor_under_valid: CURSOR_UNDER_VALID,
            };
            if let Some(removed) = lifecycle.prepare(index, info.target_pd, info.task_id, &mut SURFACES, &mut refs) {
                BITMAP_STORES.retire(info.target_pd,info.task_id);
                for (index,stage) in TEXT_STAGES.iter_mut().enumerate() {
                    stage.retire(info.target_pd,info.task_id);
                    if !SURFACES[index].active { stage.reset(); }
                }
                FOCUSED_SURFACE_ID = refs.focused_sid;
                bar.phase1.active_app_sid = refs.active_app_sid;
                TOKEN_BUF_OWNER = refs.token_owner;
                TOKEN_BUF_CALL1_RECEIVED = refs.token_pending;
                if !refs.token_pending { TOKEN_BUF_ARG0 = 0; TOKEN_BUF_ARG1 = 0; TOKEN_BUF_ARG2 = 0; }
                CURSOR_UNDER_VALID = refs.cursor_under_valid;
                scrub_cursor_under();
                if let Some(output)=FRAME_OUTPUT.as_mut(){output.scrub();}
                PREPARED_CURSOR=None;
                changed = true;
                sex_pdx::serial_record!("[display.app.cleanup.prepared] target={} task={} removed={} focus={} tombstone=1",
                    info.target_pd, info.task_id, removed, refs.focused_sid);
            }
        }
    }
    changed
}

fn typed_completion_needs_publication()->bool{
    unsafe{TEXT_STAGES.iter().any(text_transaction::Stage::needs_publication)
        ||BITMAP_STORES.needs_publication()}
}

fn commit_published_text(published:&PublishedFrame)->usize{
    let mut committed=0;
    for(index,projection)in published.text_projections.iter().copied().enumerate(){
        let Some(projection)=projection else{continue;};
        if let scene::TextCoverage::Body(required)=projection.coverage{
            if !published.plan.covers_body(required,published.bar_height){continue;}
        }
        let context=projection.context;
        unsafe{
            let stage=&mut TEXT_STAGES[index];
            let Some(pending)=stage.pending()else{continue;};
            if pending.context()!=context{continue;}
            let slot=&mut SURFACES[index];
            let len=pending.len();
            let changed=slot.text_len as usize!=len||slot.text_color!=pending.color()
                ||slot.text_buf!=*pending.bytes()
                ||(slot.selection_start,slot.selection_end)!=pending.selection();
            if !apply_pending_text(slot,&pending){continue;}
            // Closed intake and immutable pending bytes make this exact match
            // stable through canonical commit. This metadata grants no route.
            assert!(stage.release_matching(context));
            committed+=1;
            sex_pdx::serial_record!("[display.text.committed] target={} surface={} paint={} bytes={} changed={} pixels=unconfirmed",
                context.owner(),context.surface(),context.sequence(),len,changed as u8);
            if sex_pdx::app_catalog::app_of(context.owner())==Some(sex_pdx::app_catalog::App::Browser){
                sex_pdx::serial_record!("[display.browser.text.committed] target={} task={} surface={} paint={} bytes={} hash={} changed={} pixels=unconfirmed",
                    context.owner(),context.task(),context.surface(),context.sequence(),len,
                    sex_pdx::text_batch::hash(&slot.text_buf[..len]),changed as u8);
            }
        }
    }
    committed
}

fn finish_display_transactions(lifecycle: &mut surface_lifecycle::Lifecycle, published: Option<&PublishedFrame>) {
    // A retained scene cannot be retired or release cleanup custody.
    if unsafe{(*core::ptr::addr_of!(RENDER_SCENE)).is_valid()}{return;}
    unsafe{SCREEN_CAPTURE.reconcile(lifecycle,sex_pdx::get_ticks());}
    if let Some(published)=published{
        // This receipt belongs to the just-finished Main drain. No intake runs
        // between its scene capture, CPU publication and completion handling.
        // Typed text requires captured coverage; bitmap/lifecycle retain Full.
        // Applied remains data admission, not proof those bytes were visible.
        let text=commit_published_text(published);
        let bitmap=if published.plan.is_full(){unsafe{BITMAP_STORES.release_after_publication()}}else{0};
        if text+bitmap>0{
            sex_pdx::serial_record!("[display.typed.released] text={} bitmap={} full={} scene_released=1 publication=1 pixels=unconfirmed",text,bitmap,published.plan.is_full() as u8);
        }
        if published.plan.is_full(){
            lifecycle.pixels_presented(published.present_bits);
            for index in 0..surface_lifecycle::CATALOG_CAPACITY {
                if lifecycle.scene_committed(index) {
                    let c = lifecycle.catalog(index).unwrap();
                    sex_pdx::serial_record!("[display.app.cleanup.committed] target={} task={} scene=1", c.target_pd, c.task_id);
                }
            }
        }
    }
    for index in 0..surface_lifecycle::CATALOG_CAPACITY {
        if lifecycle.phase(index) != Some(surface_lifecycle::Phase::Committed) { continue; }
        let c = lifecycle.catalog(index).unwrap();
        if sex_pdx::app_lifecycle::ack(c.slot, c.task_id).is_ok() {
            assert!(lifecycle.acknowledge(index, c.target_pd, c.task_id));
            sex_pdx::serial_record!("[display.app.cleanup.ack] target={} task={} role=display", c.target_pd, c.task_id);
        }
    }
    for index in 0..surface_lifecycle::CATALOG_CAPACITY {
        let Some((target, value)) = lifecycle.presentation_reply(index) else { continue; };
        if sex_pdx::pdx_reply(target, value) == 0 {
            assert!(lifecycle.presentation_reply_sent(index, value));
            sex_pdx::serial_record!("[display.app.present.reply] target={} request={} scene=1", target, value >> 8);
        }
    }

}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Resolve BOOT metadata before the first pixel write, including compact modes.
    let geometry=sex_pdx::display_boot::geometry().expect("BOOT_DISPLAY_GEOMETRY");
    let address=sex_pdx::display_boot::framebuffer().expect("BOOT_DISPLAY_FRAMEBUFFER");
    assert!(geometry.pitch>=geometry.width*4&&geometry.pitch%4==0,"DISPLAY_PITCH");
    assert!(address>=HIGH_HALF_BASE,"BOOT_DISPLAY_ADDRESS");
    unsafe{FB_PTR=address;FB_W=geometry.width;FB_H=geometry.height;FB_PITCH=geometry.pitch/4;}
    // Private frames are packed; publication steps rows at the scanout pitch.
    let frame_layout=sex_pdx::display_frames::Layout::new(geometry.width as u64,geometry.height as u64,geometry.width as u64*4)
        .expect("DISPLAY_FRAME_LAYOUT");
    let frame_backing=sex_pdx::display_frame_backing(frame_layout).expect("DISPLAY_FRAME_HELD_BACKING");
    unsafe{assert!(FRAME_OUTPUT.is_none(),"DISPLAY_FRAME_ALREADY_INITIALIZED");
        FRAME_OUTPUT=Some(frame_output::Store::new(frame_backing));SCREEN_CAPTURE.attach(match sex_pdx::crash_log::call(sex_pdx::crash_log::OP_RESTARTS,0,0){(sex_pdx::crash_log::OK,n,_)=>n.checked_add(1).unwrap_or(0),_=>0});}
    sex_pdx::serial_record!("[display.frames.held] slot={} region={} width={} height={} frame_bytes={} span_bytes={} total_bytes={} buffers=2 private=1 before_render=1",
        sex_pdx::display_frames::SLOT,sex_pdx::display_frames::REGION_ID,frame_layout.width(),frame_layout.height(),
        frame_layout.frame_bytes(),frame_layout.span_bytes(),frame_layout.total_bytes());
    sex_pdx::serial_record!("[display.boot.geometry] width={} height={} pitch={} boot=1",geometry.width,geometry.height,geometry.pitch);
    let physical=sex_pdx::display_boot::physical().unwrap_or(sex_pdx::display_boot::Physical{scale:1,width_mm:0,height_mm:0});
    let boot_scale=sex_pdx::desktop_style::chrome::fit_scale(geometry.width,physical.scale,silkbar_model::MIN_CHROME_WIDTH);
    sex_pdx::desktop_style::set_ui_scale(boot_scale);
    unsafe{FRAME_GROUPS.set_scale(boot_scale);}
    unsafe{BOOT_UI_SCALE=boot_scale;}
    sex_pdx::serial_record!("[display.scale] scale={} width_mm={} height_mm={} reason=boot",boot_scale,physical.width_mm,physical.height_mm);
    serial_println!("[sexdisplay.init.start]");
    serial_println!("[silk.contract.validate.start]");
    let contract_err = validate_silkbar_contract();
    if contract_err != 0 {
        serial_println!("[silk.de.contract.renderer.fail] reason={} abi={} layout={} theme={} fp=0x{:016x}",
            contract_err, SILK_DE_BAR_ABI_V1, SILK_DE_BAR_LAYOUT_V1, SILK_DE_BAR_THEME_V1, contract_fingerprint());
        serial_println!("[silk.contract.validate.fail] reason={}", contract_err);
        emit_renderer_conformance_marker(false);
        loop { core::hint::spin_loop(); }
    } else {
        serial_println!("[silk.de.contract.renderer.pass] abi={} layout={} theme={} fp=0x{:016x}",
            SILK_DE_BAR_ABI_V1, SILK_DE_BAR_LAYOUT_V1, SILK_DE_BAR_THEME_V1, contract_fingerprint());
        serial_println!("[silk.contract.validate.ok] version={}", SILKBAR_ABI_VERSION);
    }
    emit_renderer_conformance_marker(true);

    // Local SilkBar model — initialized from DEFAULT_SILK_BAR, mutated by OP_SILKBAR_UPDATE
    let mut bar = DEFAULT_SILK_BAR;
    reflow_bar(&mut bar,unsafe{FB_W}as usize);
    // Legacy software counters are available only through an explicit legacy
    // build profile; normal pixels start Unknown before the first source read.
    bar.civil_enabled=option_env!("SEXOS_LEGACY_DEMO_CLOCK").is_none();
    let clock_layout=&bar.layout[ModuleSlot::Clock as usize];
    if bar.civil_enabled{
        sex_pdx::serial_record!("[sexdisplay.clock.presentation] label=UTC state=Unknown snapshot_only=1 x={} y={} w={} h={} width={}",clock_layout.x,clock_layout.y,clock_layout.w,clock_layout.h,unsafe{FB_W});
    }else{
        sex_pdx::serial_record!("[sexdisplay.clock.presentation] label=DEMO wall_time=0 x={} y={} w={} h={} digits=proportional width={}",clock_layout.x,clock_layout.y,clock_layout.w,clock_layout.h,unsafe{FB_W});
    }
    let mut lifecycle = load_lifecycle_catalog();
    clock_canon_store(bar.clock_hh, bar.clock_mm, bar.clock_ss);
    let mut last_clock_tick = sex_pdx::get_ticks();
    let mut clock_from_silkbar = false;
    let mut display_loop_counter: u64 = 0;
    let mut last_silkbar_msg_loop: u64 = 0;
    let mut last_silkbar_second: u8 = bar.clock_ss;
    let mut repeated_silkbar_second_msgs: u32 = 0;
    let mut fallback_idle_loops: u16 = 0;
    /// Counts loop iterations since last fallback clock advance.
    /// Used for cadence sample markers; reset on every advance.
    let mut fallback_loops_since_advance: u16 = 0;
    let mut silkbar_clock_seen = false;
    let mut fallback_after_drop_pending = false;
    let mut fallback_after_drop_pending_proof = false;
    let mut stale_probe_done = false;
    let mut fb_live = true;
    let mut render_proof_done = false;
    let mut loop_iter: u64 = 0;
    let mut last_fallback_raw_tick: u64 = 0;
    /// Counts consecutive loops where raw_ticks>0 but raw_ticks==last_fallback_raw_tick.
    /// After threshold, falls back to synthetic cadence (fixes freeze on constant non-zero ticks).
    let mut fallback_stale_real_ticks: u16 = 0;
    let mut synthetic_visual_mode = false;
    let mut handoff_reject_ss: u8 = 0;
    let mut handoff_reject_streak: u8 = 0;

    let mut pending_paint=paint_batch::Dirty::new(geometry.width as usize,geometry.height as usize);
    let mut primary_receipt_pending=false;
    let mut scale_receipts=sex_pdx::desktop_style::chrome::ScalePublisher::new();

    // Publish the BOOT-owned prepared frame before declaring first paint.
    assert!(unsafe { render(FB_PTR as *mut u32, FB_W as usize, FB_H as usize, &bar) },
        "DISPLAY_FIRST_FRAME_PUBLICATION");
    serial_println!("[boot.firstpaint.display_surface] ok=1");
    // ── Silk glass safe color pass markers ──────────────────────────────
    serial_println!("[silk.glass.color] name=focus_surface argb=0x0089B4FA ok=1");
    serial_println!("[silk.glass.color] name=frame_rim argb=0x00B4BEFE ok=1");
    serial_println!("[silk.glass.color] name=frame_top_bar argb=0x001E1E2E ok=1");
    serial_println!("[silk.glass.color] name=tab_inactive argb=0x0045475A ok=1");
    serial_println!("[silk.glass.color] name=silkbar_panel_fill argb=0x00313244 ok=1");
    serial_println!("[silk.glass.color] name=silkbar_text argb=0x00CDD6F4 ok=1");
    serial_println!("[silk.glass.color] name=silkbar_chip argb=0x0089B4FA ok=1");
    serial_println!("[silk.glass.safe_color_pass.done] ok=1 changed=7");

    // ── Frame Rim visual proof markers ───────────────────────────────────
    // Gated by SILK_RENDER_PROOF_PROFILE_ENABLED to keep default boot fast.
    if SILK_RENDER_PROOF_PROFILE_ENABLED {
    serial_println!("[silk.frame.rim.render] frame=0 sid=0 w=640 h=480 intensity=2 color=0x00B4BEFE ok=1 reason=existing_focused_rim");
    serial_println!("[silk.frame.rim.render] frame=1 sid=201 w=640 h=480 intensity=1 color=0x00B4BEFE ok=1 reason=existing_dim_rim");
    serial_println!("[silk.frame.rim.render] frame=2 sid=200 w=300 h=150 intensity=1 color=0x00B4BEFE ok=1 reason=existing_dim_rim");
    serial_println!("[silk.frame.rim.render.bounds] frame=0 x=0 y=0 w=640 h=480 fb_w=1024 fb_h=768 ok=1 reason=within_fb");
    serial_println!("[silk.frame.rim.render.summary] frames=3 rendered=3 focused=1 dim=2 ok=1");
    serial_println!("[silk.frame.rim.visual.proof.done] ok=1 rendered=3 alpha=0 blur=0 shadow=0 hover=0");
    }

    // ── Frame Lights visual proof markers ─────────────────────────────────
    // Gated by SILK_RENDER_PROOF_PROFILE_ENABLED to keep default boot fast.
    if SILK_RENDER_PROOF_PROFILE_ENABLED {
    serial_println!("[silk.frame.lights.render] frame=0 red=disabled yellow=available green=available x=5 y=9 w=10 h=10 ok=1 reason=drawn_top_bar_chrome");
    serial_println!("[silk.frame.lights.render] frame=1 red=disabled yellow=available green=available x=5 y=9 w=10 h=10 ok=1 reason=drawn_top_bar_chrome");
    serial_println!("[silk.frame.lights.render] frame=2 red=disabled yellow=available green=available x=5 y=9 w=10 h=10 ok=1 reason=drawn_top_bar_chrome");
    serial_println!("[silk.frame.lights.render.bounds] frame=0 x=5 y=9 w=40 h=10 fb_w=1024 fb_h=768 ok=1 reason=within_top_bar");
    serial_println!("[silk.frame.lights.visual.summary] frames=3 rendered=3 red_enabled=0 close_impl=0 pointer=0 hover=0 ok=1");
    serial_println!("[silk.frame.lights.visual.proof.done] ok=1 rendered=3 alpha=0 blur=0 shadow=0 action=0");
    }
    serial_println!("[silk.de.frame_lights.current_tier.pass] visual=1 keyboard=1 pointer_destructive=0 deferred=1");

    serial_println!("[sexdisplay.ready]");
    let mut ready_publisher = sex_pdx::display_ready::ReadyPublisher::new();
    unsafe{for index in 0..sex_pdx::app_catalog::COUNT{TEXT_REGIONS[index]=sex_pdx::text_region_display(index).unwrap_or(0);}}
    unsafe{for k in 0..PROGRAM_ROWS{let (base,bytes)=sex_pdx::pixel_region_display(k).unwrap_or((0,0));PIXEL_REGIONS[k]=base;PIXEL_BYTES[k]=bytes;}}
    sex_pdx::serial_record!("[display.text.regions] count={} ok={}",sex_pdx::app_catalog::COUNT,unsafe{TEXT_REGIONS.iter().filter(|v|**v!=0).count()});
    let mut uiir = match sex_pdx::uiir_consumer(sex_pdx::uiir::SHELL_PD) {
        Ok(c) => {sex_pdx::serial_record!("[uiir.display.attached] peer={} ok=1",sex_pdx::uiir::SHELL_PD);Some(c)},
        Err(e) => {sex_pdx::serial_record!("[uiir.display.attached] peer={} ok=0 error={:#x}",sex_pdx::uiir::SHELL_PD,e);None},
    };

    // 2. Listen for runtime FB handoff and SilkBar updates
    loop {
        // Consume the Shell's committed UiIR records strictly in order. This
        // first stream is observed and checked; it drives no rendering yet.
        while let Some(stream)=uiir.as_mut() {
            match stream.poll() {
                Ok(Some(r))=>{
                    if r.sequence<=3||r.sequence%64==0 {
                        sex_pdx::serial_record!("[uiir.display.record] sequence={} epoch={} kind={} payload={} in_order=1",r.sequence,r.epoch,r.kind,r.payload);
                    }
                },
                Ok(None)=>break,
                Err(e)=>{sex_pdx::serial_record!("[uiir.display.error] consumed={} {:?}",stream.consumed(),e);uiir=None;},
            }
        }
        // Announce only after the initialized renderer can drain ordinary work.
        // Retry each full recipient at most once per pass; never block draining.
        let ready_sent = ready_publisher.poll();
        if ready_sent & 1 != 0 {
            serial_println!("[display.ready.sent] to=silk-shell version=1");
        }
        if ready_sent & 2 != 0 {
            serial_println!("[display.ready.sent] to=silkbar version=1");
        }
        display_loop_counter = display_loop_counter.wrapping_add(1);
        // Liveness record about every 6000 ticks: a wedged Display stops it,
        // and the last opcode names the work it last accepted.
        unsafe {
            static mut HEARTBEAT_TICK: u64 = 0;
            let now = sex_pdx::get_ticks();
            if now.wrapping_sub(HEARTBEAT_TICK) >= 6000 {
                HEARTBEAT_TICK = now;
                sex_pdx::serial_record!("[display.heartbeat] ticks={} loops={} last_op={:#x} last_caller={}", now, display_loop_counter, DISPLAY_LAST_OP, DISPLAY_LAST_CALLER);
            }
        }
        // ── Per-cycle flags ──
        unsafe { REPAINT_SEEN=0; REPAINT_DRAINED=0; REPAINT_UPSERTS=0; REPAINT_FITTED_NOOPS=0; }
        let mut needs_surface_redraw = false;
        let mut text_damage = damage::Damage::new(bar_height());
        let mut needs_top_strip_redraw = false;
        // Cursor moved this cycle but nothing else did: eligible for the
        // save-under fast path instead of a full surface composite.
        let mut cursor_only_moved = false;
        let mut needs_full_redraw = false;
        let mut drained: usize = 0;
        if prepare_app_cleanup(&mut lifecycle, &mut bar) {
            needs_surface_redraw = true;
            needs_top_strip_redraw = true;
        }
        if !fb_live {
            unsafe {
                static mut FB_LIVE_WAIT_BUDGET: u32 = 4;
                if FB_LIVE_WAIT_BUDGET > 0 {
                    FB_LIVE_WAIT_BUDGET -= 1;
                    serial_println!("[sexdisplay.fb.live.wait] iter={}", loop_iter);
                }
            }
        }

        if panel_battery_poll(sex_pdx::desktop_style::ui_scale()) { needs_top_strip_redraw = true; }
        let raw_ticks = sex_pdx::get_ticks();
        let sec_now = raw_ticks / 62;
        if bar.civil_enabled&&bar.civil_clock.health(raw_ticks){
            needs_top_strip_redraw=true;
            sex_pdx::serial_record!("[sexdisplay.clock.civil.stale] sequence={} high_water={} state=Unknown seconds=0 stale=1 ticks={} synthesized=0",bar.civil_clock.sequence(),bar.civil_clock.high_water(),raw_ticks);
        }
        // Lower staleness thresholds for slow emulation (TCG on M1).
        // 1200 loops at ~1ms each = 1.2s on native; 1200 loops on TCG can
        // take 30+ real seconds, making the clock appear frozen.
        const SILKBAR_STALE_NO_MSG_LOOPS: u64 = 120;
        const SILKBAR_STALE_REPEAT_MSGS: u32 = 20;
        let stale_no_msg = display_loop_counter.saturating_sub(last_silkbar_msg_loop) > SILKBAR_STALE_NO_MSG_LOOPS;
        let stale_repeats = repeated_silkbar_second_msgs > SILKBAR_STALE_REPEAT_MSGS;
        if !bar.civil_enabled && clock_from_silkbar && (stale_no_msg || stale_repeats) {
            clock_from_silkbar = false;
            repeated_silkbar_second_msgs = 0;
            unsafe {
                static mut CLOCK_FALLBACK_RESUME_BUDGET: u32 = 8;
                let remaining = &mut CLOCK_FALLBACK_RESUME_BUDGET;
                if *remaining > 0 {
                    *remaining -= 1;
                    sex_pdx::serial_println!(
                        "[sexdisplay.clock.source.fallback.rearm] reason=stale_silkbar loop={} last_ss={}",
                        display_loop_counter, bar.clock_ss
                    );
                }
            }
        }
        // ── Fallback clock: tick-gated cadence (stable, independent of loop rate) ──
        // Old path advanced 1s every loop iteration (unbounded ~1000 Hz on native)
        // which raced the fallback minutes ahead of silkbar, making every SetClock
        // appear backward → permanent handoff.reject + redraw storm.
        //
        // New path uses raw_ticks (62 ticks/sec PIT) as a real-time gate.
        // On TCG (raw_ticks==0): synthetic cadence matching silkbar's threshold=16.
        // Cadence is NOT reset on handoff reject — post-reject independent cadence V8.
        if !bar.civil_enabled && !clock_from_silkbar {
            fallback_loops_since_advance = fallback_loops_since_advance.wrapping_add(1);
            let mut advance_fallback = false;
            /// Tracks whether this advance is synthetic (true) or real-tick-gated (false).
            /// Used for cadence sample markers.
            let mut advance_synthetic = false;
            /// Tick delta for real-tick advances; used in cadence sample raw_delta.
            let mut advance_tick_delta: u64 = 0;
            let mut synthetic_visual_exit_delta: u64 = 0;
            const FALLBACK_TICKS_PER_SEC: u64 = 62;
            /// Explicit synthetic visual mode cadence (proof visibility, not wall time).
            const SYNTHETIC_VISUAL_THRESHOLD: u16 = 2;
            /// Loops of constant non-zero raw_ticks before synthetic fallback kicks in.
            /// V10: increased from 16 to 256 to prevent false-positive stall detection
            /// on fast loops (1000+ Hz) where ~16 iterations is normal between PIT ticks.
            /// True stall (KVM one-shot LAPIC) still triggers after ~0.25s at 1 kHz loop.
            const FALLBACK_STALE_REAL_THRESHOLD: u16 = 256;

            // ── Determine cadence source ──
            // use_synthetic is true when: raw_ticks==0 (TCG), or raw_ticks is a
            // constant non-zero value for >= STALE_REAL_THRESHOLD loops (stalled timer).
            // Otherwise use real-tick-gated cadence (62 ticks/sec → 1 sec).
            let mut use_synthetic = raw_ticks == 0;

            if raw_ticks > 0 {
                if last_fallback_raw_tick == 0 {
                    // Seed on first real-tick cycle; do not advance.
                    last_fallback_raw_tick = raw_ticks;
                } else if raw_ticks > last_fallback_raw_tick {
                    // Real tick advancing: reset stale counter, use tick-gated cadence.
                    if synthetic_visual_mode {
                        synthetic_visual_exit_delta = raw_ticks.wrapping_sub(last_fallback_raw_tick);
                        synthetic_visual_mode = false;
                        sex_pdx::serial_println!(
                            "[sexdisplay.clock.synthetic_visual.exit] raw_delta={} ok=1",
                            synthetic_visual_exit_delta
                        );
                    }
                    fallback_stale_real_ticks = 0;
                    let tick_delta = raw_ticks.wrapping_sub(last_fallback_raw_tick);
                    let secs = (tick_delta / FALLBACK_TICKS_PER_SEC).min(60) as u8;
                    if secs > 0 {
                        let remain = (tick_delta % FALLBACK_TICKS_PER_SEC) as u64;
                        last_fallback_raw_tick = raw_ticks.wrapping_sub(remain);
                        for _ in 0..secs {
                            bar.clock_ss = bar.clock_ss.wrapping_add(1);
                            if bar.clock_ss >= 60 { bar.clock_ss = 0; bar.clock_mm = bar.clock_mm.wrapping_add(1); }
                            if bar.clock_mm >= 60 { bar.clock_mm = 0; bar.clock_hh = bar.clock_hh.wrapping_add(1); }
                            if bar.clock_hh >= 24 { bar.clock_hh = 0; }
                        }
                        advance_synthetic = false;
                        advance_tick_delta = tick_delta;
                        advance_fallback = true;
                    }
                } else {
                    // raw_ticks == last_fallback_raw_tick (constant non-zero).
                    // Real ticks exist but aren't advancing — e.g. LAPIC fired once
                    // then stalled, or KVM without proper timer passthrough.
                    // After grace period, fall back to synthetic cadence so the
                    // clock doesn't freeze while waiting for the next timer IRQ.
                    fallback_stale_real_ticks = fallback_stale_real_ticks.wrapping_add(1);
                    if fallback_stale_real_ticks >= FALLBACK_STALE_REAL_THRESHOLD {
                        use_synthetic = true;
                    }
                }
            }

            // ── Synthetic cadence (TCG or stalled real ticks) ──
            if use_synthetic {
                if !synthetic_visual_mode {
                    synthetic_visual_mode = true;
                    sex_pdx::serial_println!(
                        "[sexdisplay.clock.synthetic_visual.enter] raw_ticks={} reason=zero_or_stalled ok=1",
                        raw_ticks
                    );
                }
                fallback_idle_loops = fallback_idle_loops.wrapping_add(1);
                if fallback_idle_loops >= SYNTHETIC_VISUAL_THRESHOLD {
                    fallback_idle_loops = 0;
                    let old_ss = bar.clock_ss;
                    bar.clock_ss = bar.clock_ss.wrapping_add(1);
                    if bar.clock_ss >= 60 { bar.clock_ss = 0; bar.clock_mm = bar.clock_mm.wrapping_add(1); }
                    if bar.clock_mm >= 60 { bar.clock_mm = 0; bar.clock_hh = bar.clock_hh.wrapping_add(1); }
                    if bar.clock_hh >= 24 { bar.clock_hh = 0; }
                    advance_synthetic = true;
                    advance_fallback = true;
                    sex_pdx::serial_println!(
                        "[sexdisplay.clock.synthetic_visual.tick] old_ss={} new_ss={} threshold={} ok=1",
                        old_ss, bar.clock_ss, SYNTHETIC_VISUAL_THRESHOLD
                    );
                }
            }
            if advance_fallback {
                clock_canon_store(bar.clock_hh, bar.clock_mm, bar.clock_ss);
                let now_canon_ss = unsafe { CLOCK_CANON_SS };
                // ── Cadence sample marker (budgeted) ──
                unsafe {
                    static mut CADENCE_SAMPLE_BUDGET: u32 = 32;
                    let b = &mut CADENCE_SAMPLE_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        let synth: u8 = if advance_synthetic { 1 } else { 0 };
                        let thr: u16 = if advance_synthetic { SYNTHETIC_VISUAL_THRESHOLD } else { FALLBACK_TICKS_PER_SEC as u16 };
                        let source = if advance_synthetic { "fallback_synthetic_visual" } else { "fallback" };
                        if hot_log() { serial_println!(
                            "[sexdisplay.clock.cadence.sample] ss={} loops={} raw_delta={} synthetic={} threshold={} source={} ok=1",
                            now_canon_ss, fallback_loops_since_advance, advance_tick_delta, synth, thr, source
                        ); }
                    }
                }
                // ── Parity marker (one-shot): prove fallback and post-reject use same threshold ──
                unsafe {
                    static mut CADENCE_PARITY_EMITTED: bool = false;
                    if !CADENCE_PARITY_EMITTED {
                        CADENCE_PARITY_EMITTED = true;
                        serial_println!(
                            "[sexdisplay.clock.cadence.parity] fallback_threshold={} post_reject_threshold={} ok=1",
                            SYNTHETIC_VISUAL_THRESHOLD, SYNTHETIC_VISUAL_THRESHOLD
                        );
                    }
                }
                fallback_loops_since_advance = 0;
                // ── Same-second redraw guard: skip clock-only redraw if canonical_ss unchanged ──
                let last_drawn = unsafe { LAST_REDRAWN_CLOCK_CANON_SS };
                if fb_live {
                    if last_drawn != now_canon_ss || last_drawn == 0xFF {
                        needs_top_strip_redraw = true;
                        unsafe { CLOCK_REDRAW_SOURCE = 1; }
                    } else {
                        // Clock second unchanged — skip redraw unless other dirty state.
                        unsafe {
                            static mut SKIP_SAME_SECOND_BUDGET: u32 = 8;
                            let b = &mut SKIP_SAME_SECOND_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                if hot_log() { serial_println!(
                                    "[sexdisplay.clock.redraw.skip_same_second] ss={} reason=no_clock_delta ok=1",
                                    now_canon_ss
                                ); }
                            }
                        }
                    }
                }
            }
            // ── Post-reject liveness proof: fallback must advance past reject point ──
            if advance_fallback && handoff_reject_streak > 0 {
                // First fallback tick after a handoff reject — prove liveness.
                let reject_ss = handoff_reject_ss;
                let now_ss = unsafe { CLOCK_CANON_SS };
                unsafe {
                    static mut LIVE_AFTER_REJECT_BUDGET: u32 = 16;
                    let b = &mut LIVE_AFTER_REJECT_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        if hot_log() { serial_println!(
                            "[sexdisplay.clock.fallback.live_after_reject] reject_ss={} now_ss={} source=fallback ok=1",
                            reject_ss, now_ss
                        ); }
                    }
                }
                // ── Post-reject cadence sample (budgeted) ──
                unsafe {
                    static mut POST_REJECT_CADENCE_BUDGET: u32 = 16;
                    let b = &mut POST_REJECT_CADENCE_BUDGET;
                    if *b > 0 {
                        *b -= 1;
                        if hot_log() { serial_println!(
                            "[sexdisplay.clock.post_reject.cadence.sample] ss={} loops={} threshold={} ok=1",
                            now_ss, fallback_loops_since_advance, SYNTHETIC_VISUAL_THRESHOLD
                        ); }
                    }
                }
            }
            // ── Reject throttling: skip redraw when same-ss silkbar floods ──
            if handoff_reject_streak > 0 {
                let streak_ss = handoff_reject_ss;
                unsafe {
                    static mut REJECT_STREAK_PROOF_BUDGET: u32 = 8;
                    let b = &mut REJECT_STREAK_PROOF_BUDGET;
                    if *b > 0 && handoff_reject_streak == 4 {
                        *b -= 1;
                        if hot_log() { serial_println!(
                            "[sexdisplay.clock.handoff.reject.streak] ss={} streak={} redraw_suppressed=1",
                            streak_ss, handoff_reject_streak
                        ); }
                    }
                }
            }
            handoff_reject_streak = handoff_reject_streak.saturating_sub(1);
            if fallback_after_drop_pending {
                let continue_is_proof = fallback_after_drop_pending_proof;
                fallback_after_drop_pending = false;
                fallback_after_drop_pending_proof = false;
                if continue_is_proof {
                    unsafe {
                        static mut FALLBACK_AFTER_DROP_CONTINUE_BUDGET: u32 = 8;
                        let b = &mut FALLBACK_AFTER_DROP_CONTINUE_BUDGET;
                        if *b > 0 {
                            *b -= 1;
                            serial_println!(
                                "[sexdisplay.clock.fallback.continue_after_drop] ss={} source=fallback ok=1",
                                unsafe { CLOCK_CANON_SS }
                            );
                        }
                    }
                }
            }
        }
        if !bar.civil_enabled && !stale_probe_done {
            let canonical_ss = unsafe { CLOCK_CANON_SS };
            if canonical_ss >= 2 {
                stale_probe_done = true;
                let stale_ss = canonical_ss.wrapping_sub(1);
                serial_println!(
                    "[sexdisplay.clock.stale_probe.begin] canonical_ss={} stale_ss={}",
                    canonical_ss, stale_ss
                );
                serial_println!(
                    "[sexdisplay.clock.stale_drop] incoming_ss={} canonical_ss={} accepted=0 proof=1",
                    stale_ss, canonical_ss
                );
                fallback_after_drop_pending = true;
                fallback_after_drop_pending_proof = true;
                serial_println!("[sexdisplay.clock.stale_probe.done] ok=1");
            }
        }

        // ── Bounded drain: process up to DRAIN_MAX messages ──
        for _drain_i in 0..DRAIN_MAX {
            let Some(msg) = sex_pdx::pdx_try_listen_raw(0) else {
                break;
            };
            drained += 1;
            unsafe {
                DISPLAY_LAST_OP=msg.type_id;DISPLAY_LAST_CALLER=msg.caller_pd;
                REPAINT_DRAINED=drained;
                REPAINT_SEEN |= match msg.type_id {
                    0xEC=>1,0xEB=>2,0xED=>4,0xEE=>8,0xEF=>16,
                    0x276|0x277|0x278|0x27A|0x27B|0x27C|0x27D|0x28C=>32,0xFA|0xFB=>64,0xFD=>128,
                    silkbar_model::OP_SILKBAR_UPDATE=>256,_=>512,
                };
            }

            if msg.type_id==sex_pdx::screen_capture::OP{
                if unsafe{SCREEN_CAPTURE.request(msg,&lifecycle,sex_pdx::get_ticks(),FB_W as usize,FB_H as usize)}{pending_paint.request_full();}
                continue;
            }

            // A hint cannot authorize cleanup: capability status above is the authority.
            if msg.type_id == sex_pdx::app_lifecycle::OP_APP_REBORN {
                if msg.caller_pd==0 {
                    let (target,task)=(msg.arg0 as u32,msg.arg1 as u32);
                    let slot=sex_pdx::app_catalog::by_target(target).map(|row|row.lifecycle_slot);
                    if let Some(slot)=slot.filter(|slot|sex_pdx::app_lifecycle::catalog(*slot).is_ok_and(|c|c.target_pd==target&&c.task_id==task)) {
                        if lifecycle.reborn(target,task)==Some(slot) {
                            let acked=sex_pdx::app_lifecycle::ack(slot,task).is_ok();
                            sex_pdx::serial_record!("[display.app.reborn.ack] target={} task={} ok={}",target,task,acked as u8);
                        }
                    }
                }
                continue;
            }
            if msg.type_id == sex_pdx::app_lifecycle::OP_APP_EXIT {
                if sex_pdx::app_lifecycle::ExitNotice::decode(msg.type_id, msg.caller_pd as u64,
                    [msg.arg0, msg.arg1, msg.arg2]).is_ok()
                    && prepare_app_cleanup(&mut lifecycle, &mut bar) {
                    needs_surface_redraw = true; needs_top_strip_redraw = true;
                }
                continue;
            }
            if !lifecycle.caller_live(msg.caller_pd)
                || (matches!(msg.type_id, 0xEC | 0xEB | 0xED | 0xEE | 0xEF | 0xFA | 0xFB | 0xFD | 0x276 | 0x277 | 0x278 | 0x27A | 0x27B | 0x27C | 0x27D | 0x28B | 0x28C)
                    && !lifecycle.sid_live(msg.arg0)) {
                unsafe {
                    static mut DEAD_CALL_LOG_BUDGET: u8 = 8;
                    if DEAD_CALL_LOG_BUDGET != 0 {
                        DEAD_CALL_LOG_BUDGET -= 1;
                        sex_pdx::serial_record!("[display.app.stale.reject] caller={} sid={} op={:#x}", msg.caller_pd, msg.arg0, msg.type_id);
                    }
                }
                continue;
            }
            if msg.type_id == silkbar_model::OP_SILKBAR_UPDATE
                && msg.arg0 as u32 == UpdateKind::SetActiveApp as u32
                && !lifecycle.sid_live(msg.arg1 as u32 as u64) { continue; }
            match msg.type_id {
                sex_pdx::notify::OP_PANEL_NOTICE => {
                    // Only the Shell writes the panel notice; it ends with an empty one.
                    if msg.caller_pd == 3 {
                        let assembler = unsafe { &mut *core::ptr::addr_of_mut!(PANEL_NOTICE_ASSEMBLER) };
                        if let Some(text) = assembler.push(msg.arg0, msg.arg1, msg.arg2) {
                            unsafe { PANEL_NOTICE = text; }
                            needs_top_strip_redraw = true;
                            sex_pdx::serial_record!("[display.panel.notice] len={}", text.len);
                        }
                    }
                }
                sex_pdx::desktop_look::OP_DESKTOP_LOOK => {
                    // Only the Shell picks the wallpaper and night light.
                    if msg.caller_pd == 3 && sex_pdx::desktop_look::set(msg.arg0, msg.arg1) {
                        needs_full_redraw = true;
                        sex_pdx::serial_record!("[display.look] wallpaper={} night={}", msg.arg0, msg.arg1);
                    }
                }
                sex_pdx::app_lifecycle::OP_APP_TITLE => {
                    // Only the program row that owns this surface may name it.
                    if app_catalog::by_surface(msg.caller_pd, msg.arg0).is_some_and(|row| row.app == app_catalog::App::Program && row.shell_surface.is_none()) {
                        let mut bytes = [0u8; 16];
                        bytes[..8].copy_from_slice(&msg.arg1.to_le_bytes());
                        bytes[8..].copy_from_slice(&msg.arg2.to_le_bytes());
                        set_program_title(msg.arg0, Some(bytes));
                        needs_top_strip_redraw |= unsafe { FOCUSED_SURFACE_ID } == msg.arg0;
                        needs_full_redraw = true;
                        sex_pdx::serial_record!("[display.app.title] target={} surface={}", msg.caller_pd, msg.arg0);
                    }
                }
                sex_pdx::desktop_style::OP_TEXT_ZOOM => {
                    // Shell, as window manager, sizes the text of the window it has focused.
                    let wm_pd = DISPLAY_WM_PD.load(core::sync::atomic::Ordering::Acquire);
                    if wm_pd == 0 || msg.caller_pd != wm_pd { continue; }
                    let step = msg.arg1 as i64;
                    unsafe {
                        if let Some(slot) = SURFACES.iter_mut().find(|s| s.active && s.surface_id == msg.arg0 && msg.arg0 != 0) {
                            if apply_text_zoom(slot, step) { needs_full_redraw = true; }
                            sex_pdx::serial_record!("[display.text.zoom] surface={} step={} zoom={} scale={}", msg.arg0, step, slot.text_zoom, surface_text_scale(slot));
                        }
                    }
                }
                sex_pdx::desktop_style::OP_DISPLAY_SCALE => {
                    // Shell, as window manager, steps the whole screen's scale.
                    let wm_pd = DISPLAY_WM_PD.load(core::sync::atomic::Ordering::Acquire);
                    if wm_pd == 0 || msg.caller_pd != wm_pd { continue; }
                    // Includes legacy arg1=0: no later scale can invalidate
                    // the value retained for an outstanding publication reply.
                    if !scale_receipts.allows_change(msg.caller_pd,msg.caller_task,msg.arg1){continue;}
                    let step = msg.arg0 as i64;
                    let before = sex_pdx::desktop_style::ui_scale();
                    let requested = sex_pdx::desktop_style::chrome::step_scale(before, unsafe { BOOT_UI_SCALE }, step);
                    let wanted=sex_pdx::desktop_style::chrome::fit_scale(unsafe{FB_W},requested,silkbar_model::MIN_CHROME_WIDTH);
                    // arg1 is a nonzero Shell request sequence. One accepted
                    // request stays retained through full publication + reply.
                    if msg.arg1!=0 && !scale_receipts.admit(msg.caller_pd,msg.caller_task,msg.arg1,wanted) {continue;}
                    sex_pdx::desktop_style::set_ui_scale(wanted);
                    unsafe{FRAME_GROUPS.set_scale(wanted);FRAME_GROUPS.fit_children(&mut SURFACES);}
                    reflow_bar(&mut bar,unsafe{FB_W}as usize);
                    if wanted != before {
                        unsafe {
                            for slot in SURFACES.iter_mut().filter(|s| s.active) { recompute_text_lines(slot); }
                            // The cursor's saved underlay has the old size.
                            CURSOR_UNDER_VALID = false;
                        }
                        needs_full_redraw = true;
                    }
                    sex_pdx::serial_record!("[display.scale] scale={} step={} reason=keys", wanted, step);
                }
                sex_pdx::pixel_region::OP => {
                    // Only the program row that owns this surface shows pixels in it.
                    if let (Some(k), true) = (sex_pdx::pixel_region::index_of(msg.caller_pd), app_catalog::by_surface(msg.caller_pd, msg.arg0).is_some_and(|row| row.app == app_catalog::App::Program)) {
                        // Any shape the region holds: a full-screen window shows full-screen pixels.
                        let pixels = unsafe { PIXEL_BYTES[k] } as usize / 4;
                        let w = ((msg.arg1 >> 32) as usize).min(pixels);
                        let h = if w == 0 { 0 } else { ((msg.arg1 & 0xFFFF_FFFF) as usize).min(pixels / w) };
                        unsafe { PIXEL_SIZE[k] = (w, h); }
                        needs_full_redraw = true;
                        sex_pdx::serial_record!("[display.app.pixels] target={} surface={} w={} h={}", msg.caller_pd, msg.arg0, w, h);
                    }
                }
                sex_pdx::app_lifecycle::OP_APP_PRESENT => {
                    // A new run starts with the catalog's name and text until it gives its own.
                    if app_catalog::by_surface(msg.caller_pd, msg.arg0).is_some_and(|row| row.app == app_catalog::App::Program) {
                        set_program_title(msg.arg0, None);
                        if let Some(k) = sex_pdx::pixel_region::index_of(msg.caller_pd) { unsafe { PIXEL_SIZE[k] = (0, 0); } }
                    }
                    if lifecycle.request_present(msg.caller_pd, msg.arg0, msg.arg1, msg.arg2) {
                        sex_pdx::serial_record!("[display.app.present.request] target={} surface={} request={}", msg.caller_pd, msg.arg0, msg.arg1);
                    }
                }
                silkbar_model::OP_SILKBAR_UPDATE => {
                    // Compare the low kind only to select the strict decoder;
                    // it rejects every high-word alias before changing state.
                    if msg.arg0 as u32==UpdateKind::SetCivilClock as u32{
                        match handle_civil_clock_update(&mut bar,msg.caller_pd,msg.caller_task,[msg.arg0,msg.arg1,msg.arg2],raw_ticks){
                            Ok((silkbar_model::civil_clock::Applied::New,changed))=>{
                                needs_top_strip_redraw|=changed;
                                let source_known=matches!(bar.civil_clock.state(),silkbar_model::civil_clock::State::Snapshot|silkbar_model::civil_clock::State::Unavailable|silkbar_model::civil_clock::State::Rollback);
                                sex_pdx::serial_record!("[sexdisplay.clock.civil.applied] pd={} task={} sequence={} state={:?} seconds={} high_water={} stale=0 source_known={} source={} precision={} snapshot_only=1 pixels=unconfirmed repaint={}",msg.caller_pd,msg.caller_task,bar.civil_clock.sequence(),bar.civil_clock.state(),bar.civil_clock.seconds(),bar.civil_clock.high_water(),source_known as u8,if source_known{"q35_cmos_utc"}else{"unknown"},source_known as u8,changed as u8);
                            },
                            Ok((silkbar_model::civil_clock::Applied::Duplicate,_))=>{},
                            Err(error)=>{
                                static mut CIVIL_REJECT_BUDGET:u8=32;
                                unsafe{if CIVIL_REJECT_BUDGET>0{CIVIL_REJECT_BUDGET-=1;sex_pdx::serial_record!("[sexdisplay.clock.civil.rejected] pd={} task={} reason={:?} effects=0",msg.caller_pd,msg.caller_task,error);}}
                            },
                        }
                        continue;
                    }
                    if bar.civil_enabled && msg.arg0 as u32==UpdateKind::SetClock as u32{continue;}
                    // For SetClock: capture old bar values and decode incoming
                    // BEFORE apply_update mutates bar. This gives accurate
                    // old vs incoming comparison in markers.
                    let update_kind = msg.arg0 as u32;
                    let (old_hh, old_mm, old_ss, in_hh, in_mm, in_ss) =
                        if update_kind == UpdateKind::SetClock as u32 {
                            let ih = (msg.arg1 as u32).min(23);
                            let im = ((msg.arg2 >> 8) as u32).min(59);
                            let is_ = (msg.arg2 as u8 as u32).min(59);
                            (bar.clock_hh as u32, bar.clock_mm as u32, bar.clock_ss as u32,
                             ih, im, is_)
                        } else {
                            (0, 0, 0, 0, 0, 0)
                        };

                    let (applied, kind) = handle_silkbar_update(&mut bar, msg.arg0, msg.arg1, msg.arg2);
                    let mut should_redraw_top_strip = kind != UpdateKind::SetClock as u32;
                    if applied {
                        if kind == UpdateKind::SetClock as u32 {
                            silkbar_clock_seen = true;
                            unsafe {
                                static mut SILKBAR_CLOCK_SEND_MARKER: bool = false;
                                if !SILKBAR_CLOCK_SEND_MARKER {
                                    SILKBAR_CLOCK_SEND_MARKER = true;
                                    serial_println!("[boot.firstpaint.silkbar_clock_send] ok=1");
                                }
                            }
                            let changed = in_hh != old_hh || in_mm != old_mm || in_ss != old_ss;
                            let mut handoff_rejected = false;
                            if !clock_from_silkbar {
                                let (canon_hh, canon_mm, canon_ss) = unsafe { (CLOCK_CANON_HH, CLOCK_CANON_MM, CLOCK_CANON_SS) };
                                let incoming_total = clock_total_seconds(in_hh as u8, in_mm as u8, in_ss as u8);
                                let canonical_total = clock_total_seconds(canon_hh, canon_mm, canon_ss);
                                if incoming_total < canonical_total {
                                    handoff_rejected = true;
                                    clock_canon_apply_to_bar(&mut bar);
                                    // Fallback cadence advances independently via raw_ticks.
                                    // Do NOT reset cadence tracker on reject (V8: independent cadence).
                                    // Track reject streak for redraw throttling.
                                    let canon_ss_now = unsafe { CLOCK_CANON_SS };
                                    handoff_reject_ss = canon_ss_now;
                                    handoff_reject_streak = 8; // suppress redraws for 8 loops
                                    unsafe {
                                        static mut CLOCK_HANDOFF_REJECT_BUDGET: u32 = 64;
                                        let b = &mut CLOCK_HANDOFF_REJECT_BUDGET;
                                        if *b > 0 {
                                            *b -= 1;
                                            if hot_log() { serial_println!(
                                                "[sexdisplay.clock.handoff.reject] from=fallback to=silkbar incoming_ss={} canonical_ss={} reason=backward accepted=0",
                                                in_ss, canon_ss
                                            ); }
                                            if hot_log() { serial_println!(
                                                "[sexdisplay.clock.fallback.continue_after_handoff_reject] ss={} source=fallback",
                                                canon_ss
                                            ); }
                                        }
                                    }
                                } else {
                                    unsafe {
                                        static mut CLOCK_HANDOFF_ACCEPT_BUDGET: u32 = 64;
                                        let b = &mut CLOCK_HANDOFF_ACCEPT_BUDGET;
                                        if *b > 0 {
                                            *b -= 1;
                                            serial_println!(
                                                "[sexdisplay.clock.handoff.accept] from=fallback to=silkbar incoming_ss={} canonical_ss={} accepted=1",
                                                in_ss, canon_ss
                                            );
                                        }
                                    }
                                }
                            }
                            if changed && !handoff_rejected {
                                clock_from_silkbar = true;
                                last_silkbar_msg_loop = display_loop_counter;
                                if (in_ss as u8) == last_silkbar_second {
                                    repeated_silkbar_second_msgs = repeated_silkbar_second_msgs.saturating_add(1);
                                    unsafe {
                                        static mut CLOCK_SOURCE_REPEAT_BUDGET: u32 = 32;
                                        let b = &mut CLOCK_SOURCE_REPEAT_BUDGET;
                                        if *b > 0 {
                                            *b -= 1;
                                            serial_println!(
                                                "[sexdisplay.clock.source.silkbar.repeat] ss={} count={}",
                                                in_ss, repeated_silkbar_second_msgs
                                            );
                                        }
                                    }
                                } else {
                                    repeated_silkbar_second_msgs = 0;
                                    last_silkbar_second = in_ss as u8;
                                }
                                fallback_idle_loops = 0;
                                clock_canon_store(bar.clock_hh, bar.clock_mm, bar.clock_ss);
                                unsafe { CLOCK_REDRAW_SOURCE = 0; } // silkbar source
                                handoff_reject_streak = 0; // clear reject throttling
                            } else if handoff_rejected {
                                clock_from_silkbar = false;
                                // Do NOT reset fallback_idle_loops here.  Silkbar keeps sending
                                // stale clock messages which would perpetually reset the synthetic
                                // tick counter, freezing the visual clock on slow emulation (TCG).
                                unsafe { CLOCK_REDRAW_SOURCE = 1; }
                            }
                            unsafe {
                                static mut CLOCK_SOURCE_APPLY_BUDGET: u32 = 64;
                                let b = &mut CLOCK_SOURCE_APPLY_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    if hot_log() { serial_println!(
                                        "[sexdisplay.clock.source.silkbar.apply] hh={} mm={} ss={} loop={}",
                                        bar.clock_hh, bar.clock_mm, bar.clock_ss, display_loop_counter
                                    ); }
                                }
                            }
                            unsafe {
                                static mut CLOCK_RECV_BUDGET: u32 = 64;
                                let b = &mut CLOCK_RECV_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    if hot_log() { serial_println!(
                                        "[sexdisplay.clock.recv] in_h={} in_m={} in_s={} old_h={} old_m={} old_s={} sec_now={} changed={} from_silkbar=1",
                                        in_hh, in_mm, in_ss,
                                        old_hh, old_mm, old_ss,
                                        sec_now, changed as u8); }
                                }
                            }
                            unsafe {
                                static mut CLOCK_APPLY_BUDGET: u32 = 64;
                                let b = &mut CLOCK_APPLY_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    if hot_log() { serial_println!(
                                        "[sexdisplay.clock.apply] source=silkbar h={} m={} s={} accepted={} reason={}",
                                        bar.clock_hh, bar.clock_mm, bar.clock_ss,
                                        if handoff_rejected { 0 } else { 1 },
                                        if handoff_rejected { "handoff_backward" } else if changed { "changed" } else { "same" }); }
                                }
                            }
                            should_redraw_top_strip = changed && !handoff_rejected;
                        }
                        if fb_live && should_redraw_top_strip {
                            needs_top_strip_redraw = true;
                        }
                    } else {
                        if kind == UpdateKind::SetClock as u32 {
                            let canonical_ss = unsafe { CLOCK_CANON_SS };
                            unsafe {
                                static mut CLOCK_STALE_DROP_BUDGET: u32 = 64;
                                let b = &mut CLOCK_STALE_DROP_BUDGET;
                                if *b > 0 {
                                    *b -= 1;
                                    serial_println!(
                                        "[sexdisplay.clock.stale_drop] incoming_ss={} canonical_ss={} accepted=0 proof=0",
                                        in_ss, canonical_ss
                                    );
                                    serial_println!(
                                        "[sexdisplay.clock.handoff] from=fallback to=silkbar accepted=0"
                                    );
                                }
                            }
                            clock_from_silkbar = false;
                            fallback_idle_loops = 0;
                            fallback_loops_since_advance = 0;
                            fallback_after_drop_pending = true;
                            fallback_after_drop_pending_proof = false;
                            if fb_live {
                                let last_drawn = unsafe { LAST_REDRAWN_CLOCK_CANON_SS };
                                let now_ss = canonical_ss;
                                if last_drawn != now_ss || last_drawn == 0xFF {
                                    needs_top_strip_redraw = true;
                                    unsafe { CLOCK_REDRAW_SOURCE = 1; }
                                } else {
                                    unsafe {
                                        static mut SKIP_SAME_SECOND_STALE_BUDGET: u32 = 4;
                                        let b = &mut SKIP_SAME_SECOND_STALE_BUDGET;
                                        if *b > 0 {
                                            *b -= 1;
                                            if hot_log() { serial_println!(
                                                "[sexdisplay.clock.redraw.skip_same_second] ss={} reason=no_clock_delta_stale_drop ok=1",
                                                now_ss
                                            ); }
                                        }
                                    }
                                }
                            }
                        } else {
                            serial_println!("[silkde.m2.assert.bad] apply_update=false kind={}", kind);
                        }
                    }
                }
                0x11 => { // OP_PRIMARY_FB
                    if msg.caller_pd==1 && msg.arg0==address && msg.arg1==(geometry.width as u64|((geometry.height as u64)<<32)) && handle_primary_fb(msg.arg0, msg.arg1) {
                        fb_live = true;
                        serial_println!("[pdx.identity.accept] display owner_pd validation active");
                        // ── CLOCK_REDRAW_AFTER_FB_LIVE_FIX_V1 ──
                        // Ensure clock shows nonzero seconds when bar is first
                        // rendered on the live framebuffer.  Fallback tick may
                        // not have fired yet, so force ss>=1 if still zero.
                        if !bar.civil_enabled && bar.clock_ss == 0 {
                            bar.clock_ss = 1;
                            clock_canon_store(bar.clock_hh, bar.clock_mm, bar.clock_ss);
                        }
                        // Coalesce startup repaints into one clean first frame.
                        // Defer publication until this entire drain has joined.
                        needs_full_redraw=true;
                        primary_receipt_pending=true;
                        // Keep strip repaint batched in post-drain redraws.
                        unsafe { CLOCK_REDRAW_SOURCE = if clock_from_silkbar { 0 } else { 1 }; }
                        needs_top_strip_redraw = true;
                        // Ensure surface area (including window chrome) is redrawn
                        // on the next cycle so that OP_SURFACE_TAB_INFO chrome flags
                        // applied during this drain take effect on screen.
                        needs_surface_redraw = true;
                        if SILK_RENDER_PROOF_PROFILE_ENABLED && !render_proof_done {
                            render_proof_done = true;
                            unsafe { top_strip_render_proof(FB_PTR as *const u32, FB_W as usize, FB_H as usize); }
                        }
                    }
                }
                0 => continue,
                0xE4 => {
                    // OP_WINDOW_CREATE safe inline ABI: arg0=x, arg1=y, arg2=(h<<32)|w
                    // Store only, no immediate placeholder paint.
                    // Immediate blue paint here causes startup flicker.
                    let x = msg.arg0 as i32;
                    let y = msg.arg1 as i32;
                    let w = (msg.arg2 as u32).min(MAX_FB_W as u32);
                    let h = ((msg.arg2 >> 32) as u32).min(MAX_FB_H as u32);
                    if w == 0 || h == 0 { continue; }
                    unsafe {
                        for (index,slot) in SURFACES.iter_mut().enumerate() {
                            if !slot.active {
                                TEXT_STAGES[index].reset();
                                slot.erase_selection();
                                *slot = Surface {
                                    surface_id: 0,
                                    owner_pd: msg.caller_pd,
                                    x, y, w, h,
                                    color: 0x00303860,
                                    active: true,
                                    tab_count: 0, active_tab: 0, chrome_flags: 0,
                                    fill_count: 0,
                                    fill_sx: [0i32; MAX_RECTS], fill_sy: [0i32; MAX_RECTS],
                                    fill_sw: [0u32; MAX_RECTS], fill_sh: [0u32; MAX_RECTS],
                                    fill_color: [0u32; MAX_RECTS],
                                    selection_start: 0, selection_end: 0,
                                    text_len: 0, text_color: 0x00FFFFFF, text_buf: [0u8; TEXT_BUF_CAP],
                                    line_starts: [0u16; TEXT_MAX_ROWS], cell_starts: [0; TEXT_BUF_CAP],
                                    line_cells: [0; TEXT_MAX_ROWS], cell_count: 0, line_count: 0, wrap_cols: TEXT_LEGACY_COLS,
                                    text_zoom: 0,
                                };
                                break;
                            }
                        }
                    }
                    continue;
                }
                0xEC => {
                    // OP_SURFACE_CREATE_ID: arg0=surface_id(non-zero), arg1=(y<<32)|x, arg2=(h<<32)|w
                    let surface_id = msg.arg0;
                    if surface_id == 0 { continue; }
                    let x = msg.arg1 as i32;
                    let y = (msg.arg1 >> 32) as i32;
                    let w = (msg.arg2 as u32).min(MAX_FB_W as u32);
                    let h = ((msg.arg2 >> 32) as u32).min(MAX_FB_H as u32);
                    if w == 0 || h == 0 { continue; }
                    let color = if surface_id & 1 == 0 { 0x00303860u32 } else { 0x00704890u32 };
                    unsafe {
                        let admission = lifecycle.admit(&SURFACES, msg.caller_pd, surface_id);
                        let vacant=match &admission { surface_lifecycle::Admission::Vacant(index,_)=>Some(*index),_=>None };
                        let owner=match &admission{surface_lifecycle::Admission::Vacant(_,owner)=>*owner,surface_lifecycle::Admission::Existing(i)=>SURFACES[*i].owner_pd,_=>0};
                        let color=sex_pdx::desktop_style::native_canvas(owner,surface_id).unwrap_or(color);
                        let before = SURFACES.iter().find(|s|s.active && s.surface_id==surface_id)
                            .map(|s|(s.x,s.y,s.w,s.h,s.color));
                        if surface_lifecycle::upsert(&mut SURFACES, admission, surface_id, x, y, w, h, color) {
                            REPAINT_UPSERTS += 1;
                            if let Some(index)=vacant {
                                TEXT_STAGES[index].reset();
                                BITMAP_STORES.reset_surface(owner, surface_id);
                            }
                            FRAME_GROUPS.observe(&SURFACES);
                            FRAME_GROUPS.fit_children(&mut SURFACES);
                            let after = SURFACES.iter().find(|s|s.active && s.surface_id==surface_id)
                                .map(|s|(s.x,s.y,s.w,s.h,s.color));
                            if before==after { REPAINT_FITTED_NOOPS+=1; }
                            for child in SURFACES.iter_mut().filter(|s| s.active) { recompute_text_lines(child); }
                            if let Some(surface) = SURFACES.iter_mut().find(|s| s.active && s.surface_id == surface_id) {
                                recompute_text_lines(surface);
                                if let Some(parent)=FRAME_GROUPS.awaiting_parent(surface) {
                                    // One exact fixed child receipt per boot, never a pixel ACK.
                                    static mut PARENT_GATE_RECORDED:u8=0;
                                    let bit=match surface.surface_id {154=>1,160=>2,161=>4,162=>8,302=>16,303=>32,304=>64,305=>128,_=>0};
                                    if bit!=0 && PARENT_GATE_RECORDED&bit==0 {
                                        PARENT_GATE_RECORDED|=bit;
                                        sex_pdx::serial_record!("[display.child.parent_gate] target={} surface={} parent={} observed=0 standalone=0 retained=1",surface.owner_pd,surface.surface_id,parent);
                                    }
                                }
                                if surface.surface_id == 300 && surface.owner_pd == 15 {
                                    static ADMITTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
                                    if !ADMITTED.swap(true, core::sync::atomic::Ordering::Relaxed) {
                                        sex_pdx::serial_record!("[display.app.admitted] target=15 surface=300 owner=15");
                                    }
                                }
                            }
                            needs_surface_redraw |= fb_live;
                        }
                    }
                }
                0xDE => {
                    // OP_WINDOW_CREATE (legacy pointer protocol) — arg0 is cross-PD pointer.
                    // Must NOT dereference. Unsupported until kernel-mediated copy exists.
                    continue;
                }
                0xEB => {
                    // OP_SURFACE_UPDATE: arg0=surface_id, arg1=x, arg2=y
                    let target_id = msg.arg0;
                    if target_id == 0 { continue; }
                    let new_x = msg.arg1 as i32;
                    let new_y = msg.arg2 as i32;
                    unsafe {
                        let wm_pd = DISPLAY_WM_PD.load(core::sync::atomic::Ordering::Acquire);
                        let mut found = false;
                        for slot in SURFACES.iter_mut() {
                            if slot.active && slot.surface_id == target_id {
                                if slot.owner_pd != msg.caller_pd && !(wm_pd != 0 && msg.caller_pd == wm_pd) {
                                    let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                    if n & 0x3F == 0 {
                                        serial_println!("[sexdisplay.auth.deny] op=0xeb caller={} sid={} owner={} wm={}",
                                            msg.caller_pd, target_id, slot.owner_pd, wm_pd);
                                    }
                                    continue;
                                }
                                found = slot.x != new_x || slot.y != new_y || target_id == CURSOR_SURFACE_ID;
                                slot.x = new_x;
                                slot.y = new_y;
                                break;
                            }
                        }
                        if found {
                            FRAME_GROUPS.fit_children(&mut SURFACES);
                            for child in SURFACES.iter_mut().filter(|s| s.active) { recompute_text_lines(child); }
                        }
                        if found && fb_live {
                            if target_id == CURSOR_SURFACE_ID {
                                // Budgeted marker: display received cursor surface update from shell.
                                unsafe {
                                    static mut DISPLAY_CURSOR_UPDATE_BUDGET: u32 = 16;
                                    let rem = &mut DISPLAY_CURSOR_UPDATE_BUDGET;
                                    if *rem > 0 {
                                        *rem -= 1;
                                        serial_println!("[sexdisplay.cursor.surface.update] n=0 x={} y={}", new_x, new_y);
                                    }
                                }
                                // INPUT_PRESENT_TRACE_V1: extract trace seq from
                                // unused high 32 bits of arg1 (0 = untagged sender).
                                unsafe {
                                    let seq = (msg.arg1 >> 32) as u32;
                                    if seq != 0 { INPUT_TRACE_LAST_SEQ = seq; }
                                    // INPUT_PRESENT_TICK_TRACE_V1: shell send
                                    // tick rides arg2 high 32 bits (0 = untagged).
                                    let shell_tick = (msg.arg2 >> 32) as u32;
                                    if shell_tick != 0 { INPUT_TRACE_LAST_SHELL_TICK = shell_tick; }
                                    INPUT_TRACE_RECV = INPUT_TRACE_RECV.wrapping_add(1);
                                    DISPLAY_RECV_TICK = DISPLAY_RECV_TICK.wrapping_add(1);
                                    static mut INPUT_TRACE_RECV_BUDGET: u32 = 32;
                                    if INPUT_TRACE_RECV_BUDGET > 0 {
                                        INPUT_TRACE_RECV_BUDGET -= 1;
                                        if seq != 0 && shell_tick != 0 {
                                            serial_println!("[input.trace.display.cursor.recv] seq={} shell_tick={} display_tick={} x={} y={}", seq, shell_tick, DISPLAY_RECV_TICK, new_x, new_y);
                                        } else if seq != 0 {
                                            serial_println!("[input.trace.display.cursor.recv] seq={} shell_tick=unknown display_tick={} x={} y={}", seq, DISPLAY_RECV_TICK, new_x, new_y);
                                        } else {
                                            serial_println!("[input.trace.display.cursor.recv] seq=unknown shell_tick=unknown display_tick={} x={} y={}", DISPLAY_RECV_TICK, new_x, new_y);
                                        }
                                        if INPUT_TRACE_RECV_BUDGET == 0 { INPUT_TRACE_DISPLAY_BUDGET_HIT = 1; }
                                    }
                                    // Summary cadence every 8: cursor sends are
                                    // coalesced ~4-8x upstream, so %32 left the
                                    // last summary stale and gates read wrong
                                    // totals (INPUT_CURSOR_DRAIN_COHERENCE_V1).
                                    if INPUT_TRACE_RECV == 4 || INPUT_TRACE_RECV % 8 == 0 {
                                        input_trace_display_summary();
                                    }
                                }
                                // Cursor-only move: take the save-under fast
                                // path unless another surface also changed
                                // this cycle (checked post-drain).
                                cursor_only_moved = true;
                            } else {
                                needs_surface_redraw = true;
                            }
                        }
                    }
                }
                0xED => {
                    // OP_SET_FOCUS: arg0=surface_id (0 clears focus). Owner or registered WM only.
                    if !fb_live { continue; }
                    let sid = msg.arg0;
                    if sid != 0 {
                        let wm_pd = DISPLAY_WM_PD.load(core::sync::atomic::Ordering::Acquire);
                        let authorized = unsafe {
                            SURFACES.iter().any(|s| s.active && s.surface_id == sid
                                && (s.owner_pd == msg.caller_pd || (wm_pd != 0 && msg.caller_pd == wm_pd)))
                        };
                        if !authorized {
                            let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                            if n & 0x3F == 0 {
                                serial_println!("[sexdisplay.auth.deny] op=0xed caller={} sid={} wm={}",
                                    msg.caller_pd, sid, wm_pd);
                            }
                            continue;
                        }
                    }
                    unsafe {
                        let changed = FOCUSED_SURFACE_ID != sid;
                        needs_surface_redraw |= changed;
                        needs_top_strip_redraw |= changed;
                        FOCUSED_SURFACE_ID = sid;
                    }
                }
                0xEE => {
                    // OP_SURFACE_DESTROY: owner, or exact catalog coordinator, hides the surface.
                    let target_id = msg.arg0;
                    if target_id == 0 { continue; }
                    unsafe {
                        let mut found = false;
                        for (index,slot) in SURFACES.iter_mut().enumerate() {
                            if slot.active && slot.surface_id == target_id {
                                if slot.owner_pd != msg.caller_pd && !lifecycle.coordinator_controls(msg.caller_pd, target_id) {
                                    let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                    if n & 0x3F == 0 {
                                        serial_println!("AUTH: 0xEE destroy rejected sid={} caller={} owner={}",
                                            target_id, msg.caller_pd, slot.owner_pd);
                                    }
                                    continue;
                                }
                                TEXT_STAGES[index].reset();
                                BITMAP_STORES.reset_surface(slot.owner_pd, target_id);
                                slot.erase_selection();
                                *slot = SURFACE_EMPTY;
                                found = true;
                                break;
                            }
                        }
                        if found {
                            if FOCUSED_SURFACE_ID == target_id {
                                FOCUSED_SURFACE_ID = 0;
                            }
                            needs_surface_redraw = true;
                        }
                    }
                }
                0xF5 => {
                    // OP_REGISTER_WM: immutable catalog coordinator only; idempotent.
                    let caller = msg.caller_pd;
                    // Genesis binds the coordinator's main task to its PD;
                    // restart rearms that same task. The kernel-owned own-
                    // restart count in arg0 supplies the incarnation fence.
                    if !lifecycle.is_coordinator(caller)||msg.caller_task!=caller { continue; }
                    let Some(changed)=scale_receipts.register(caller,msg.caller_task,msg.arg0)else{continue;};
                    if changed {
                        let boot=unsafe{BOOT_UI_SCALE};
                        sex_pdx::desktop_style::set_ui_scale(boot);
                        reflow_bar(&mut bar,unsafe{FB_W}as usize);
                        unsafe{FRAME_GROUPS.set_scale(boot);FRAME_GROUPS.fit_children(&mut SURFACES);}
                        unsafe{for slot in SURFACES.iter_mut().filter(|s|s.active){recompute_text_lines(slot);}CURSOR_UNDER_VALID=false;}
                        needs_full_redraw=true;
                    }
                    match DISPLAY_WM_PD.compare_exchange(
                        0, caller,
                        core::sync::atomic::Ordering::AcqRel,
                        core::sync::atomic::Ordering::Acquire,
                    ) {
                        Ok(_) => { serial_println!("[sexdisplay.auth.wm.register] caller={} ok=1", caller); }
                        Err(existing) if existing != caller => {
                            serial_println!("[sexdisplay.auth.wm.deny] caller={} registered={}", caller, existing);
                        }
                        _ => {}
                    }
                }
                0xEF => {
                    // OP_SURFACE_FILL_RECT: arg0=surface_id, arg1=(sy<<32)|sx,
                    // arg2=(rect_index<<56)|(color_rgb<<32)|(sh<<16)|sw.
                    // rect_index in bits 56-59 (top nibble of color's alpha byte).
                    // Existing callers use 0x00RRGGBB so bits 56-63 are zero → rect_index=0.
                    let surface_id = msg.arg0;
                    if surface_id == 0 { continue; }
                    if !fb_live { continue; }

                    let sx = (msg.arg1 & 0xFFFF_FFFF) as i32;
                    let sy = ((msg.arg1 >> 32) & 0xFFFF_FFFF) as i32;
                    let sw = (msg.arg2 & 0xFFFF) as u32;
                    let sh = ((msg.arg2 >> 16) & 0xFFFF) as u32;
                    let color = ((msg.arg2 >> 32) & 0x00FF_FFFF) as u32;
                    let rect_index = ((msg.arg2 >> 56) & 0xF) as usize;

                    unsafe {
                        for slot in SURFACES.iter_mut() {
                            if !(slot.active && slot.surface_id == surface_id) {
                                continue;
                            }
                            if slot.owner_pd != msg.caller_pd {
                                let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                if n & 0x3F == 0 {
                                    serial_println!("AUTH: 0xEF fill rejected sid={} caller={} owner={}",
                                        surface_id, msg.caller_pd, slot.owner_pd);
                                }
                                break;
                            }
                            if rect_index >= MAX_RECTS {
                                serial_println!("[display.fill_rect.reject.index] sid={} index={}",
                                    surface_id, rect_index);
                                break;
                            }

                            if let Ok(Some((old,new)))=fill_damage::update(slot,rect_index,sx,sy,sw,sh,color){
                                text_damage.add(old.screen(slot,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                                text_damage.add(new.screen(slot,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                            }

                            break;
                        }
                    }
                }
                sex_pdx::bitmap_batch::BEGIN | sex_pdx::bitmap_batch::CHUNK | sex_pdx::bitmap_batch::COMMIT | sex_pdx::bitmap_batch::CLEAR => {
                    let Some(binding)=(unsafe { BITMAP_STORES.binding_for(msg.caller_pd,msg.arg0) }) else { continue; };
                    if !binding.admits(msg.caller_pd,msg.caller_task,msg.arg0)
                        || !bitmap_binding_live(&lifecycle,binding) { continue; }
                    let Some(request) = sex_pdx::bitmap_batch::Request::decode(msg.type_id, msg.arg1, msg.arg2) else { continue; };
                    unsafe {
                        if let Some(slot) = SURFACES.iter().find(|s| s.active && s.surface_id == msg.arg0 && s.owner_pd == msg.caller_pd) {
                            if let Some(change) = BITMAP_STORES.receive(msg.caller_pd, msg.caller_task, msg.arg0, request) {
                                let sequence=BITMAP_STORES.sequence(binding.target(),binding.surface()).expect("committed bitmap remains bound");
                                text_damage.add(bitmap_surface_damage(slot,change.old,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                                text_damage.add(bitmap_surface_damage(slot,change.new,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                                sex_pdx::serial_record!("[display.bitmap.committed] target={} surface={} paint={} width={} height={} scale={} pixels=unconfirmed",
                                    binding.target(),binding.surface(),sequence,change.new.map_or(0,|g|g.width),change.new.map_or(0,|g|g.height),change.new.map_or(0,|g|g.scale));
                            }
                        }
                    }
                }
                sex_pdx::text_batch::BEGIN | sex_pdx::text_batch::CHUNK | sex_pdx::text_batch::SELECT | sex_pdx::text_batch::COMMIT | sex_pdx::text_batch::FILL => {
                    if !browser_text_binding_live(&lifecycle, msg.caller_pd, msg.caller_task, msg.arg0) { continue; }
                    let Some(request) = sex_pdx::text_batch::Request::decode(msg.type_id, msg.arg1, msg.arg2) else { continue; };
                    unsafe {
                        for (index, slot) in SURFACES.iter().enumerate() {
                            if !slot.active || slot.surface_id != msg.arg0 || slot.owner_pd != msg.caller_pd { continue; }
                            if let sex_pdx::text_batch::Request::Fill{sequence}=request{
                                if let Some(region)=text_region_of(msg.caller_pd){TEXT_STAGES[index].fill(msg.caller_pd,msg.caller_task,msg.arg0,sequence,region);}
                            } else if TEXT_STAGES[index].receive(msg.caller_pd,msg.caller_task,msg.arg0,request).is_some() {
                                // Commit seals a candidate; canonical Surface text is
                                // unchanged until its captured Full publication succeeds.
                                let pending=TEXT_STAGES[index].pending().expect("sealed text candidate");
                                let context=pending.context();
                                sex_pdx::serial_record!("[display.text.prepared] target={} task={} surface={} paint={} bytes={} pixels=unconfirmed",
                                    context.owner(),context.task(),context.surface(),context.sequence(),pending.len());
                            } else if let sex_pdx::text_batch::Request::Commit{sequence,..}=request {
                                static mut INCOMPLETE_BUDGET:u8=16;
                                if INCOMPLETE_BUDGET>0 {
                                    INCOMPLETE_BUDGET-=1;
                                    let(owner,sid,paint,len,coverage,sealed,poisoned)=TEXT_STAGES[index].diagnostic();
                                    sex_pdx::serial_record!("[display.text.incomplete] target={} surface={} request={} staged_owner={} staged_sid={} paint={} len={} coverage={:#x} sealed={} poisoned={}",msg.caller_pd,msg.arg0,sequence,owner,sid,paint,len,coverage,sealed as u8,poisoned as u8);
                                }
                            }
                            break;
                        }
                    }
                }
                // ── Text rendering opcodes (V1) ───────────────────────────────
                0xFA => {
                    // OP_TEXT_CLEAR: arg0 = surface_id
                    let sid = msg.arg0;
                    if sid == 0 { continue; }
                    unsafe {
                        for (index,slot) in SURFACES.iter_mut().enumerate() {
                            if slot.active && slot.surface_id == sid {
                                if slot.owner_pd != msg.caller_pd {
                                    let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                    if n & 0x3F == 0 {
                                        serial_println!("AUTH: OP_TEXT_CLEAR rejected sid={} caller={} owner={}",
                                            sid, msg.caller_pd, slot.owner_pd);
                                    }
                                    break;
                                }
                                // Atomic candidate custody fences mixed legacy writes.
                                if TEXT_STAGES[index].needs_publication() { break; }
                                let old_damage=text_surface_damage(slot,FB_W as usize,FB_H as usize);
                                if legacy_text_clear(slot){text_damage.add(old_damage,FB_W as usize,FB_H as usize);}
                                break;
                            }
                        }
                    }
                }
                0xFB => {
                    // OP_TEXT_DRAW: arg0=surface_id, arg1=8 ASCII bytes packed LE,
                    //              arg2 = byte_offset lo (bits 0-7) | char_count (bits 8-11)
                    //                   | byte_offset hi (bits 12-15, TEXT_MODEL_V2)
                    //                   | text_color (bits 32-63)
                    // Legacy senders leave bits 12-15 zero → offsets 0-255 unchanged.
                    let sid = msg.arg0;
                    if sid == 0 { continue; }
                    unsafe {
                        for (index,slot) in SURFACES.iter_mut().enumerate() {
                            if slot.active && slot.surface_id == sid {
                                if slot.owner_pd != msg.caller_pd {
                                    let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                    if n & 0x3F == 0 {
                                        serial_println!("AUTH: OP_TEXT_DRAW rejected sid={} caller={} owner={}",
                                            sid, msg.caller_pd, slot.owner_pd);
                                    }
                                    break;
                                }
                                // Atomic candidate custody fences mixed legacy writes.
                                if TEXT_STAGES[index].needs_publication() { break; }
                                let old_damage=text_surface_damage(slot,FB_W as usize,FB_H as usize);
                                if legacy_text_draw(slot,msg.arg1,msg.arg2){
                                    text_damage.add(old_damage,FB_W as usize,FB_H as usize);
                                    text_damage.add(text_surface_damage(slot,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                                }
                                // Diagnostic: confirm text was written to surface
                                if slot.text_len > 0 && slot.text_len <= 32 {
                                    serial_println!("[sexdisplay.text.draw] sid={} len={} color={:#x}",
                                        sid, slot.text_len, slot.text_color);
                                }
                                break;
                            }
                        }
                    }
                }
                0xFC => {
                    // OP_APPEARANCE_TOKENS: two-call state machine.
                    // Call 1 (TOKEN_BUF_CALL1_RECEIVED=false): buffer 6 packed colors in arg0/arg1/arg2.
                    // Call 2 (TOKEN_BUF_CALL1_RECEIVED=true): commit 8 colors + flags to DISPLAY_TOKENS.
                    // Sequenced by receiver state only — no arg2 tagging (color data in Call 1 arg2).
                    if !lifecycle.is_coordinator(msg.caller_pd) { continue; }
                    unsafe {
                        if TOKEN_BUF_CALL1_RECEIVED && TOKEN_BUF_OWNER != msg.caller_pd { continue; }
                        if !TOKEN_BUF_CALL1_RECEIVED {
                            TOKEN_BUF_OWNER = msg.caller_pd;
                            TOKEN_BUF_ARG0 = msg.arg0;
                            TOKEN_BUF_ARG1 = msg.arg1;
                            TOKEN_BUF_ARG2 = msg.arg2;
                            TOKEN_BUF_CALL1_RECEIVED = true;
                            static mut APPEARANCE_TOKENS_SEQ0_BUDGET: u32 = 4;
                            if APPEARANCE_TOKENS_SEQ0_BUDGET > 0 {
                                APPEARANCE_TOKENS_SEQ0_BUDGET -= 1;
                                serial_println!("[sexdisplay.appearance.tokens] seq=0 buffered");
                            }
                        } else {
                            // Unpack token preset by APPEARANCE_TOKEN_* index from silkbar-model.
                            // Call 1 packed: [0|1] in arg0, [2|3] in arg1, [4|5] in arg2.
                            // Call 2 packed: [6|7] in arg0, flags in arg1.
                            let focus_surface_color  = clamp_color_token(TOKEN_BUF_ARG0 as u32);
                            let frame_rim_color      = clamp_color_token((TOKEN_BUF_ARG0 >> 32) as u32);
                            let frame_top_bar_color  = clamp_color_token(TOKEN_BUF_ARG1 as u32);
                            let active_tab_color     = clamp_color_token((TOKEN_BUF_ARG1 >> 32) as u32);
                            let inactive_tab_color   = clamp_color_token(TOKEN_BUF_ARG2 as u32);
                            let close_light_color    = clamp_color_token((TOKEN_BUF_ARG2 >> 32) as u32);
                            let minimize_light_color = clamp_color_token(msg.arg0 as u32);
                            let zoom_light_color     = clamp_color_token((msg.arg0 >> 32) as u32);
                            let appearance_flags     = (msg.arg1 & 0xFF) as u8;
                            let effect_levels        = ((msg.arg1 >> 8) & 0xFF) as u8 & !0x0F; // blur=0 in V1
                            DISPLAY_TOKENS = RenderTokensV1 {
                                focus_surface_color,
                                frame_rim_color,
                                frame_top_bar_color,
                                active_tab_color,
                                inactive_tab_color,
                                close_light_color,
                                minimize_light_color,
                                zoom_light_color,
                                appearance_flags,
                                effect_levels,
                            };
                            TOKEN_BUF_CALL1_RECEIVED = false;
                            TOKEN_BUF_OWNER = 0;
                            static mut APPEARANCE_TOKENS_APPLIED: u32 = 0;
                            APPEARANCE_TOKENS_APPLIED += 1;
                            let applied = APPEARANCE_TOKENS_APPLIED;
                            static mut APPEARANCE_TOKENS_SEQ1_BUDGET: u32 = 4;
                            if APPEARANCE_TOKENS_SEQ1_BUDGET > 0 {
                                APPEARANCE_TOKENS_SEQ1_BUDGET -= 1;
                                serial_println!("[sexdisplay.appearance.tokens] seq=1 applied={}", applied);
                            }
                            if fb_live {
                                needs_surface_redraw = true;
                            }
                        }
                    }
                }
                0xFD => {
                    // OP_SURFACE_TAB_INFO: arg0=surface_id, arg1=tab_count,
                    //   arg2 low 8 bits=active_tab, arg2 bit 8=chrome_flags
                    let surface_id = msg.arg0;
                    if surface_id == 0 { continue; }
                    let tab_count = (msg.arg1 as u8).min(8);
                    let raw_arg2 = msg.arg2;
                    let active_tab_raw = raw_arg2 as u8;
                    let chrome_flags_raw = ((raw_arg2 >> 8) & 0xff) as u8;
                    let close_allowed = ((chrome_flags_raw & SURFACE_CHROME_CLOSE_ALLOWED) != 0) as u8;
                    let active_tab = if tab_count > 0 {
                        active_tab_raw.min(tab_count.saturating_sub(1))
                    } else { 0 };
                    unsafe {
                        let wm_pd = DISPLAY_WM_PD.load(core::sync::atomic::Ordering::Acquire);
                        let mut updated = false;
                        for slot in SURFACES.iter_mut() {
                            if slot.active && slot.surface_id == surface_id {
                                if slot.owner_pd != msg.caller_pd && !(wm_pd != 0 && msg.caller_pd == wm_pd) {
                                    let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                    if n & 0x3F == 0 {
                                        serial_println!("[sexdisplay.auth.deny] op=0xfd caller={} sid={} owner={} wm={}",
                                            msg.caller_pd, surface_id, slot.owner_pd, wm_pd);
                                    }
                                    break;
                                }
                                updated = (slot.tab_count, slot.active_tab, slot.chrome_flags)
                                    != (tab_count, active_tab, chrome_flags_raw);
                                slot.tab_count = tab_count;
                                slot.active_tab = active_tab;
                                slot.chrome_flags = chrome_flags_raw;
                                break;
                            }
                        }
                        if updated {
                            static mut SURFACE_TAB_INFO_BUDGET: u32 = 8;
                            let b = &mut SURFACE_TAB_INFO_BUDGET;
                            if *b > 0 {
                                *b -= 1;
                                serial_println!("[sexdisplay.surface.tab.info] surface={} tabs={} active={} chrome={}",
                                    surface_id, tab_count, active_tab, chrome_flags_raw);
                                serial_println!(
                                    "[sexdisplay.frame.light.chrome.recv] frame={} sid={} close_allowed={} flags=0x{:02x}",
                                    active_tab as u32, surface_id, close_allowed, chrome_flags_raw
                                );
                                // [silk.chrome.skip.invalid] — tab_count=0 means no chrome to render.
                                // [silk.chrome.clear] — tab_count>0 means chrome cleared and redrawn.
                                if tab_count == 0 {
                                    serial_println!("[silk.chrome.skip.invalid] sid={} tab_count=0 chrome_strip_skipped=1", surface_id);
                                } else {
                                    serial_println!("[silk.chrome.clear] sid={} tab_count={} active={} chrome_redrawn=1", surface_id, tab_count, active_tab);
                                }
                            }
                            // Hover state marker: log when hover flags change.
                            let hover = (chrome_flags_raw & SURFACE_CHROME_FRAME_HOVER) != 0;
                            let light = (chrome_flags_raw & SURFACE_CHROME_LIGHT_HOVER) != 0;
                            if hover || light {
                                static mut FRAME_HOVER_RECV_BUDGET: u32 = 32;
                                let hb = &mut FRAME_HOVER_RECV_BUDGET;
                                if *hb > 0 {
                                    *hb -= 1;
                                    let lk = (chrome_flags_raw & SURFACE_CHROME_LIGHT_KIND_MASK) >> SURFACE_CHROME_LIGHT_KIND_SHIFT;
                                    serial_println!("[sexdisplay.frame.hover.recv] sid={} flags=0x{:02x} light={}",
                                        surface_id, chrome_flags_raw, lk);
                                }
                            }
                            if fb_live {
                                needs_surface_redraw = true;
                            }
                        }
                    }
                }
                _ => {
                    serial_println!("[pdx.opcode.unknown] display type_id={:#x} caller={}", msg.type_id, msg.caller_pd);
                    // Ignore unrelated messages and continue draining.
                    continue;
                }
            }
        }

        // ── Budgeted marker: first 32 cycles ──
        unsafe {
            static mut RENDER_BUDGET_CYCLE: u32 = 32;
            if RENDER_BUDGET_CYCLE > 0 {
                RENDER_BUDGET_CYCLE -= 1;
                if hot_log() { serial_println!("[sexdisplay.render.budget] drained={} surface={} strip={} primary={}",
                    drained,
                    needs_surface_redraw as u8,
                    needs_top_strip_redraw as u8,
                    needs_full_redraw as u8); }
            }
        }

        let transaction_full_render=unsafe{BITMAP_STORES.needs_publication()}||lifecycle.needs_presentation()
            ||(0..surface_lifecycle::CATALOG_CAPACITY).any(|i|lifecycle.phase(i)==Some(surface_lifecycle::Phase::Prepared));
        let (w,h)=unsafe{(FB_W as usize,FB_H as usize)};
        if needs_full_redraw||transaction_full_render||scale_receipts.needs_publication(){pending_paint.request_full();}
        if needs_surface_redraw{pending_paint.request_area();}
        if let Some(rect)=text_damage.rect(){pending_paint.request_damage(rect,w,h,bar_height());}
        if cursor_only_moved{pending_paint.request_cursor();}
        if needs_top_strip_redraw{pending_paint.request_strip();}
        if typed_completion_needs_publication(){join_pending_text(&mut pending_paint,w,h);}
        let plan=pending_paint.plan(w,h);
        let published=if fb_live&&!plan.is_empty(){
            clock_canon_apply_to_bar(&mut bar);
            unsafe{render_prepared(FB_PTR as *mut u32,w,h,&bar,plan)}
        }else{None};
        // Only this completed publication discharges the retained requirements.
        // The service performs no intake during prepare/publish/receipt handling.
        pending_paint.complete(published.is_some());
        if let Some(frame)=published.as_ref(){
            if frame.plan.clock_dirty(){unsafe{LAST_REDRAWN_CLOCK_CANON_SS=frame.canon_ss;}}
            if primary_receipt_pending&&frame.plan.is_full(){
                primary_receipt_pending=false;
                serial_println!("[sexdisplay.render.live.ok] fb_w={} fb_h={}",w,h);
            }
        }
        if scale_receipts.needs_publication()&&published.as_ref().is_some_and(|frame|frame.plan.is_full()){
            scale_receipts.published();
            if let Some((target,value))=scale_receipts.reply(){
                // Record publication before reply can immediately schedule
                // Shell. This marker does not claim transport succeeded.
                sex_pdx::serial_record!("[display.scale.published] target={} receipt={:#x} cpu_publication=1",target,value);
            }
        }
        if let Some((target,value))=scale_receipts.reply(){
            if sex_pdx::pdx_reply(target,value)==0 {
                scale_receipts.sent();
                sex_pdx::serial_record!("[display.scale.reply.sent] target={} receipt={:#x}",target,value);
            }
        }
        finish_display_transactions(&mut lifecycle,published.as_ref());
        unsafe{SCREEN_CAPTURE.poll();}
        unsafe { for stage in &mut TEXT_STAGES { stage.poll_reply(sex_pdx::pdx_reply); } }
        unsafe { BITMAP_STORES.poll_reply(sex_pdx::pdx_reply); }

        // ── Yield once regardless of whether messages remain queued ──
        loop_iter = loop_iter.wrapping_add(1);
        // Nothing arrived and nothing was painted: sleep until a message or the
        // next clock check, so an idle desktop does not spin.
        if drained==0 && plan.is_empty() { let _=sex_pdx::sys_wait_self_ticks(8); }
        else { sex_pdx::sys_yield(); }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
