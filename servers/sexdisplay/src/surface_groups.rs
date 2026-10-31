// SPDX-License-Identifier: GPL-3.0-only
//! Fixed boot associations activated only by the authenticated lifecycle catalog.
use crate::surface_lifecycle::{Catalog, Surface, CATALOG_CAPACITY};
#[derive(Clone, Copy)]
struct Group { catalog: Catalog, frame_seen: bool }
impl Group {
    fn parent_required(&self) -> bool {
        crate::app_catalog::by_surface(self.catalog.target_pd,self.catalog.surface_id).is_some_and(|i|!i.children.is_empty())
    }
}
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
                if let Some(app)=crate::app_catalog::by_surface(g.catalog.target_pd,g.catalog.surface_id).filter(|i|!i.children.is_empty()) {
                    let index=if child.surface_id==app.surface{0}else{app.children.iter().position(|c|*c==child.surface_id).map_or(3,|p|p+1)};
                    let p=sex_pdx::spindle_view::Layout::new(frame.w,frame.h).panes[index];
                    child.x=frame.x.saturating_add(p.x as i32);child.y=frame.y.saturating_add(p.y as i32);child.w=p.w;child.h=p.h;
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
    /// Spindle's exact catalog children are retained data, never standalone
    /// windows, even before the sanctioned Shell parent has first appeared.
    /// Other groups retain their existing early-surface presentation policy.
    pub fn independent(&self, s: &Surface) -> bool {
        s.active && !self.child_group(s).map_or(false, |g| g.frame_seen || g.parent_required())
    }
    /// Admission diagnostic only. No visibility, fitting or PRESENT completion
    /// follows from the existence of a catalog or an awaiting child.
    pub fn awaiting_parent(&self, s: &Surface) -> Option<u64> {
        if !s.active {return None;}
        self.child_group(s).filter(|g|g.parent_required()&&!g.frame_seen)
            .and_then(|g|g.catalog.frame_surface_id())
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

// Exact composition changes only at clamped surface edges and bound frame body
// edges. References belong to one immutable render row, never another drain.
#[derive(Clone, Copy)]
pub struct PixelSource<'a> {
    pub surface: &'a Surface,
    pub content: &'a Surface,
    pub chrome: bool,
}
#[derive(Clone, Copy)]
struct RowRun<'a> { end: usize, source: Option<PixelSource<'a>> }
pub struct RowWinners<'a> { runs: [RowRun<'a>; MAX_ROW_RUNS], count: usize, cursor: usize }
const MAX_ROW_RUNS: usize = crate::surface_lifecycle::MAX_SURFACES * 4 + 1;
impl<'a> RowWinners<'a> {
    #[inline(always)]
    pub fn source(&mut self, x: usize) -> Option<PixelSource<'a>> {
        while self.cursor + 1 < self.count && x >= self.runs[self.cursor].end { self.cursor += 1; }
        if self.cursor < self.count && x < self.runs[self.cursor].end { self.runs[self.cursor].source } else { None }
    }
    pub fn selected(&mut self, x: usize) -> Option<u64> { self.source(x).map(|s| s.surface.surface_id) }
}
impl FrameGroups {
    pub fn row_winners<'a>(&self, surfaces: &'a [Surface], focused: u64, y: usize, w: usize, h: usize) -> Option<RowWinners<'a>> {
        if surfaces.len() > crate::surface_lifecycle::MAX_SURFACES || w == 0 || y >= h { return None; }
        let mut edges = [0usize; MAX_ROW_RUNS + 1]; edges[1] = w; let mut n = 2;
        for s in surfaces.iter().filter(|s| s.active) {
            let (sx,sy,sw,sh) = rect(s,w,h);
            if sw != 0 && sh != 0 && y >= sy && y-sy < sh {
                edges[n] = sx; edges[n+1] = sx+sw; n += 2;
                // Even an unfitted child cannot replace pixels outside the
                // parent's body. Splitting here also preserves tiny/clipped frames.
                if self.is_frame(s) {
                    let body_y = sy.saturating_add(36);
                    if y >= body_y && y-body_y < sh.saturating_sub(44) {
                        let left = sx.saturating_add(8).min(w);
                        edges[n] = left;
                        edges[n+1] = left.saturating_add(sw.saturating_sub(16)).min(w);
                        n += 2;
                    }
                }
            }
        }
        for i in 1..n { let value=edges[i]; let mut j=i; while j>0 && edges[j-1]>value { edges[j]=edges[j-1]; j-=1; } edges[j]=value; }
        let mut result = RowWinners { runs: [RowRun { end: 0, source: None }; MAX_ROW_RUNS], count: 0, cursor: 0 };
        for i in 1..n { if edges[i] > edges[i-1] {
            let x = edges[i-1];
            let source = self.winner(surfaces,focused,x,y,w,h).map(|surface| PixelSource {
                surface, content: self.content(surface,surfaces,x,y,w,h),
                chrome: surface.surface_id == focused || self.is_frame(surface),
            });
            result.runs[result.count] = RowRun { end: edges[i], source }; result.count += 1;
        } }
        Some(result)
    }
}
