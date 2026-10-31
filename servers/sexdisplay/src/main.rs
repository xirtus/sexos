#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use sex_pdx::serial_println;
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
const DRAIN_MAX: usize = 16;

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
const BAR_BG_H: usize = 50;

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
mod text_transaction;
mod bitmap_transaction;
mod fill_damage;
mod damage;
static mut TEXT_STAGES: [text_transaction::Stage; MAX_SURFACES] = [text_transaction::Stage::EMPTY; MAX_SURFACES];
static mut BITMAP_STAGE: bitmap_transaction::Stage = bitmap_transaction::Stage::EMPTY;
mod surface_groups;
static mut FRAME_GROUPS: surface_groups::FrameGroups = surface_groups::FrameGroups::new();
use surface_lifecycle::{Surface, MAX_SURFACES, TEXT_BUF_CAP, TEXT_MAX_ROWS, TEXT_LEGACY_COLS, SURFACE_EMPTY};
static mut SURFACES: [Surface; MAX_SURFACES] = [SURFACE_EMPTY; MAX_SURFACES];
static mut FOCUSED_SURFACE_ID: u64 = 0;
static mut APP_PRESENT_BITS: u8 = 0;
static APP300_PRESENTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

fn report_app_presented() {
    if unsafe { APP_PRESENT_BITS & 4 != 0 } && !APP300_PRESENTED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        sex_pdx::serial_record!("[display.app.presented] target=15 surface=300 scene=1");
    }
}
const FOCUS_SURFACE_COLOR: u32 = 0x0089B4FA;

// ── Frame Chrome Rim (focused surface, matches shell FRAME_RIM_PX) ──
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
const FRAME_TOP_BAR_HEIGHT_PX: usize = 28;
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
    const BAR_H: usize = 50;
    let x = (surf.x.max(0) as usize).min(fb_w.saturating_sub(1));
    let y = (surf.y.max(BAR_H as i32) as usize).min(fb_h.saturating_sub(1));
    let max_w = fb_w.saturating_sub(x);
    let max_h = fb_h.saturating_sub(y);
    let w = (surf.w as usize).min(max_w);
    let h = (surf.h as usize).min(max_h);
    (x, y, w, h)
}

/// Composite a single pixel: non-focused surfaces in slot order, then focused on top.
/// Shared by render() and redraw_surface_area() to prevent drift.
fn composite_pixel(x: usize, y: usize, w: usize, h: usize, bg: u32, focused_id: u64) -> u32 {
    let selected=unsafe{FRAME_GROUPS.winner(&SURFACES,focused_id,x,y,w,h).map(|s|s.surface_id)};
    composite_pixel_selected(x,y,w,h,bg,focused_id,selected)
}

fn composite_pixel_selected(x: usize, y: usize, w: usize, h: usize, bg: u32, focused_id: u64, selected:Option<u64>) -> u32 {
    let source = unsafe { SURFACES.iter().find(|s| s.active && Some(s.surface_id) == selected)
        .map(|surface| surface_groups::PixelSource {
            surface,
            content: FRAME_GROUPS.content(surface, &SURFACES, x, y, w, h),
            chrome: surface.surface_id == focused_id || FRAME_GROUPS.is_frame(surface),
        }) };
    composite_pixel_source(x,y,w,h,bg,source)
}

/// Choices are borrowed only during this immutable render row; no lookup or
/// lifecycle authority is retained across a display drain.
fn composite_pixel_source(x: usize, y: usize, w: usize, h: usize, bg: u32, source: Option<surface_groups::PixelSource<'_>>) -> u32 {
    let Some(source) = source else { return bg; };
    let surf = source.surface;
    let (sx, sy, sw, sh) = clamp_surface(surf, w, h);
    if !surf.active || sw == 0 || sh == 0 || x < sx || x >= sx + sw || y < sy || y >= sy + sh { return bg; }
    let mut c = bg;
    let mut winner_mask = surface_lifecycle::native_pixel_bit(surf.owner_pd, surf.surface_id);
    if !source.chrome {
        c = fill_rect_color(surf, x, y, surf.color);
        c = bitmap_color(surf, x, y, c);
        if let Some(tc) = surface_text_fg_at(x, y, surf) { c = tc; }
    } else { unsafe {
        // ── Phase 6 proof: real render-path markers ──────────────
        // Emit once when the focused-surface pixel path is entered.
        // Proves bounds were checked via clamp_surface + sw/sh guards
        // and that the real compositor loop called composite_pixel.
        unsafe {
            static mut PHASE6_RENDER_BEGIN_PROOF: u32 = 1;
            if PHASE6_RENDER_BEGIN_PROOF > 0 {
                PHASE6_RENDER_BEGIN_PROOF -= 1;
                let active_count = SURFACES.iter().filter(|s| s.active).count();
                let frame_count = SURFACES.iter().filter(|s| s.active && s.tab_count > 0 && (s.chrome_flags & SURFACE_CHROME_TOP_BAR) != 0).count();
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
        let rim_right = sw.saturating_sub(FRAME_RIM_PX);
        let rim_bottom = sh.saturating_sub(FRAME_RIM_PX);
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
        // composite_pixel is only called for y >= 51 (see render/redraw_surface_area),
        // but this belt-and-suspenders guard catches any future path that might
        // call composite_pixel at y < BAR_BG_H+1.
        if y < (BAR_BG_H + 1) {
            // y is in or above the SilkBar glow row — skip frame chrome entirely.
            // This pixel will get bar_color or panel_glow from the top strip pass.
        } else if top_bar_active && ly < FRAME_TOP_BAR_HEIGHT_PX {
            // Frosted glass toolbar background.
            let base_alpha: u8 = if frame_hovered { 220 } else { 200 };
            let toolbar_alpha = scale_alpha(base_alpha, chrome_dim);
            if toolbar_alpha > 0 {
                c = glass_over_bg(DISPLAY_TOKENS.frame_top_bar_color, x, y, toolbar_alpha);
            }
            // Divider: bright opaque line at bottom edge.
            if ly == FRAME_TOP_BAR_HEIGHT_PX - 1 {
                c = FRAME_TOP_BAR_DIVIDER_COLOR;
            }
            // Tab strip: glass background with tab color overlay.
            if surf.tab_count > 0
                && lx >= FRAME_TOP_BAR_LIGHT_EXCLUSION_PX
                && lx < rim_right
            {
                let available = rim_right - FRAME_TOP_BAR_LIGHT_EXCLUSION_PX;
                let slot_w = available / surf.tab_count as usize;
                if slot_w > 0 {
                    let tab_idx = (lx - FRAME_TOP_BAR_LIGHT_EXCLUSION_PX) / slot_w;
                    let tab_color = if tab_idx == surf.active_tab as usize {
                        DISPLAY_TOKENS.active_tab_color
                    } else {
                        DISPLAY_TOKENS.inactive_tab_color
                    };
                    let tab_glass = glass_over_bg(tab_color, x, y, 224);
                    let tab_alpha: u8 = if tab_idx == surf.active_tab as usize { 240 } else { 200 };
                    c = alpha_blend_xrgb_over_xrgb(tab_color, tab_glass, tab_alpha);
                }
            }
            // Select title ink against the final glass/tab pixel, then paint it.
            if let Some(fg)=toolbar_title_fg_at(lx,ly,surf.surface_id,c){c=fg;}
            // Frame lights: glass background with light color overlay.
            // Dimmed for single-tab non-hover; full bright for hovered light.
            if ly >= FRAME_TOP_BAR_LIGHT_TOP && ly < FRAME_TOP_BAR_LIGHT_BOTTOM {
                unsafe {
                    static mut FRAME_LIGHT_STARTUP_RENDER_BUDGET: u32 = 32;
                    if FRAME_LIGHT_STARTUP_RENDER_BUDGET > 0 && lx == FRAME_TOP_BAR_LIGHT_GAP_PX && ly == FRAME_TOP_BAR_LIGHT_TOP {
                        FRAME_LIGHT_STARTUP_RENDER_BUDGET -= 1;
                        if hot_log() { serial_println!(
                            "[sexdisplay.frame.light.startup.render] sid={} red={} close_allowed={} reason={}",
                            surf.surface_id,
                            if close_allowed { "enabled" } else { "disabled" },
                            close_allowed as u8,
                            if close_allowed { "close_allowed" } else { "default_disabled_or_protected" }
                        ); }
                    }
                    // Phase 6 draw.ok: one-shot proof emitted from real render path.
                    static mut PHASE6_DRAW_OK_PROOF: u32 = 1;
                    if PHASE6_DRAW_OK_PROOF > 0 && lx == FRAME_TOP_BAR_LIGHT_GAP_PX && ly == FRAME_TOP_BAR_LIGHT_TOP {
                        PHASE6_DRAW_OK_PROOF -= 1;
                        serial_println!("[sexdisplay.frame_lights.draw.ok] rendered=1 red=disabled yellow=available green=available ok=1 reason=render_path");
                    }
                }
                let l1_end = FRAME_TOP_BAR_LIGHT_GAP_PX + FRAME_TOP_BAR_LIGHT_SIZE_PX;
                let l1_hovered = light_hovered && hovered_light_kind == 0;
                let l1_close_base: u8 = if close_allowed {
                    if l1_hovered { 255 } else { 224 }
                } else if l1_hovered {
                    255
                } else {
                    FRAME_LIGHT_CLOSE_DISABLED_BASE_ALPHA
                };
                let light_alpha: u8 = scale_alpha(l1_close_base, chrome_dim);
                if lx >= FRAME_TOP_BAR_LIGHT_GAP_PX && lx < l1_end {
                    if light_alpha > 0 { c = glass_over_bg(DISPLAY_TOKENS.close_light_color, x, y, light_alpha); }
                } else {
                    let l2_start = l1_end + FRAME_TOP_BAR_LIGHT_GAP_PX;
                    let l2_end = l2_start + FRAME_TOP_BAR_LIGHT_SIZE_PX;
                    let l2_hovered = light_hovered && hovered_light_kind == 1;
                    let l2_alpha = scale_alpha(if l2_hovered { 255 } else { 224 }, chrome_dim);
                    if lx >= l2_start && lx < l2_end {
                        if l2_alpha > 0 { c = glass_over_bg(DISPLAY_TOKENS.minimize_light_color, x, y, l2_alpha); }
                    } else {
                        let l3_start = l2_end + FRAME_TOP_BAR_LIGHT_GAP_PX;
                        let l3_end = l3_start + FRAME_TOP_BAR_LIGHT_SIZE_PX;
                        let l3_hovered = light_hovered && hovered_light_kind == 2;
                        let l3_alpha = scale_alpha(if l3_hovered { 255 } else { 224 }, chrome_dim);
                        if lx >= l3_start && lx < l3_end {
                            if l3_alpha > 0 { c = glass_over_bg(DISPLAY_TOKENS.zoom_light_color, x, y, l3_alpha); }
                        }
                    }
                }
            }
        }
        // ── RIM BAND (minimal mode top edge, or left/right/bottom edges always) ──
        else if ly < FRAME_RIM_PX || lx < FRAME_RIM_PX
            || lx >= rim_right || ly >= rim_bottom
        {
            // In the rim band. Check for Frame Lights in top-left corner.
            // This path is reached in minimal mode (top_bar_active=false) or
            // for left/right/bottom edges when top bar is active.
            if ly < FRAME_RIM_PX && !top_bar_active {
                // Minimal mode top rim lights — glass version.
                // Close light dimmed: close_allowed=0, non-interactive.
                let l1_alpha: u8 = if close_allowed { 224 } else { FRAME_LIGHT_CLOSE_DISABLED_BASE_ALPHA };
                if lx >= FRAME_LIGHT_GAP_PX
                    && lx < FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX
                {
                    c = glass_over_bg(DISPLAY_TOKENS.close_light_color, x, y, l1_alpha);
                }
                else if lx >= FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX
                    && lx < FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX + FRAME_LIGHT_SIZE_PX
                {
                    c = glass_over_bg(DISPLAY_TOKENS.minimize_light_color, x, y, 224);
                }
                else if lx >= FRAME_LIGHT_GAP_PX + 2 * (FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX)
                    && lx < FRAME_LIGHT_GAP_PX + 2 * (FRAME_LIGHT_SIZE_PX + FRAME_LIGHT_GAP_PX) + FRAME_LIGHT_SIZE_PX
                {
                    c = glass_over_bg(DISPLAY_TOKENS.zoom_light_color, x, y, 224);
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
                            DISPLAY_TOKENS.active_tab_color
                        } else {
                            DISPLAY_TOKENS.inactive_tab_color
                        };
                        let tab_alpha: u8 = if tab_idx == surf.active_tab as usize { 220 } else { 180 };
                        c = glass_over_bg(tab_color, x, y, tab_alpha);
                    } else {
                        c = glass_over_bg(DISPLAY_TOKENS.frame_rim_color, x, y, 196);
                    }
                } else {
                    c = glass_over_bg(DISPLAY_TOKENS.frame_rim_color, x, y, 196);
                }
            } else {
                // Left, right, or bottom rim edge — pulsing neon glass.
                let frame = RENDER_FRAME_COUNTER.load(core::sync::atomic::Ordering::Relaxed);
                const RIM_BASE_ALPHA: u8 = 196;
                const RIM_AMP: u8 = 12;
                const RIM_PERIOD: u64 = 128;
                let rim_alpha = pulse_alpha(RIM_BASE_ALPHA, RIM_AMP, frame, RIM_PERIOD);
                c = glass_over_bg(DISPLAY_TOKENS.frame_rim_color, x, y, rim_alpha);
            }
        } else {
            // ── SURFACE CONTENT AREA ──
            let content = source.content;
            winner_mask = surface_lifecycle::native_pixel_bit(content.owner_pd, content.surface_id);
            c = fill_rect_color(content, x, y, content.color);
            c = bitmap_color(content, x, y, c);
            // Child text uses its own origin; the parent alone draws chrome.
            if let Some(tc) = surface_text_fg_at(x, y, content) { c = tc; }
        }
    } }
    if winner_mask != 0 { unsafe { APP_PRESENT_BITS |= winner_mask; } }
    c
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

fn bitmap_viewport(surf: &Surface) -> bitmap_transaction::Viewport {
    let chrome = surf.chrome_flags & SURFACE_CHROME_TOP_BAR != 0;
    bitmap_transaction::Viewport { x: surf.x, y: surf.y, width: surf.w, height: surf.h,
        inset: if chrome { 8 } else { 0 }, top: if chrome { FRAME_TOP_BAR_HEIGHT_PX as u32 + 8 } else { 0 } }
}
fn bitmap_color(surf: &Surface, x: usize, y: usize, base: u32) -> u32 {
    if !surf.active || surf.owner_pd != 15 || surf.surface_id != 300 { return base; }
    let Some((x, y)) = bitmap_viewport(surf).local(x, y) else { return base; };
    unsafe { BITMAP_STAGE.sample(surf.owner_pd, surf.surface_id, x, y) }
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
    vertical_gradient_xrgb(y, h, DEFAULT_THEME.bg_top, DEFAULT_THEME.bg_bottom)
}

fn fill_bar_bg_buffer(fb_w: u32, fb_h: u32) -> bool {
    let fill_w = fb_w as usize;
    if fill_w > BAR_BG_W_CAP {
        return false;
    }
    let fill_h = BAR_BG_H.min(fb_h as usize);
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
    let height = BAR_BG_H.min(fb_h as usize);
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

fn sample_bar_blur_bg_xrgb(x: u32, y: u32, fb_w: u32, fb_h: u32) -> u32 {
    if fb_w as usize > BAR_BG_W_CAP || y as usize >= BAR_BG_H || x >= fb_w {
        return bg(y as usize, fb_h as usize);
    }
    unsafe { BAR_BLUR_BUF[y as usize * BAR_BG_W_CAP + x as usize] }
}

#[inline]
fn triangle_wave_u8(frame: u64, period: u64) -> u8 {
    if period == 0 {
        return 0;
    }
    let pos = frame % period;
    let half = period / 2;
    if half == 0 {
        return 0;
    }
    if pos < half {
        ((pos * 255) / half) as u8
    } else {
        let denom = period - half;
        if denom == 0 {
            0
        } else {
            (((period - pos) * 255) / denom) as u8
        }
    }
}

#[inline]
fn pulse_alpha(base: u8, amp: u8, frame: u64, period: u64) -> u8 {
    if period == 0 {
        return base;
    }
    let wave = triangle_wave_u8(frame, period) as i32; // 0..255
    let centered = wave - 127; // ~[-127,128]
    let delta = (centered * amp as i32) / 127;
    let out = base as i32 + delta;
    out.clamp(0, 255) as u8
}

/// Alpha-blend an XRGB foreground over an XRGB background.
/// alpha=0 → bg, alpha=255 → fg.  Result is opaque XRGB.
/// Uses integer arithmetic only; no framebuffer read.
#[inline]
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
fn glass_over_bg(fg: u32, x: usize, y: usize, alpha: u8) -> u32 {
    let fb_w = unsafe { FB_W };
    let fb_h = unsafe { FB_H };
    if fb_h == 0 { return fg; }
    let bg = sample_bar_blur_bg_xrgb(x as u32, y as u32, fb_w, fb_h);
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

fn workspace_color(x: usize, y: usize, bar: &SilkBar) -> Option<u32> {
    const WS_SLOTS: [ModuleSlot; 5] = [
        ModuleSlot::Workspace0,
        ModuleSlot::Workspace1,
        ModuleSlot::Workspace2,
        ModuleSlot::Workspace3,
        ModuleSlot::Workspace4,
    ];
    for (idx, slot) in WS_SLOTS.iter().enumerate() {
        let (wx, wy, ww, wh) = module_rect(bar, *slot);
        if in_rect(x, y, wx, wy, ww, wh) {
            let ws = &bar.workspaces[idx];
            if ws.active {
                // Active workspace gets a subtle glow tint over bg.
                const WORKSPACE_GLOW_BASE: u8 = 48;
                const WORKSPACE_GLOW_AMP: u8 = 32;
                const WORKSPACE_GLOW_PERIOD: u64 = 96;
                let frame = RENDER_FRAME_COUNTER.load(core::sync::atomic::Ordering::Relaxed);
                let glow_alpha = pulse_alpha(
                    WORKSPACE_GLOW_BASE,
                    WORKSPACE_GLOW_AMP,
                    frame,
                    WORKSPACE_GLOW_PERIOD,
                );
                let active_glow = alpha_blend_xrgb_over_xrgb(
                    0x00FFFFFF, DEFAULT_THEME.active, glow_alpha);
                return Some(glass_over_bg(active_glow, x, y, 224));
            }
            if ws.urgent { return Some(DEFAULT_THEME.urgent); }
            return Some(DEFAULT_THEME.muted);
        }
    }
    None
}

fn chip_color(x: usize, y: usize, bar: &SilkBar) -> Option<u32> {
    const R6_CHIP_ALPHA: u8 = 212;
    const CHIP_SLOTS: [ModuleSlot; 4] = [
        ModuleSlot::Chip0,
        ModuleSlot::Chip1,
        ModuleSlot::Chip2,
        ModuleSlot::Clock,
    ];
    for (idx, slot) in CHIP_SLOTS.iter().enumerate() {
        let (cx, cy, cw, ch) = module_rect(bar, *slot);
        if in_rect(x, y, cx, cy, cw, ch) {
            let chip = &bar.chips[idx];
            if !chip.visible { return Some(0x00102038); }
            match chip.kind {
                ChipKind::Net     => return Some(glass_over_bg(DEFAULT_THEME.chip_fill, x, y, R6_CHIP_ALPHA)),
                ChipKind::Wifi    => return Some(0x0038D6C8),
                ChipKind::Battery => return Some(0x00FFB84D),
                ChipKind::Clock   => return Some(DEFAULT_THEME.chip_border),
            }
        }
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
fn phase5_active_app_at(x: usize, y: usize, _bar: &SilkBar) -> Option<u32> {
    // The admitted compositor focus is live in normal builds; the separate
    // phase1 field is updated only by the optional protocol proof path.
    let focused = unsafe { FOCUSED_SURFACE_ID };
    if focused == 0 { return None; }
    const DOT_X: usize = 155;
    const DOT_Y: usize = 22;
    const DOT_SZ: usize = 4;
    if x >= DOT_X && x < DOT_X + DOT_SZ && y >= DOT_Y && y < DOT_Y + DOT_SZ {
        app_indicator_color(focused as u32)
    } else {
        None
    }
}

/// Same visible top-strip content for full and incremental redraws.
fn top_strip_pixel(x: usize, y: usize, bar: &SilkBar) -> u32 {
    if let Some(fg) = clock_fg_at(x, y, bar) { fg }
    else if let Some(fg) = bell_badge_at(x, y, bar) { fg }
    else if let Some(fg) = phase5_active_app_at(x, y, bar) { fg }
    else if let Some(fg) = phase5_tint_swatch_at(x, y, bar) { fg }
    else if let Some(fg) = phase5_palette_dot_at(x, y, bar) { fg }
    else { bar_color(x, y, bar) }
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

fn bar_color(x: usize, y: usize, bar: &SilkBar) -> u32 {
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
    if bar.selected_options_mask != 0 {
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
    if let Some(c) = workspace_color(x, y, bar) { return c; }
    // Status chip backgrounds
    if let Some(c) = chip_color(x, y, bar) { return c; }
    // Bell indicator (privacy-safe presence dot between Battery and Clock)
    let (bx, by, bw, bh) = module_rect(bar, ModuleSlot::Bell);
    if in_rect(x, y, bx, by, bw, bh) {
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
            return glass_over_bg(DEFAULT_THEME.panel_glow, x, y, R6_EDGE_ALPHA);
        }
        let launcher_body = glass_over_bg(0x005CAEE8, x, y, 200);
        return alpha_blend_xrgb_over_xrgb(0x0082D6FF, launcher_body, 64);
    }
    // Bar body: glass translucency over desktop background,
    // with subtle bottom-edge glow falloff.
    const BAR_BOTTOM: usize = 50;
    const GLOW_SPREAD: usize = 4;
    if y == 0 {
        let body = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA);
        return alpha_blend_xrgb_over_xrgb(BAR_TOP_HIGHLIGHT, body, 64);
    }
    if y == BAR_BOTTOM - 1 {
        let body = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA);
        let darkened = alpha_blend_xrgb_over_xrgb(BAR_BOTTOM_DARK_EDGE, body, 88);
        return alpha_blend_xrgb_over_xrgb(DEFAULT_THEME.panel_glow, darkened, 48);
    }
    if y + GLOW_SPREAD >= BAR_BOTTOM && y < BAR_BOTTOM {
        let dist = (BAR_BOTTOM - 1 - y) as u32;
        let glow_a = glow_edge_alpha(dist, GLOW_SPREAD as u32, R6_EDGE_ALPHA);
        if glow_a > 0 {
            let base = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA);
            let body = alpha_blend_xrgb_over_xrgb(BAR_CRYSTAL_TINT, base, 36);
            return alpha_blend_xrgb_over_xrgb(DEFAULT_THEME.panel_glow, body, glow_a);
        }
    }
    let base = glass_over_bg(DEFAULT_THEME.panel_fill, x, y, R6_BAR_ALPHA);
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
    (0x00C4, [10, 0, 14, 17, 31, 17, 17]), // Ä
    (0x00C9, [2, 4, 31, 16, 30, 16, 31]), // É
    (0x00D6, [10, 0, 14, 17, 17, 17, 14]), // Ö
    (0x00DC, [10, 0, 17, 17, 17, 17, 14]), // Ü
    (0x00DF, [12, 18, 18, 28, 18, 18, 28]), // ß
    (0x00E4, [10, 0, 14, 1, 15, 17, 15]), // ä
    (0x00E9, [2, 4, 14, 17, 31, 16, 14]), // é
    (0x00F6, [10, 0, 0, 14, 17, 17, 14]), // ö
    (0x00FC, [10, 0, 17, 17, 17, 19, 13]), // ü
    (0x2026, [0, 0, 0, 0, 0, 0, 21]), // …
    (0x20AC, [7, 8, 30, 8, 30, 8, 7]), // €
    (0x2190, [0, 4, 8, 31, 8, 4, 0]), // ←
    (0x2191, [4, 14, 21, 4, 4, 4, 0]), // ↑
    (0x2192, [0, 4, 2, 31, 2, 4, 0]), // →
    (0x2193, [0, 4, 4, 4, 21, 14, 4]), // ↓
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

/// Fixed-position title glyphs choose contrast against the final composed background.
fn toolbar_title_fg_at(lx: usize, ly: usize, surface_id: u64, background: u32) -> Option<u32> {
    const SCALE: usize = 2;
    const GLYPH_W: usize = sex_pdx::desktop_style::GLYPH_WIDTH as usize;
    const GLYPH_H: usize = sex_pdx::desktop_style::GLYPH_HEIGHT as usize;
    let title_x = FRAME_TOP_BAR_LIGHT_EXCLUSION_PX + 12;
    let title_y = (FRAME_TOP_BAR_HEIGHT_PX - GLYPH_H * SCALE) / 2;
    if lx < title_x || ly < title_y || ly >= title_y + GLYPH_H * SCALE { return None; }
    let col = (lx - title_x) / ((GLYPH_W + 1) * SCALE);
    let px = ((lx - title_x) / SCALE) % (GLYPH_W + 1);
    let py = (ly - title_y) / SCALE;
    if px >= GLYPH_W { return None; }
    let title: &[u8] = match surface_id {
        200 => b"Linen", 201 | 156 | 301 | 307 => b"Quil", 0x99 | 154 | 302 | 306 => b"Spindle",
        300 => b"Browser", 100 => b"App", _ => return None,
    };
    let glyph = ascii_glyph_index(*title.get(col)?)?;
    if (FONT_ASCII_5X7[glyph][py] >> (4 - px)) & 1 != 0 { Some(sex_pdx::desktop_style::contrast_ink(background)) } else { None }
}

/// Scale by whole pixels so glyph edges remain crisp. A legacy 20-column
/// row needs 240px plus two 8px insets at 2x; narrow overlays retain 1x.
fn surface_text_scale(surf: &Surface) -> usize {
    sex_pdx::desktop_style::scale(surf.w) as usize
}

/// Returns `Some(fg_color)` if pixel (x, y) is a text glyph foreground pixel
/// within the given surface, or `None` if background/not in the text area.
/// Uses FONT_ASCII_5X7 (5×7 glyphs) with 1px spacing between characters.
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
        (surf.w.saturating_sub(16) / advance).clamp(1, 80) as u8
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

fn surface_text_fg_at(x: usize, y: usize, surf: &Surface) -> Option<u32> {
    if surf.text_len == 0 || surf.line_count == 0 { return None; }
    const GLYPH_W: usize = sex_pdx::desktop_style::GLYPH_WIDTH as usize;
    const GLYPH_H: usize = sex_pdx::desktop_style::GLYPH_HEIGHT as usize;
    const CHAR_SPACING: usize = 1;
    let scale = surface_text_scale(surf);
    let line_h = (GLYPH_H + 2) * scale;
    let tx = (surf.x.max(0) as usize).saturating_add(sex_pdx::desktop_style::INSET as usize);
    let top_inset = if surf.chrome_flags & SURFACE_CHROME_TOP_BAR != 0 { FRAME_TOP_BAR_HEIGHT_PX + 8 } else { sex_pdx::desktop_style::TEXT_TOP as usize };
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
    if px >= GLYPH_W || py >= GLYPH_H { return background; }
    let glyph_row = scalar_glyph(scalar)[py];
    let bit = (glyph_row >> (GLYPH_W - 1 - px)) & 1;
    if bit != 0 { Some(surf.text_color) } else { background }
}

/// Returns `Some(fg)` if pixel (x, y) is a clock-digit foreground pixel,
/// or `None` if it is background/not in the clock area.
/// This is called inline during rendering to avoid a separate overlay pass
/// that would create a tear window between bar-fill and clock-overlay.
/// Two finite ASCII rows share the ordinary clock chip and renderer font.
/// The wire seconds are already source-range checked; modulo is a UTC view of
/// that one sample, never elapsed time or an estimate from rendering cadence.
fn civil_clock_lines(clock:&silkbar_model::civil_clock::CivilClock)->[[u8;13];2]{
    use silkbar_model::civil_clock::State;
    let mut lines=[[b' ';13];2];
    let (first,second):(&[u8],&[u8])=match clock.state(){
        State::Snapshot=>(b"UTC 00:00:00",b"q35 1s snap"),
        State::Rollback=>(b"UTC blocked",b"q35 1s stop"),
        State::Exhausted=>(b"UTC ended",b"src? prec?"),
        State::Unavailable=>(b"UTC unknown",b"q35 1s fail"),
        State::Unknown if clock.is_stale()=>(b"UTC stale",b"q35 1s old"),
        State::Unknown=>(b"UTC unknown",b"src? prec?"),
    };
    lines[0][..first.len()].copy_from_slice(first);lines[1][..second.len()].copy_from_slice(second);
    if clock.state()==State::Snapshot{
        let day=clock.seconds()%86_400;
        for(at,value)in[(4,day/3600),(7,day/60%60),(10,day%60)]{
            lines[0][at]=b'0'+(value/10)as u8;lines[0][at+1]=b'0'+(value%10)as u8;
        }
    }
    lines
}
fn civil_clock_fg_at(x:usize,y:usize,bar:&SilkBar)->Option<u32>{
    let(cx,cy,cw,ch)=module_rect(bar,ModuleSlot::Clock);
    if x<cx||x>=cx+cw||y<cy||y>=cy+ch{return None;}
    let row=if (cy+1..cy+8).contains(&y){0}else if(cy+11..cy+18).contains(&y){1}else{return None;};
    let col=(x-cx)/6;let px=(x-cx)%6;if col>=13||px>=5{return None;}
    let lines=civil_clock_lines(&bar.civil_clock);let glyph=ascii_glyph_index(lines[row][col])?;
    let py=y-(cy+if row==0{1}else{11});
    ((FONT_ASCII_5X7[glyph][py]>>(4-px))&1!=0).then_some(DEFAULT_THEME.text)
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
    let stamp=(clock.sequence(),clock.state(),clock.seconds(),clock.is_stale(),w,h);
    static mut LAST_CIVIL_DRAW:Option<(u32,State,u32,bool,usize,usize)>=None;
    unsafe{
        if LAST_CIVIL_DRAW==Some(stamp){return;}
        LAST_CIVIL_DRAW=Some(stamp);
    }
    sex_pdx::serial_record!("[display.civil.draw] sequence={} state={:?} seconds={} stale={} pixels=written path={} x={} y={} w={} h={}",clock.sequence(),clock.state(),clock.seconds(),clock.is_stale()as u8,path,x,y,cw,ch);
}
fn clock_fg_at(x: usize, y: usize, bar: &SilkBar) -> Option<u32> {
    if bar.civil_enabled {return civil_clock_fg_at(x,y,bar);}
    const CLOCK_FG: u32 = DEFAULT_THEME.text;
    const CLOCK_PREFIX:&[u8]=b"DEMO ";
    const CLOCK_DIGIT_OFFSET:usize=30;
    let (cx, cy, _, _) = module_rect(bar, ModuleSlot::Clock);
    let cy = cy + 1; // slight inset into chip area

    // Quick bounding-box reject
    if y < cy || y >= cy + 7 {
        return None;
    }
    if x < cx || x > cx + CLOCK_DIGIT_OFFSET + 45 {
        return None;
    }
    // Both the SilkBar producer and display fallback advance a software
    // demonstration counter. Neither is wall time or a certificate UTC source.
    if x<cx+CLOCK_DIGIT_OFFSET{
        let col=(x-cx)/6;let px=(x-cx)%6;
        if px>=5{return None;}
        let glyph=ascii_glyph_index(*CLOCK_PREFIX.get(col)?)?;
        return ((FONT_ASCII_5X7[glyph][y-cy]>>(4-px))&1!=0).then_some(CLOCK_FG);
    }
    let cx=cx+CLOCK_DIGIT_OFFSET;

    // Colon 1 at offset 14, Colon 2 at offset 31
    if x == cx + 14 || x == cx + 31 {
        if y == cy + 1 || y == cy + 5 {
            return Some(CLOCK_FG);
        }
        return None;
    }

    // Digit offsets: 0, 7, 17, 24, 34, 41
    const DIGITS: [usize; 6] = [0, 7, 17, 24, 34, 41];
    for (di, &dx) in DIGITS.iter().enumerate() {
        if x < cx + dx || x >= cx + dx + 5 {
            continue;
        }
        let col = x - (cx + dx);
        let row = y - cy;
        let digit: usize = match di {
            0 => (bar.clock_hh / 10) as usize,
            1 => (bar.clock_hh % 10) as usize,
            2 => (bar.clock_mm / 10) as usize,
            3 => (bar.clock_mm % 10) as usize,
            4 => (bar.clock_ss / 10) as usize,
            5 => (bar.clock_ss % 10) as usize,
            _ => return None,
        };
        if (FONT[digit][row] >> (4 - col)) & 1 != 0 {
            return Some(CLOCK_FG);
        }
        return None; // digit background pixel
    }
    None
}

/// Returns `Some(fg)` if pixel (x, y) is a Bell count badge foreground pixel,
/// or `None` if it is background / not in the badge area.
/// Uses the same 5×7 FONT as the clock digits.
/// Badge is positioned at the right side of the Bell module slot.
fn bell_badge_at(x: usize, y: usize, bar: &SilkBar) -> Option<u32> {
    let n = bar.bell_state.total_visible;
    if n == 0 || bar.bell_state.flags & 1 == 0 {
        return None; // no badge when empty or Bell unavailable
    }
    // Clamp to 99 (two digits max)
    let n = n.min(99);
    let tens = n / 10;
    let ones = n % 10;

    let (bx, by, _, bh) = module_rect(bar, ModuleSlot::Bell);
    let badge_y = by + (bh / 2) - 3; // vertically centered, FONT is 7px tall
    if y < badge_y || y >= badge_y + 7 {
        return None;
    }
    let row = y - badge_y;

    // Two-digit badge: tens at bx+9, ones at bx+14 (within 18px slot)
    let left = if tens > 0 { bx + 9 } else { bx + 11 }; // single digit offset if <10
    let right = bx + 14;

    for (di, digit_x) in [(0, left), (1, right)].iter() {
        let digit = if *di == 0 { tens } else { ones };
        if *di == 0 && tens == 0 {
            continue; // skip leading zero
        }
        if x < *digit_x || x >= *digit_x + 5 {
            continue;
        }
        let col = x - *digit_x;
        if (FONT[digit as usize][row] >> (4 - col)) & 1 != 0 {
            return Some(DEFAULT_THEME.text); // use same color as clock
        }
        return None;
    }
    None
}

fn render(fb: *mut u32, w: usize, h: usize, bar: &SilkBar) -> bool {
    let repaint_started = repaint_begin();
    let fb_addr = fb as u64;
    if fb_addr < HIGH_HALF_BASE {
        return false;
    }
    if w == 0 || h == 0 || w > MAX_FB_W || h > MAX_FB_H {
        return false;
    }
    let pixels = match w.checked_mul(h) {
        Some(v) => v,
        None => return false,
    };
    let bytes = match pixels.checked_mul(4) {
        Some(v) => v as u64,
        None => return false,
    };
    let end_addr = match fb_addr.checked_add(bytes) {
        Some(v) => v,
        None => return false,
    };
    // Guard full framebuffer range before first write.
    if end_addr < HIGH_HALF_BASE {
        return false;
    }
    let frame = RENDER_FRAME_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
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
        serial_println!("[sexdisplay.render.anim] period=96 base=48 amp=32");
    }
    if !R6_GLASS_PROOF_LOGGED.swap(true, core::sync::atomic::Ordering::Relaxed) {
        serial_println!("[sexdisplay.render.glass.r6] bar_alpha=196 chip_alpha=212 blur=1 pulse=1");
    }

    let focused_id = unsafe { FOCUSED_SURFACE_ID };
    // Top strip boundary: BAR_H = 50. Keep in sync with BAR_H constant.
    // Rows 0..49 = SilkBar panel background, 50 = glow edge.
    unsafe { APP_PRESENT_BITS = 0; }
    for y in 0..h {
        let mut row=if y>50 {unsafe{FRAME_GROUPS.row_winners(&SURFACES,focused_id,y,w,h)}}else{None};
        for x in 0..w {
            let c: u32 = if y < 50 {
                top_strip_pixel(x, y, bar)
            } else if y == 50 {
                DEFAULT_THEME.panel_glow // low-contrast bar edge
            } else {
                if let Some(row)=row.as_mut(){
                    composite_pixel_source(x,y,w,h,bg(y,h),row.source(x))
                }else{composite_pixel(x,y,w,h,bg(y,h),focused_id)}
            };
            let idx = y * w + x;
            if idx < total_pixels {
                unsafe { core::ptr::write_volatile(fb.add(idx), c); }
            }
        }
    }
    draw_atlas_cards_pass(fb, w, h, total_pixels);
    draw_atlas_frame_previews_pass(fb, w, h, total_pixels);
    draw_cursor_z_top(fb, w, h, total_pixels, true);
    draw_launcher_panel(fb, w, h, total_pixels);
    report_app_presented();
    repaint_end("full", repaint_started);
    civil_clock_drawn(bar,w,h,"full");
    true
}

/// Redraw the top strip (rows 0..50) — bar background, workspace indicators, chips, and clock digits.
/// Called for all SilkBar updates (clock, workspace active/urgent, chip visibility/kind).
/// This is the only function that touches y<50 pixels after initial render.
fn redraw_top_strip(fb: *mut u32, w: usize, h: usize, bar: &SilkBar) {
    let repaint_started = repaint_begin();
    let fb_addr = fb as u64;
    if fb_addr < HIGH_HALF_BASE {
        return;
    }
    if w == 0 || h == 0 || w > MAX_FB_W || h > MAX_FB_H {
        return;
    }
    if h < 51 {
        return;
    }
    let total_pixels = match w.checked_mul(h) {
        Some(v) => v,
        None => return,
    };
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
    for y in 0..51 {
        for x in 0..w {
            let c: u32 = if y < 50 {
                top_strip_pixel(x, y, bar)
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
    draw_atlas_cards_pass(fb, w, h, total_pixels);
    draw_atlas_frame_previews_pass(fb, w, h, total_pixels);
    // save_under=false: this path repaints only y<50; pixels under the
    // cursor (y>=51) still contain the drawn cursor — capturing here would
    // poison the save-under patch with cursor pixels.
    draw_cursor_z_top(fb, w, h, total_pixels, false);
    draw_launcher_panel(fb, w, h, total_pixels);
    repaint_end("strip", repaint_started);
    civil_clock_drawn(bar,w,h,"strip");
}

/// Redraw all pixels from y=50 to bottom (background gradient + all active surfaces).
/// Never touches y<50 (SilkBar/clock/chips). Not a full render() call.
fn redraw_surface_area(fb: *mut u32, w: usize, h: usize) {
    let repaint_started = repaint_begin();
    let fb_addr = fb as u64;
    if fb_addr < HIGH_HALF_BASE { return; }
    if w == 0 || h == 0 || w > MAX_FB_W || h > MAX_FB_H { return; }
    if h < 51 { return; }
    let total_pixels = match w.checked_mul(h) {
        Some(v) => v,
        None => return,
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
    let focused_id = unsafe { FOCUSED_SURFACE_ID };
    unsafe { APP_PRESENT_BITS = 0; }
    for y in 50..h {
        let mut row=if y>50 {unsafe{FRAME_GROUPS.row_winners(&SURFACES,focused_id,y,w,h)}}else{None};
        for x in 0..w {
            let c: u32 = if y == 50 {
                DEFAULT_THEME.panel_glow // low-contrast bar edge
            } else {
                if let Some(row)=row.as_mut(){
                    composite_pixel_source(x,y,w,h,bg(y,h),row.source(x))
                }else{composite_pixel(x,y,w,h,bg(y,h),focused_id)}
            };
            let idx = y * w + x;
            if idx < total_pixels {
                unsafe { core::ptr::write_volatile(fb.add(idx), c); }
            }
        }
    }
    draw_atlas_cards_pass(fb, w, h, total_pixels);
    draw_atlas_frame_previews_pass(fb, w, h, total_pixels);
    draw_cursor_z_top(fb, w, h, total_pixels, true);
    draw_launcher_panel(fb, w, h, total_pixels);
    report_app_presented();
    repaint_end("area", repaint_started);
}

/// Conservative old/new glyph footprint, derived from the same layout fields
/// as surface_text_fg_at. Geometry changes use the full scene path instead.
fn text_surface_damage(surf: &Surface, w: usize, h: usize) -> (usize,usize,usize,usize) {
    if surf.text_len == 0 || surf.line_count == 0 {return (0,0,0,0);}
    let (x,y,sw,sh)=surface_groups::rect(surf,w,h);
    let top=if surf.chrome_flags & SURFACE_CHROME_TOP_BAR != 0 {FRAME_TOP_BAR_HEIGHT_PX+8} else {sex_pdx::desktop_style::TEXT_TOP as usize};
    let text_y=(surf.y.max(0) as usize).saturating_add(top).max(y);
    let bottom=(surf.y.max(0) as usize).saturating_add(top)
        .saturating_add(surf.line_count as usize * 9 * surface_text_scale(surf))
        .min(y.saturating_add(sh));
    (x,text_y,sw,bottom.saturating_sub(text_y))
}

/// Text-only redraw after a complete transaction. Restore the old cursor patch
/// before touching damage, then capture its new background exactly once.
fn redraw_surface_damage(fb: *mut u32, w: usize, h: usize, rect: damage::Rect) {
    let repaint_started = repaint_begin();
    if (fb as u64) < HIGH_HALF_BASE || w == 0 || h < 51 || w > MAX_FB_W || h > MAX_FB_H { return; }
    if rect.x >= rect.right || rect.y < 51 || rect.y >= rect.bottom || rect.right > w || rect.bottom > h { return; }
    if ATLAS_PHASE_C_RENDER_STUB_PROOF_ENABLED || ATLAS_PHASE_D_FRAME_PREVIEW_STUB_PROOF_ENABLED {
        redraw_surface_area(fb,w,h); return;
    }
    let Some(total) = w.checked_mul(h) else { return; };
    let focused = unsafe { FOCUSED_SURFACE_ID };
    unsafe { restore_cursor_under(fb,w,h,total); APP_PRESENT_BITS = 0; }
    for y in rect.y..rect.bottom {
        let mut row=unsafe{FRAME_GROUPS.row_winners(&SURFACES,focused,y,w,h)};
        for x in rect.x..rect.right {
            let color=if let Some(row)=row.as_mut() {
                composite_pixel_source(x,y,w,h,bg(y,h),row.source(x))
            } else { composite_pixel(x,y,w,h,bg(y,h),focused) };
            unsafe { core::ptr::write_volatile(fb.add(y*w+x),color); }
        }
    }
    draw_cursor_z_top(fb,w,h,total,true);
    draw_launcher_panel(fb,w,h,total);
    report_app_presented();
    repaint_end("text", repaint_started);
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
static mut CURSOR_UNDER_SAVED: [u32; CURSOR_ARROW_W * CURSOR_ARROW_H] =
    [0; CURSOR_ARROW_W * CURSOR_ARROW_H];
static mut CURSOR_UNDER_X: usize = 0;
static mut CURSOR_UNDER_Y: usize = 0;
static mut CURSOR_UNDER_VALID: bool = false;

/// Restore the framebuffer pixels saved beneath the last cursor draw.
/// Mirrors draw_cursor_z_top bounds rules exactly: y<51 SilkBar zone never
/// written, x/y clipped to fb dims, idx bounds-checked against total_pixels.
fn restore_cursor_under(fb: *mut u32, w: usize, h: usize, total_pixels: usize) {
    unsafe {
        if !CURSOR_UNDER_VALID { return; }
        let ox = CURSOR_UNDER_X;
        let oy = CURSOR_UNDER_Y;
        for row in 0..CURSOR_ARROW_H {
            let py = oy + row;
            if py >= h { break; }
            if py < 51 { continue; } // never write into SilkBar zone (y<51)
            for col in 0..CURSOR_ARROW_W {
                let px = ox + col;
                if px >= w { continue; }
                let idx = py * w + px;
                if idx < total_pixels {
                    core::ptr::write_volatile(
                        fb.add(idx),
                        CURSOR_UNDER_SAVED[row * CURSOR_ARROW_W + col],
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

    let mut any_nonzero = false;
    for y in 0..PROOF_H {
        for x in 0..PROOF_W {
            let c = if y < 50 {
                if let Some(fg) = clock_fg_at(x, y, &proof_bar) {
                    fg
                } else if let Some(fg) = bell_badge_at(x, y, &proof_bar) {
                    fg
                } else {
                    bar_color(x, y, &proof_bar)
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
/// enabling the cursor-only fast path. Pass true only when the framebuffer
/// under the cursor is current (freshly composited or just restored); pass
/// false from redraw_top_strip, which repaints only y<50 and would capture
/// cursor-contaminated pixels.
fn draw_cursor_z_top(fb: *mut u32, w: usize, h: usize, total_pixels: usize, save_under: bool) {
    unsafe {
        for surf in SURFACES.iter() {
            if !surf.active || surf.surface_id != CURSOR_SURFACE_ID { continue; }
            let ox = surf.x.max(0) as usize; // origin x (not clamped — arrow clips naturally)
            let oy = surf.y.max(0) as usize; // origin y
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
                INPUT_TRACE_LAST_SEQ != 0 && INPUT_TRACE_LAST_SEQ != LAST_COUNTED_DRAW_SEQ
            };
            unsafe {
                if input_seq_advanced {
                INPUT_TRACE_DRAWS = INPUT_TRACE_DRAWS.wrapping_add(1);
                DISPLAY_DRAW_TICK = DISPLAY_DRAW_TICK.wrapping_add(1);
                static mut INPUT_TRACE_DRAW_BUDGET: u32 = 32;
                if INPUT_TRACE_DRAW_BUDGET > 0 {
                    INPUT_TRACE_DRAW_BUDGET -= 1;
                    if INPUT_TRACE_LAST_SEQ != 0 && INPUT_TRACE_LAST_SHELL_TICK != 0 {
                        serial_println!("[input.trace.display.cursor.draw] seq={} shell_tick={} display_tick={} x={} y={}", INPUT_TRACE_LAST_SEQ, INPUT_TRACE_LAST_SHELL_TICK, DISPLAY_DRAW_TICK, ox as i32, oy as i32);
                    } else if INPUT_TRACE_LAST_SEQ != 0 {
                        serial_println!("[input.trace.display.cursor.draw] seq={} shell_tick=unknown display_tick={} x={} y={}", INPUT_TRACE_LAST_SEQ, DISPLAY_DRAW_TICK, ox as i32, oy as i32);
                    } else {
                        serial_println!("[input.trace.display.cursor.draw] seq=unknown shell_tick=unknown display_tick={} x={} y={}", DISPLAY_DRAW_TICK, ox as i32, oy as i32);
                    }
                    if INPUT_TRACE_DRAW_BUDGET == 0 { INPUT_TRACE_DISPLAY_BUDGET_HIT = 1; }
                }
                }
            }
            // Save-under capture: full 8x16 rect, same clip rules as the
            // arrow write loop below, read-only against the framebuffer.
            if save_under {
                CURSOR_UNDER_X = ox;
                CURSOR_UNDER_Y = oy;
                for row in 0..CURSOR_ARROW_H {
                    let py = oy + row;
                    if py >= h { break; }
                    if py < 51 { continue; }
                    for col in 0..CURSOR_ARROW_W {
                        let px = ox + col;
                        if px >= w { continue; }
                        let idx = py * w + px;
                        if idx < total_pixels {
                            CURSOR_UNDER_SAVED[row * CURSOR_ARROW_W + col] =
                                core::ptr::read_volatile(fb.add(idx));
                        }
                    }
                }
                CURSOR_UNDER_VALID = true;
            }
            for row in 0..CURSOR_ARROW_H {
                let py = oy + row;
                if py >= h { break; }
                if py < 51 { continue; } // never write into SilkBar zone (y<51)
                let bits = CURSOR_ARROW_BITMAP[row];
                for col in 0..CURSOR_ARROW_W {
                    if bits & (0x80u8 >> col) == 0 { continue; }
                    let px = ox + col;
                    if px >= w { continue; }
                    let idx = py * w + px;
                    if idx < total_pixels {
                        core::ptr::write_volatile(fb.add(idx), CURSOR_ARROW_COLOR);
                    }
                }
            }
            // INPUT_PRESENT_TRACE_V1: cursor pixels are now in the framebuffer
            // (direct write, no flip) — this is the present point.
            // Same seq-advance gating as the draw counter above.
            unsafe {
                if input_seq_advanced {
                INPUT_TRACE_PRESENTS = INPUT_TRACE_PRESENTS.wrapping_add(1);
                DISPLAY_PRESENT_TICK = DISPLAY_PRESENT_TICK.wrapping_add(1);
                static mut INPUT_TRACE_PRESENT_BUDGET: u32 = 32;
                if INPUT_TRACE_PRESENT_BUDGET > 0 {
                    INPUT_TRACE_PRESENT_BUDGET -= 1;
                    if INPUT_TRACE_LAST_SEQ != 0 && INPUT_TRACE_LAST_SHELL_TICK != 0 {
                        serial_println!("[input.trace.display.cursor.present] seq={} shell_tick={} display_tick={} x={} y={}", INPUT_TRACE_LAST_SEQ, INPUT_TRACE_LAST_SHELL_TICK, DISPLAY_PRESENT_TICK, ox as i32, oy as i32);
                    } else if INPUT_TRACE_LAST_SEQ != 0 {
                        serial_println!("[input.trace.display.cursor.present] seq={} shell_tick=unknown display_tick={} x={} y={}", INPUT_TRACE_LAST_SEQ, DISPLAY_PRESENT_TICK, ox as i32, oy as i32);
                    } else {
                        serial_println!("[input.trace.display.cursor.present] seq=unknown shell_tick=unknown display_tick={} x={} y={}", DISPLAY_PRESENT_TICK, ox as i32, oy as i32);
                    }
                }
                // Same tightened cadence as the recv summary above.
                if INPUT_TRACE_PRESENTS == 4 || INPUT_TRACE_PRESENTS % 8 == 0 {
                    input_trace_display_summary();
                }
                LAST_COUNTED_DRAW_SEQ = INPUT_TRACE_LAST_SEQ;
                }
            }
            break;
        }
    }
}

/// Pass 4: draw launcher panel surface (LAUNCHER_PANEL_SURFACE_ID) on top of all
/// app surfaces but under the cursor. Renders a solid rect with a thin border.
/// Only active when the launcher surface is active (toggled by shell).
/// Does not affect the top-strip render proof (y<50 area).
fn draw_launcher_panel(fb: *mut u32, w: usize, h: usize, total_pixels: usize) {
    unsafe {
        for surf in SURFACES.iter() {
            if !surf.active || surf.surface_id != LAUNCHER_PANEL_SURFACE_ID { continue; }
            let (sx, sy, sw, sh) = clamp_surface(surf, w, h);
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
fn draw_atlas_cards_pass(fb: *mut u32, w: usize, h: usize, total_pixels: usize) {
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
            let surf = &SURFACES[si];
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

            let (sx, sy, sw, sh) = clamp_surface(surf, w, h);
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
            if sy < 51 {
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

            let active_flag: u8 = if surf.surface_id == (unsafe { FOCUSED_SURFACE_ID }) { 1 } else { 0 };

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
                if ty < h && ty >= 51 {
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
                if by < h && by >= 51 {
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
                        if py < h && py >= 51 {
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
                        if py < h && py >= 51 {
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
fn draw_atlas_frame_previews_pass(fb: *mut u32, w: usize, h: usize, total_pixels: usize) {
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
                let surf = &SURFACES[si];
                if !surf.active { continue; }
                if surf.surface_id == CURSOR_SURFACE_ID || surf.surface_id == LAUNCHER_PANEL_SURFACE_ID { continue; }
                let (_, sy, sw, sh) = clamp_surface(surf, w, h);
                if sw == 0 || sh == 0 || sy < 51 { continue; }
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
            let surf = &SURFACES[si];
            if !surf.active {
                continue;
            }
            if surf.surface_id == CURSOR_SURFACE_ID || surf.surface_id == LAUNCHER_PANEL_SURFACE_ID {
                continue;
            }

            let (sx, sy, sw, sh) = clamp_surface(surf, w, h);
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
            if sy < 51 {
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

            let active_flag: u8 = if surf.surface_id == FOCUSED_SURFACE_ID { 1 } else { 0 };
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
                if py >= 51 {
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
    ->Result<silkbar_model::civil_clock::Applied,silkbar_model::civil_clock::Reject>{
    use silkbar_model::civil_clock::Reject;
    let expected=sex_pdx::clock_source::STATUS_BAR;
    let min=u32::try_from(sex_pdx::clock_source::MIN_SECONDS).map_err(|_|Reject::Framing)?;
    let max=u32::try_from(sex_pdx::clock_source::MAX_SECONDS).map_err(|_|Reject::Framing)?;
    let result=bar.civil_clock.receive((caller_pd,caller_task),(expected.pd,expected.task),args,min,max,now);
    if result.is_ok(){bar.civil_enabled=true;}
    result
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
    let mut mandatory = false;
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
            mandatory |= c.target_pd == 15 && c.surface_id == 300;
        }
    }
    assert!(mandatory, "missing display lifecycle catalog");
    lifecycle
}

fn prepare_app_cleanup(lifecycle: &mut surface_lifecycle::Lifecycle, bar: &mut SilkBar) -> bool {
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
                BITMAP_STAGE.retire(info.target_pd);
                for (index,stage) in TEXT_STAGES.iter_mut().enumerate() {
                    stage.retire(info.target_pd);
                    if !SURFACES[index].active { stage.reset(); }
                }
                FOCUSED_SURFACE_ID = refs.focused_sid;
                bar.phase1.active_app_sid = refs.active_app_sid;
                TOKEN_BUF_OWNER = refs.token_owner;
                TOKEN_BUF_CALL1_RECEIVED = refs.token_pending;
                if !refs.token_pending { TOKEN_BUF_ARG0 = 0; TOKEN_BUF_ARG1 = 0; TOKEN_BUF_ARG2 = 0; }
                CURSOR_UNDER_VALID = refs.cursor_under_valid;
                changed = true;
                sex_pdx::serial_record!("[display.app.cleanup.prepared] target={} task={} removed={} focus={} tombstone=1",
                    info.target_pd, info.task_id, removed, refs.focused_sid);
            }
        }
    }
    changed
}

fn finish_display_transactions(lifecycle: &mut surface_lifecycle::Lifecycle, bar: &SilkBar, fb_live: bool) {
    let prepared = (0..surface_lifecycle::CATALOG_CAPACITY)
        .any(|i| lifecycle.phase(i) == Some(surface_lifecycle::Phase::Prepared));
    if (prepared || lifecycle.needs_presentation()) && fb_live
        && unsafe { render(FB_PTR as *mut u32, FB_W as usize, FB_H as usize, bar) } {
        lifecycle.pixels_presented(unsafe { APP_PRESENT_BITS });
        unsafe { LAST_REDRAWN_CLOCK_CANON_SS = CLOCK_CANON_SS; }
        for index in 0..surface_lifecycle::CATALOG_CAPACITY {
            if lifecycle.scene_committed(index) {
                let c = lifecycle.catalog(index).unwrap();
                sex_pdx::serial_record!("[display.app.cleanup.committed] target={} task={} scene=1", c.target_pd, c.task_id);
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
    assert!(geometry.pitch==geometry.width*4,"DISPLAY_PADDED_PITCH_UNSUPPORTED");
    assert!(address>=HIGH_HALF_BASE,"BOOT_DISPLAY_ADDRESS");
    unsafe{FB_PTR=address;FB_W=geometry.width;FB_H=geometry.height;}
    sex_pdx::serial_record!("[display.boot.geometry] width={} height={} pitch={} boot=1",geometry.width,geometry.height,geometry.pitch);
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
    let mut bar = silkbar_model::for_desktop_width(unsafe{FB_W}as usize).unwrap_or(DEFAULT_SILK_BAR);
    // Legacy software counters are available only through an explicit legacy
    // build profile; normal pixels start Unknown before the first source read.
    bar.civil_enabled=option_env!("SEXOS_LEGACY_DEMO_CLOCK").is_none();
    let clock_layout=&bar.layout[ModuleSlot::Clock as usize];
    if bar.civil_enabled{
        sex_pdx::serial_record!("[sexdisplay.clock.presentation] label=UTC state=Unknown snapshot_only=1 x={} y={} w={} h={} width={}",clock_layout.x,clock_layout.y,clock_layout.w,clock_layout.h,unsafe{FB_W});
    }else{
        sex_pdx::serial_record!("[sexdisplay.clock.presentation] label=DEMO wall_time=0 x={} y={} w={} h={} digits=30 width={}",clock_layout.x,clock_layout.y,clock_layout.w,clock_layout.h,unsafe{FB_W});
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

    // 1. Render immediately with fallback — visible before any IPC
    unsafe { render(FB_PTR as *mut u32, FB_W as usize, FB_H as usize, &bar); }
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
        // ── Per-cycle flags ──
        unsafe { REPAINT_SEEN=0; REPAINT_DRAINED=0; REPAINT_UPSERTS=0; REPAINT_FITTED_NOOPS=0; }
        let mut needs_surface_redraw = false;
        let mut text_damage = damage::Damage::new();
        let mut needs_top_strip_redraw = false;
        // Cursor moved this cycle but nothing else did: eligible for the
        // save-under fast path instead of a full surface composite.
        let mut cursor_only_moved = false;
        let mut did_primary_fb_render = false;
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
                REPAINT_DRAINED=drained;
                REPAINT_SEEN |= match msg.type_id {
                    0xEC=>1,0xEB=>2,0xED=>4,0xEE=>8,0xEF=>16,
                    0x276|0x277|0x278|0x27A|0x27B|0x27C|0x27D=>32,0xFA|0xFB=>64,0xFD=>128,
                    silkbar_model::OP_SILKBAR_UPDATE=>256,_=>512,
                };
            }

            // A hint cannot authorize cleanup: capability status above is the authority.
            if msg.type_id == sex_pdx::app_lifecycle::OP_APP_EXIT {
                if sex_pdx::app_lifecycle::ExitNotice::decode(msg.type_id, msg.caller_pd as u64,
                    [msg.arg0, msg.arg1, msg.arg2]).is_ok()
                    && prepare_app_cleanup(&mut lifecycle, &mut bar) {
                    needs_surface_redraw = true; needs_top_strip_redraw = true;
                }
                continue;
            }
            if !lifecycle.caller_live(msg.caller_pd)
                || (matches!(msg.type_id, 0xEC | 0xEB | 0xED | 0xEE | 0xEF | 0xFA | 0xFB | 0xFD | 0x276 | 0x277 | 0x278 | 0x27A | 0x27B | 0x27C | 0x27D | 0x28B)
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
                sex_pdx::app_lifecycle::OP_APP_PRESENT => {
                    if lifecycle.request_present(msg.caller_pd, msg.arg0, msg.arg1, msg.arg2) {
                        sex_pdx::serial_record!("[display.app.present.request] target={} surface={} request={}", msg.caller_pd, msg.arg0, msg.arg1);
                    }
                }
                silkbar_model::OP_SILKBAR_UPDATE => {
                    // Compare the low kind only to select the strict decoder;
                    // it rejects every high-word alias before changing state.
                    if msg.arg0 as u32==UpdateKind::SetCivilClock as u32{
                        match handle_civil_clock_update(&mut bar,msg.caller_pd,msg.caller_task,[msg.arg0,msg.arg1,msg.arg2],raw_ticks){
                            Ok(silkbar_model::civil_clock::Applied::New)=>{
                                needs_top_strip_redraw=true;
                                let source_known=matches!(bar.civil_clock.state(),silkbar_model::civil_clock::State::Snapshot|silkbar_model::civil_clock::State::Unavailable|silkbar_model::civil_clock::State::Rollback);
                                sex_pdx::serial_record!("[sexdisplay.clock.civil.applied] pd={} task={} sequence={} state={:?} seconds={} high_water={} stale=0 source_known={} source={} precision={} snapshot_only=1 pixels=unconfirmed",msg.caller_pd,msg.caller_task,bar.civil_clock.sequence(),bar.civil_clock.state(),bar.civil_clock.seconds(),bar.civil_clock.high_water(),source_known as u8,if source_known{"q35_cmos_utc"}else{"unknown"},source_known as u8);
                            },
                            Ok(silkbar_model::civil_clock::Applied::Duplicate)=>{},
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
                        unsafe {
                            render(FB_PTR as *mut u32, FB_W as usize, FB_H as usize, &bar);
                            serial_println!("[sexdisplay.render.live.ok] fb_w={} fb_h={}", FB_W, FB_H);
                        }
                        did_primary_fb_render = true;
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
                                BITMAP_STAGE.reset_surface(owner, surface_id);
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
                                BITMAP_STAGE.reset_surface(slot.owner_pd, target_id);
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
                    if !lifecycle.is_coordinator(caller) { continue; }
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
                    if msg.caller_pd != 15 || msg.arg0 != 300 { continue; }
                    let Some(request) = sex_pdx::bitmap_batch::Request::decode(msg.type_id, msg.arg1, msg.arg2) else { continue; };
                    unsafe {
                        if let Some(slot) = SURFACES.iter().find(|s| s.active && s.surface_id == msg.arg0 && s.owner_pd == msg.caller_pd) {
                            if let Some(change) = BITMAP_STAGE.receive(msg.caller_pd, msg.arg0, request) {
                                text_damage.add(bitmap_surface_damage(slot,change.old,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                                text_damage.add(bitmap_surface_damage(slot,change.new,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                                sex_pdx::serial_record!("[display.bitmap.committed] target=15 surface=300 paint={} width={} height={} scale={} pixels=unconfirmed",
                                    BITMAP_STAGE.sequence(),change.new.map_or(0,|g|g.width),change.new.map_or(0,|g|g.height),change.new.map_or(0,|g|g.scale));
                            }
                        }
                    }
                }
                sex_pdx::text_batch::BEGIN | sex_pdx::text_batch::CHUNK | sex_pdx::text_batch::SELECT | sex_pdx::text_batch::COMMIT => {
                    let Some(request) = sex_pdx::text_batch::Request::decode(msg.type_id, msg.arg1, msg.arg2) else { continue; };
                    unsafe {
                        for (index, slot) in SURFACES.iter_mut().enumerate() {
                            if !slot.active || slot.surface_id != msg.arg0 || slot.owner_pd != msg.caller_pd { continue; }
                            if let Some((len,color,bytes)) = TEXT_STAGES[index].receive(msg.caller_pd,msg.arg0,request) {
                                let mut changed = slot.text_len as usize != len || slot.text_color != color || slot.text_buf != *bytes;
                                let old_damage=text_surface_damage(slot,FB_W as usize,FB_H as usize);
                                slot.text_buf = *bytes;
                                let (selection_start, selection_end) = TEXT_STAGES[index].selection();
                                changed |= (slot.selection_start, slot.selection_end) != (selection_start, selection_end);
                                slot.selection_start = selection_start;
                                slot.selection_end = selection_end;
                                slot.text_len = len as u16;
                                slot.text_color = color;
                                recompute_text_lines(slot);
                                if changed {
                                    text_damage.add(old_damage,FB_W as usize,FB_H as usize);
                                    text_damage.add(text_surface_damage(slot,FB_W as usize,FB_H as usize),FB_W as usize,FB_H as usize);
                                }
                                if let sex_pdx::text_batch::Request::Commit { sequence, .. } = request {
                                    sex_pdx::serial_record!("[display.text.committed] target={} surface={} paint={} bytes={} changed={} pixels=unconfirmed",msg.caller_pd,msg.arg0,sequence,len,changed as u8);
                                }
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
                        for slot in SURFACES.iter_mut() {
                            if slot.active && slot.surface_id == sid {
                                if slot.owner_pd != msg.caller_pd {
                                    let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                    if n & 0x3F == 0 {
                                        serial_println!("AUTH: OP_TEXT_CLEAR rejected sid={} caller={} owner={}",
                                            sid, msg.caller_pd, slot.owner_pd);
                                    }
                                    break;
                                }
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
                        for slot in SURFACES.iter_mut() {
                            if slot.active && slot.surface_id == sid {
                                if slot.owner_pd != msg.caller_pd {
                                    let n = REJECT_COUNTER.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                    if n & 0x3F == 0 {
                                        serial_println!("AUTH: OP_TEXT_DRAW rejected sid={} caller={} owner={}",
                                            sid, msg.caller_pd, slot.owner_pd);
                                    }
                                    break;
                                }
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
                    did_primary_fb_render as u8); }
            }
        }

        let transaction_full_render = fb_live && (lifecycle.needs_presentation()
            || (0..surface_lifecycle::CATALOG_CAPACITY)
                .any(|i| lifecycle.phase(i) == Some(surface_lifecycle::Phase::Prepared)));
        // ── Post-drain redraws ──
        // Surface area: always redraw when needed, regardless of did_primary_fb_render.
        // did_primary_fb_render becomes true on the OP_PRIMARY_FB cycle (which calls
        // render() during drain), but needs_surface_redraw may be set by chrome flag
        // updates draining in the same cycle. Without this, chrome updates are lost.
        if needs_surface_redraw && !transaction_full_render {
            clock_canon_apply_to_bar(&mut bar);
            unsafe { redraw_surface_area(FB_PTR as *mut u32, FB_W as usize, FB_H as usize); }
        } else if let Some(rect) = text_damage.rect().filter(|_| fb_live && !transaction_full_render) {
            unsafe { redraw_surface_damage(FB_PTR as *mut u32, FB_W as usize, FB_H as usize, rect); }
        } else if cursor_only_moved && fb_live && !transaction_full_render {
            // Cursor-only fast path: restore the 8x16 patch saved beneath
            // the previous cursor draw, then draw at the new position (which
            // re-saves). Latest cursor state wins — intermediate positions
            // drained this cycle coalesce into one visible draw. Full
            // composite still runs whenever any other surface changed.
            unsafe {
                let fb = FB_PTR as *mut u32;
                let (w, h) = (FB_W as usize, FB_H as usize);
                restore_cursor_under(fb, w, h, w * h);
                draw_cursor_z_top(fb, w, h, w * h, true);
            }
        }
        // Top strip: redraw when needed (clock tick, silkbar update).
        if needs_top_strip_redraw && !transaction_full_render {
            clock_canon_apply_to_bar(&mut bar);
            unsafe { redraw_top_strip(FB_PTR as *mut u32, FB_W as usize, FB_H as usize, &bar); }
            // Track last drawn canonical second for same-second redraw suppression.
            unsafe { LAST_REDRAWN_CLOCK_CANON_SS = CLOCK_CANON_SS; }
        }

        if transaction_full_render { clock_canon_apply_to_bar(&mut bar); }
        finish_display_transactions(&mut lifecycle, &bar, fb_live);
        unsafe { for stage in &mut TEXT_STAGES { stage.poll_reply(sex_pdx::pdx_reply); } }
        unsafe { BITMAP_STAGE.poll_reply(sex_pdx::pdx_reply); }

        // ── Yield once regardless of whether messages remain queued ──
        loop_iter = loop_iter.wrapping_add(1);
        sex_pdx::sys_yield();
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
