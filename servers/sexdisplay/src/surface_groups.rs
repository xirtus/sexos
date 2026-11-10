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
#[derive(Clone, Copy)]
pub struct FrameGroups { metrics:sex_pdx::desktop_style::chrome::Metrics, groups: [Option<Group>; CATALOG_CAPACITY] }
impl FrameGroups {
    pub const fn new() -> Self { Self { metrics:sex_pdx::desktop_style::chrome::Metrics::new(1), groups: [None; CATALOG_CAPACITY] } }
    pub fn set_scale(&mut self,scale:u32){self.metrics=sex_pdx::desktop_style::chrome::Metrics::new(scale);}
    pub fn metrics(&self)->sex_pdx::desktop_style::chrome::Metrics{self.metrics}
    pub fn rect(&self,s:&Surface,w:usize,h:usize)->(usize,usize,usize,usize){rect_below(s,w,h,self.metrics.bar()as usize)}
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
            let body=self.metrics.body(frame.w,frame.h);
            let bx=frame.x.saturating_add(body.x as i32);let by=frame.y.saturating_add(body.y as i32);
            let(bw,bh)=(body.w,body.h);
            for child in surfaces.iter_mut().filter(|s| s.active && s.owner_pd == g.catalog.target_pd && g.catalog.is_content(s.surface_id)) {
                if let Some(app)=crate::app_catalog::by_surface(g.catalog.target_pd,g.catalog.surface_id).filter(|i|!i.children.is_empty()) {
                    let index=if child.surface_id==app.surface{0}else{app.children.iter().position(|c|*c==child.surface_id).map_or(3,|p|p+1)};
                    let p=sex_pdx::spindle_view::Layout::with_scale(frame.w,frame.h,self.metrics.scale()).panes[index];
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
    /// A program row's own surface is its frame: it keeps its title bar when unfocused,
    /// so several open programs can each be told apart and closed with the mouse.
    pub fn is_program_frame(&self, s: &Surface) -> bool {
        crate::app_catalog::by_surface(s.owner_pd, s.surface_id)
            .is_some_and(|i| i.app == crate::app_catalog::App::Program && i.shell_surface.is_none())
    }
    /// Spindle's exact catalog children are retained data, never standalone
    /// windows, even before the sanctioned Shell parent has first appeared.
    /// Other groups retain their existing early-surface presentation policy.
    pub fn independent(&self, s: &Surface) -> bool {
        s.active && !self.child_group(s).map_or(false, |g| g.frame_seen || g.parent_required())
    }
    /// Only independent windows have a silhouette. Exact fitted children share
    /// the parent's mask and never acquire four extra sets of rounded corners.
    pub fn coverage(&self,s:&Surface,x:usize,y:usize)->u8{
        if !s.active||y<self.metrics.bar()as usize{return 0;}
        let (lx,ly)=(x as i64-s.x as i64,y as i64-s.y as i64);
        if lx<0||ly<0||lx>=s.w as i64||ly>=s.h as i64{return 0;}
        if !self.independent(s){return 255;}
        self.metrics.rounded(s.w,s.h).coverage(lx,ly)
    }
    pub fn row_span(&self,s:&Surface,y:usize,w:usize,h:usize,opaque:bool)->(usize,usize){
        let(sx,sy,sw,sh)=self.rect(s,w,h);
        if sw==0||sh==0||y<sy||y-sy>=sh{return(0,0);}
        if !self.independent(s){return(sx,sx+sw);}
        let(a,b)=self.metrics.rounded(s.w,s.h).row(y as i64-s.y as i64,opaque);
        let clip=|v:i64|v.clamp(0,w as i64)as usize;
        (clip(s.x as i64+a as i64),clip(s.x as i64+b as i64))
    }
    pub fn opaque_run(&self,s:&Surface,x0:usize,x1:usize,y:usize,w:usize,h:usize)->bool{
        let(a,b)=self.row_span(s,y,w,h,true);x0<x1&&x0>=a&&x1<=b
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
        self.winner_below(surfaces,focused,None,x,y,w,h)
    }
    /// Actual next visible owner, borrowed from this same immutable scene.
    /// A foreign/stale `above` reference cannot resume another layer traversal.
    pub fn winner_below<'a>(&self,surfaces:&'a[Surface],focused:u64,above:Option<&Surface>,
        x:usize,y:usize,w:usize,h:usize)->Option<&'a Surface>{
        // A focus reference raises only its authenticated, observed parent.
        let focused = surfaces.iter().find(|s| s.active && s.surface_id == focused)
            .and_then(|s| self.child_group(s)).filter(|g| g.frame_seen)
            .and_then(|g| g.catalog.frame_surface_id()).unwrap_or(focused);
        let key = |s: &Surface| sex_pdx::desktop_stack::key(focused,s.owner_pd,s.surface_id);
        let floor = match above {
            Some(above) => Some(key(surfaces.iter().find(|s| core::ptr::eq(*s,above))?)),
            None => None,
        };
        // Constant-time shared ranks, independent of allocation/slot reuse.
        // The fixed scan borrows only this scene; no sorting arena or lookup
        // based on the latest mutable Shell state enters a prepared frame.
        surfaces.iter().filter(|s| self.independent(s) && contains(self.rect(s,w,h),x,y)
            && self.coverage(s,x,y)!=0 && floor.is_none_or(|floor| key(s)>floor))
            .min_by_key(|s| key(s))
    }
    /// The frame owns z-order/chrome. Only exact app-owned children can replace
    /// its body pixels; clipping never changes the child's text coordinate origin.
    pub fn content<'a>(&self, frame: &'a Surface, surfaces: &'a [Surface],
        x: usize, y: usize, w: usize, h: usize) -> &'a Surface {
        let Some(g) = self.groups.iter().flatten().find(|g| g.frame_seen
            && frame.owner_pd == g.catalog.coordinator_pd && Some(frame.surface_id) == g.catalog.frame_surface_id()) else { return frame; };
        let b=self.metrics.body(frame.w,frame.h);
        let(lx,ly)=(x as i64-frame.x as i64,y as i64-frame.y as i64);
        if lx<0||ly<0||lx>u32::MAX as i64||ly>u32::MAX as i64||!b.contains(lx as u32,ly as u32){return frame;}
        surfaces.iter().find(|s| s.active && s.owner_pd == g.catalog.target_pd && g.catalog.is_content(s.surface_id)
            && contains(self.rect(s,w,h),x,y)).unwrap_or(frame)
    }
}
pub fn rect(s:&Surface,w:usize,h:usize)->(usize,usize,usize,usize){rect_below(s,w,h,crate::bar_height())}
fn rect_below(s: &Surface,w: usize,h: usize,bar:usize) -> (usize,usize,usize,usize) {
    let left=(s.x as i64).clamp(0,w as i64);
    let top=(s.y as i64).clamp(bar.min(h)as i64,h as i64);
    let right=(s.x as i64+s.w as i64).clamp(left,w as i64);
    let bottom=(s.y as i64+s.h as i64).clamp(top,h as i64);
    (left as usize,top as usize,(right-left)as usize,(bottom-top)as usize)
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
const MAX_ROW_RUNS: usize = crate::surface_lifecycle::MAX_SURFACES * 6 + 1;
impl<'a> RowWinners<'a> {
    #[inline(always)]
    pub fn source(&mut self, x: usize) -> Option<PixelSource<'a>> {
        while self.cursor + 1 < self.count && x >= self.runs[self.cursor].end { self.cursor += 1; }
        if self.cursor < self.count && x < self.runs[self.cursor].end { self.runs[self.cursor].source } else { None }
    }
    /// End and source of the run holding `x`: every pixel to `end` has this source.
    #[inline(always)]
    pub fn run(&mut self, x: usize) -> (usize, Option<PixelSource<'a>>) {
        while self.cursor + 1 < self.count && x >= self.runs[self.cursor].end { self.cursor += 1; }
        if self.cursor < self.count && x < self.runs[self.cursor].end {
            (self.runs[self.cursor].end, self.runs[self.cursor].source)
        } else { (usize::MAX, None) }
    }
    pub fn selected(&mut self, x: usize) -> Option<u64> { self.source(x).map(|s| s.surface.surface_id) }
}
impl FrameGroups {
    pub fn row_winners<'a>(&self, surfaces: &'a [Surface], focused: u64, y: usize, w: usize, h: usize) -> Option<RowWinners<'a>> {
        if surfaces.len() > crate::surface_lifecycle::MAX_SURFACES || w == 0 || y >= h { return None; }
        let mut edges = [0usize; MAX_ROW_RUNS + 1]; edges[1] = w; let mut n = 2;
        for s in surfaces.iter().filter(|s| s.active) {
            let (_sx,sy,sw,sh) = self.rect(s,w,h);
            if sw != 0 && sh != 0 && y >= sy && y-sy < sh {
                let(a,b)=self.row_span(s,y,w,h,false);
                edges[n] = a; edges[n+1] = b; n += 2;
                let(oa,ob)=self.row_span(s,y,w,h,true);
                if(oa,ob)!=(a,b){edges[n]=oa;edges[n+1]=ob;n+=2;}
                // Even an unfitted child cannot replace pixels outside the
                // parent's body. Splitting here also preserves tiny/clipped frames.
                if self.is_frame(s) {
                    let b=self.metrics.body(s.w,s.h);
                    let body_y=s.y as i64+b.y as i64;
                    if y as i64>=body_y&&(y as i64)-body_y<(b.h as i64){
                        let left=(s.x as i64+b.x as i64).clamp(0,w as i64)as usize;
                        edges[n] = left;
                        edges[n+1]=(s.x as i64+b.x as i64+b.w as i64).clamp(left as i64,w as i64)as usize;
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
                chrome: surface.surface_id == focused || self.is_frame(surface) || self.is_program_frame(surface),
            });
            result.runs[result.count] = RowRun { end: edges[i], source }; result.count += 1;
        } }
        Some(result)
    }
}
