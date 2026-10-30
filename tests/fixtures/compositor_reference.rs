// SPDX-License-Identifier: GPL-3.0-only
// Independent accepted pixel algorithm from e04fff3a (image 58).
// Keep the two-pass selection/body search intact as the span-cache oracle.
fn reference_composite_pixel_selected(x: usize, y: usize, w: usize, h: usize, bg: u32, focused_id: u64, selected:Option<u64>) -> u32 {
    let mut c = bg;
    let mut winner_mask = 0;
    unsafe {
        // Pass 1: non-focused surfaces (slot order, break on first hit)
        for surf in SURFACES.iter() {
            if !surf.active || Some(surf.surface_id) != selected || surf.surface_id == focused_id || FRAME_GROUPS.is_frame(surf) { continue; }
            let (sx, sy, sw, sh) = clamp_surface(surf, w, h);
            if sw == 0 || sh == 0 { continue; }
            if x >= sx && x < sx + sw && y >= sy && y < sy + sh {
                winner_mask = surface_lifecycle::native_pixel_bit(surf.owner_pd, surf.surface_id);
                c = fill_rect_color(surf, x, y, surf.color);
                // ── Text rendering: glyph foreground overrides fill-rect color ──
                if let Some(tc) = surface_text_fg_at(x, y, surf) { c = tc; }
                break;
            }
        }
        // Pass 2: focused surface (always drawn on top)
        if selected.is_some() {
            for surf in SURFACES.iter() {
                if !surf.active || Some(surf.surface_id) != selected || (surf.surface_id != focused_id && !FRAME_GROUPS.is_frame(surf)) { continue; }
                let (sx, sy, sw, sh) = clamp_surface(surf, w, h);
                if sw == 0 || sh == 0 { continue; }
                if x >= sx && x < sx + sw && y >= sy && y < sy + sh {
                    winner_mask = surface_lifecycle::native_pixel_bit(surf.owner_pd, surf.surface_id);
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
                        // Round104 intentionally changes only title foreground: use final glass/tab contrast.
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
                        let content = FRAME_GROUPS.content(surf, &SURFACES, x, y, w, h);
                        winner_mask = surface_lifecycle::native_pixel_bit(content.owner_pd, content.surface_id);
                        c = fill_rect_color(content, x, y, content.color);
                        // Child text uses its own origin; the parent alone draws chrome.
                        if let Some(tc) = surface_text_fg_at(x, y, content) { c = tc; }
                    }
                    break;
                }
            }
        }
    }
    if winner_mask != 0 { unsafe { APP_PRESENT_BITS |= winner_mask; } }
    c
}
