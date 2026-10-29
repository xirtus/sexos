// SPDX-License-Identifier: GPL-3.0-only
//! Fixed boot associations activated only by the authenticated lifecycle catalog.
use crate::surface_lifecycle::{Catalog, Surface, CATALOG_CAPACITY};
#[derive(Clone, Copy)]
struct Group { catalog: Catalog, frame_seen: bool }
pub struct FrameGroups { groups: [Option<Group>; CATALOG_CAPACITY] }
impl FrameGroups {
    pub const fn new() -> Self { Self { groups: [None; CATALOG_CAPACITY] } }
    pub fn register(&mut self, catalog: Catalog) {
        if catalog.frame_surface_id().is_none() { return; }
        if self.groups.iter().flatten().any(|g| g.catalog == catalog) { return; }
        if let Some(slot) = self.groups.iter_mut().find(|g| g.is_none()) {
            *slot = Some(Group { catalog, frame_seen: false });
        }
    }
    /// Observe each successful admission, including create+hide within one drain.
    /// The binding persists after a frame slot is cleared or reused.
    pub fn observe(&mut self, surfaces: &[Surface]) {
        for g in self.groups.iter_mut().flatten() {
            g.frame_seen |= surfaces.iter().any(|s| s.active && s.owner_pd == g.catalog.coordinator_pd
                && Some(s.surface_id) == g.catalog.frame_surface_id());
        }
    }
    /// Frame geometry is authoritative after binding, including late app startup
    /// creates. This also makes text layout and fill coordinates match clipping.
    pub fn fit_children(&self, surfaces: &mut [Surface]) {
        for g in self.groups.iter().flatten().filter(|g| g.frame_seen) {
            let Some(frame) = surfaces.iter().find(|s| s.active && s.owner_pd == g.catalog.coordinator_pd
                && Some(s.surface_id) == g.catalog.frame_surface_id()).copied() else { continue; };
            let bx=frame.x.saturating_add(8); let by=frame.y.saturating_add(36);
            let bw=frame.w.saturating_sub(16); let bh=frame.h.saturating_sub(44);
            for child in surfaces.iter_mut().filter(|s| s.active && s.owner_pd == g.catalog.target_pd && g.catalog.is_content(s.surface_id)) {
                if g.catalog.target_pd == 12 {
                    let col = matches!(child.surface_id,160|162) as u32;
                    let row = matches!(child.surface_id,161|162) as u32;
                    let cw=bw.saturating_sub(8)/2; let ch=bh.saturating_sub(8)/2;
                    child.x=bx.saturating_add((col*(cw+8)) as i32);
                    child.y=by.saturating_add((row*(ch+8)) as i32);
                    child.w=if col==0 {cw} else {bw.saturating_sub(cw+8)};
                    child.h=if row==0 {ch} else {bh.saturating_sub(ch+8)};
                } else { child.x=bx;child.y=by;child.w=bw;child.h=bh; }
            }
        }
    }
    fn child_group(&self, s: &Surface) -> Option<&Group> {
        self.groups.iter().flatten().find(|g| g.catalog.target_pd == s.owner_pd && g.catalog.is_content(s.surface_id))
    }
    pub fn is_frame(&self, s: &Surface) -> bool {
        self.groups.iter().flatten().any(|g| g.catalog.coordinator_pd == s.owner_pd
            && g.catalog.frame_surface_id() == Some(s.surface_id))
    }
    pub fn independent(&self, s: &Surface) -> bool {
        s.active && !self.child_group(s).map_or(false, |g| g.frame_seen)
    }
    pub fn winner<'a>(&self, surfaces: &'a [Surface], focused: u64,
        x: usize, y: usize, w: usize, h: usize) -> Option<&'a Surface> {
        // The shell's modal launcher must remain above app focus without
        // stealing that focus (its actions still target the previous window).
        if let Some(overlay)=surfaces.iter().find(|s|s.active && s.owner_pd==3
            && s.surface_id==152 && contains(rect(s,w,h),x,y)) {return Some(overlay);}
        // A focus reference to a bound child raises its actual frame, never a ghost child.
        let focused = surfaces.iter().find(|s| s.active && s.surface_id == focused)
            .and_then(|s| self.child_group(s)).filter(|g| g.frame_seen)
            .and_then(|g| g.catalog.frame_surface_id()).unwrap_or(focused);
        let hit = |s: &&Surface| self.independent(s) && contains(rect(s,w,h),x,y);
        surfaces.iter().filter(|s| s.surface_id == focused).find(hit)
            .or_else(|| surfaces.iter().filter(|s| s.surface_id != focused).find(hit))
    }
    /// The frame owns z-order/chrome. Only exact app-owned children can replace
    /// its body pixels; clipping never changes the child's text coordinate origin.
    pub fn content<'a>(&self, frame: &'a Surface, surfaces: &'a [Surface],
        x: usize, y: usize, w: usize, h: usize) -> &'a Surface {
        let Some(g) = self.groups.iter().flatten().find(|g| g.frame_seen
            && frame.owner_pd == g.catalog.coordinator_pd && Some(frame.surface_id) == g.catalog.frame_surface_id()) else { return frame; };
        let (sx,sy,sw,sh) = rect(frame,w,h);
        let body = (sx.saturating_add(8),sy.saturating_add(36),sw.saturating_sub(16),sh.saturating_sub(44));
        if !contains(body,x,y) { return frame; }
        surfaces.iter().find(|s| s.active && s.owner_pd == g.catalog.target_pd && g.catalog.is_content(s.surface_id)
            && contains(rect(s,w,h),x,y)).unwrap_or(frame)
    }
}
pub fn rect(s: &Surface,w: usize,h: usize) -> (usize,usize,usize,usize) {
    let x=(s.x.max(0) as usize).min(w.saturating_sub(1));
    let y=(s.y.max(50) as usize).min(h.saturating_sub(1));
    (x,y,(s.w as usize).min(w.saturating_sub(x)),(s.h as usize).min(h.saturating_sub(y)))
}
pub fn contains((x,y,w,h):(usize,usize,usize,usize),px:usize,py:usize)->bool {
    px >= x && px-x < w && py >= y && py-y < h
}
