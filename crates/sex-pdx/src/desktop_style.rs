//! Shared native text metrics and focus ink; integer pixels only.
pub const INSET: u32 = 8;
pub const TEXT_TOP: u32 = 12;
pub const ASCII_FIRST: u8 = 32;
pub const ASCII_LAST: u8 = 126;
pub const GLYPH_WIDTH: u32 = 5;
pub const GLYPH_HEIGHT: u32 = 7;
pub const ADVANCE: u32 = 6;
pub const LINE: u32 = 9;
pub const TEXT: u32 = 0x00E0F0FF;
pub const FOCUS: u32 = 0x00E9D36A;
/// Shell (the window manager) to Display: make a surface's text larger or smaller.
/// arg0 = surface id, arg1 = +1 larger, -1 (as u64) smaller, 0 back to the default.
pub const OP_TEXT_ZOOM: u64 = 0x2D4;
/// Steps of zoom either way; the text scale stays within 1..=MAX_TEXT_SCALE.
pub const MAX_TEXT_SCALE: u32 = 5;
/// Shell (the window manager) to Display: step the whole screen's scale.
/// arg0 = +1 larger, -1 (as u64) smaller, 0 back to what BOOT chose for the
/// screen. Display logs `[display.scale]` with the scale now in force.
/// arg1 = nonzero sequence for a retained asynchronous post-publication reply;
/// zero preserves legacy fire-and-forget behavior. The enqueue return is not
/// an applied-scale receipt. Replies use `chrome::scale_receipt` packing.
pub const OP_DISPLAY_SCALE: u64 = 0x2D5;
/// The interface scale in force in this service: 1, or 2 and 3 on HiDPI
/// screens. Text sizes, the cursor and window chrome multiply by it.
static UI_SCALE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(1);
pub fn ui_scale() -> u32 {
    UI_SCALE.load(core::sync::atomic::Ordering::Relaxed)
}
pub fn set_ui_scale(scale: u32) {
    UI_SCALE.store(chrome::clamp_scale(scale), core::sync::atomic::Ordering::Relaxed);
}
pub const fn scale(width: u32) -> u32 {
    if width >= 256 {
        2
    } else {
        1
    }
}

/// Shared integer chrome geometry. Coordinates are window-local device pixels;
/// this data carries no surface, monitor or input authority.
pub mod chrome {
    pub const MAX_SCALE: u32 = 3;
    // Reply payload is disjoint from text/lifecycle receipts. Sequence zero is
    // reserved for legacy fire-and-forget callers; a request uses arg1.
    const RECEIPT_TAG: u64 = 0x5343_0000_0000_0000;
    const SEQUENCE_MASK: u64 = (1u64 << 46)-1;
    pub const fn scale_receipt(sequence:u64, scale:u32)->Option<u64> {
        if sequence==0 || sequence>SEQUENCE_MASK || scale<1 || scale>3 {return None;}
        Some(RECEIPT_TAG | (sequence<<2) | scale as u64)
    }
    /// One outstanding scale request per Shell. Queue admission never changes
    /// applied geometry. A stale, forged or unrelated reply cannot commit it.
    pub struct ScaleClient { sequence:u64, pending:Option<(u64,i64,bool)> }
    impl ScaleClient {
        pub const fn new()->Self {Self::for_incarnation(0)}
        /// Kernel-owned own-service restart count. A restart keeps task id 3,
        /// so caller_task alone cannot fence an old run's delayed reply.
        pub const fn for_incarnation(restarts:u64)->Self {
            Self{sequence:if restarts<(1<<14){restarts<<32}else{SEQUENCE_MASK},pending:None}
        }
        pub fn begin(&mut self,step:i64)->bool {
            if self.pending.is_some(){return false;}
            // Never wrap a sequence into another incarnation or reuse one.
            if self.sequence&0xffff_ffff==0xffff_ffff{return false;}
            self.sequence+=1;
            self.pending=Some((self.sequence,step,false));true
        }
        pub fn request(&self)->Option<(u64,i64)> {
            self.pending.and_then(|(sequence,step,sent)|if sent{None}else{Some((sequence,step))})
        }
        pub fn admitted(&mut self){if let Some(p)=self.pending.as_mut(){p.2=true;}}
        pub fn cancel(&mut self){self.pending=None;}
        pub fn pending(&self)->bool {self.pending.is_some()}
        pub fn receive(&mut self,caller:u32,display:u32,value:u64)->Option<u32> {
            let (sequence,_,sent)=self.pending?;
            if caller!=display || !sent{return None;}
            let scale=(value&3)as u32;
            if scale_receipt(sequence,scale)!=Some(value){return None;}
            self.pending=None;Some(scale)
        }
    }
    /// Display retains the admitted reply until a complete CPU publication and
    /// successful transport. Failure/retry never turns admission into visibility.
    pub struct ScalePublisher { subject:Option<(u32,u32,u64)>, pending:Option<(u32,u64,bool)> }
    impl ScalePublisher {
        pub const fn new()->Self {Self{subject:None,pending:None}}
        /// Register only after the held catalog authorized the exact Shell
        /// main task. Idempotence preserves a pending receipt; a newer own-
        /// service restart count fences the old run and its pending reply.
        pub fn register(&mut self,target:u32,task:u32,epoch:u64)->Option<bool>{
            if target==0||task==0||epoch>=(1<<14){return None;}
            if let Some((pd,old_task,old_epoch))=self.subject {
                if pd!=target||old_task!=task||epoch<old_epoch{return None;}
                if epoch==old_epoch{return Some(false);}
            }
            self.subject=Some((target,task,epoch));self.pending=None;Some(true)
        }
        pub fn allows_change(&self,target:u32,task:u32,sequence:u64)->bool {
            let Some((pd,main,epoch))=self.subject else{return false;};
            !self.pending()&&pd==target&&main==task
                &&(sequence==0||sequence>>32==epoch)
        }
        pub fn admit(&mut self,target:u32,task:u32,sequence:u64,scale:u32)->bool {
            if !self.allows_change(target,task,sequence){return false;}
            let Some(value)=scale_receipt(sequence,scale)else{return false;};
            self.pending=Some((target,value,false));true
        }
        pub fn needs_publication(&self)->bool {self.pending.is_some_and(|p|!p.2)}
        pub fn published(&mut self){if let Some(p)=self.pending.as_mut(){p.2=true;}}
        pub fn reply(&self)->Option<(u32,u64)> {self.pending.and_then(|(pd,v,ready)|if ready{Some((pd,v))}else{None})}
        pub fn sent(&mut self){self.pending=None;}
        pub fn pending(&self)->bool {self.pending.is_some()}
    }

    pub const BAR: u32 = 28; // includes the bottom separator
    pub const TITLE: u32 = 26;
    pub const TARGET: u32 = 24;
    pub const UI_FONT: u32 = 13;
    pub const UI_LINE:u32=16;
    pub const RESIZE: u32 = 8;
    pub const OUTER_GAP: u32 = 8;
    pub const INNER_GAP: u32 = 6;
    pub const FLOAT_RADIUS: u32 = 12;
    pub const TILE_RADIUS: u32 = 10;

    pub const fn clamp_scale(scale: u32) -> u32 {
        if scale < 1 { 1 } else if scale > MAX_SCALE { MAX_SCALE } else { scale }
    }
    /// Reset follows the immutable BOOT scale; nonzero steps move one notch.
    pub const fn step_scale(before: u32, boot: u32, step: i64) -> u32 {
        if step == 0 { return clamp_scale(boot); }
        let before=clamp_scale(before);
        if step > 0 { clamp_scale(before+1) } else { clamp_scale(before-1) }
    }
    /// Fit policy supplied by the data model; an excessive manual/EDID scale
    /// never panics the renderer or hides module targets outside the screen.
    pub const fn fit_scale(width:u32,requested:u32,min_logical:u32)->u32{
        let mut scale=clamp_scale(requested);
        while scale>1&&width<min_logical.saturating_mul(scale){scale-=1;}
        scale
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Rect { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }
    impl Rect {
        pub const fn contains(self, x: u32, y: u32) -> bool {
            x >= self.x && y >= self.y && x-self.x < self.w && y-self.y < self.h
        }
    }
    /// Shared panel silhouette only; coordinates never confer window authority.
    /// Original signed geometry is translated by the caller before clipping.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Rounded { width:u32, height:u32, radius:u32 }
    pub const PANEL_RADIUS:u32=12;
    impl Rounded {
        pub const fn new(width:u32,height:u32,scale:u32)->Self{
            let mut radius=PANEL_RADIUS*clamp_scale(scale);
            if width/2<radius{radius=width/2;}
            if height/2<radius{radius=height/2;}
            Self{width,height,radius}
        }
        pub const fn radius(self)->u32{self.radius}
        #[inline]
        pub fn coverage(self,x:i64,y:i64)->u8{
            if x<0||y<0||x>=self.width as i64||y>=self.height as i64{return 0;}
            let (x,y)=(x as u32,y as u32);
            let cx=x.min(self.width-1-x);let cy=y.min(self.height-1-y);
            if self.radius==0||cx>=self.radius||cy>=self.radius{return 255;}
            corner_alpha(self.radius,cx,cy)
        }
        /// Half-open local span of any visible (or entirely opaque) pixels.
        /// Straight middle rows need no corner lookup or scratch allocation.
        pub fn row(self,y:i64,opaque:bool)->(u32,u32){
            if y<0||y>=self.height as i64||self.width==0{return(0,0);}
            let cy=(y as u32).min(self.height-1-y as u32);
            if self.radius==0||cy>=self.radius{return(0,self.width);}
            let mut cut=0;
            while cut<self.radius{
                let a=corner_alpha(self.radius,cut,cy);
                if (opaque&&a==255)||(!opaque&&a!=0){break;}
                cut+=1;
            }
            (cut,self.width-cut)
        }
    }
    // Four-by-four exact integer coverage. Common UI radii use one immutable
    // 2,016-byte table, not supersampling work in a per-pixel shader. Tiny
    // clamped frames retain the same bounded rule without another arena.
    const fn sample_corner(radius:u32,x:u32,y:u32)->u8{
        let r=radius as i64*8;let mut on=0u32;let mut sy=1;
        while sy<8{let mut sx=1;while sx<8{
            let dx=r-(x as i64*8+sx);let dy=r-(y as i64*8+sy);
            if dx*dx+dy*dy<=r*r{on+=1;}
            sx+=2;
        }sy+=2;}
        ((on*255+8)/16)as u8
    }
    const fn corner_table()->[u8;2016]{
        let mut result=[0;2016];let mut scale=1;let mut at=0;
        while scale<=3{let r=PANEL_RADIUS*scale;let mut y=0;
            while y<r{let mut x=0;while x<r{
                result[at]=sample_corner(r,x,y);at+=1;x+=1;
            }y+=1;}scale+=1;
        }result
    }
    static CORNER_ALPHA:[u8;2016]=corner_table();
    #[inline]
    fn corner_alpha(radius:u32,x:u32,y:u32)->u8{
        let offset=match radius{12=>0,24=>144,36=>720,_=>return sample_corner(radius,x,y)};
        CORNER_ALPHA[offset+(y*radius+x)as usize]
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Metrics { scale: u32 }
    impl Metrics {
        pub const fn new(scale: u32) -> Self { Self { scale: clamp_scale(scale) } }
        pub const fn scale(self) -> u32 { self.scale }
        pub const fn rounded(self,width:u32,height:u32)->Rounded{Rounded::new(width,height,self.scale)}
        /// Initial pointer selection, clipped to the same applied screen and
        /// protected bar as rendering. Captured drag/release uses its owner.
        pub fn panel_hit(self,frame:(i32,i32,u32,u32),point:(i32,i32),screen:(u32,u32))->bool{
            let(px,py)=point;
            if px<0||py<0||!self.workarea(screen.0,screen.1).contains(px as u32,py as u32){return false;}
            self.rounded(frame.2,frame.3).coverage(px as i64-frame.0 as i64,py as i64-frame.1 as i64)!=0
        }
        pub const fn px(self, logical: u32) -> u32 { logical.saturating_mul(self.scale) }
        pub const fn bar(self) -> u32 { self.px(BAR) }
        pub const fn title(self) -> u32 { self.px(TITLE) }
        pub const fn workarea(self, width: u32, height: u32) -> Rect {
            let y=if self.bar()<height {self.bar()} else {height};
            Rect { x:0, y, w:width, h:height-y }
        }
        /// Center a logical overlay inside the workarea with a scaled gap.
        /// Empty/tiny screens yield empty clipped rectangles, never overflow.
        pub const fn centered_panel(self,width:u32,height:u32,logical_w:u32,logical_h:u32)->Rect{
            let area=self.workarea(width,height);let gap=self.px(8);
            let x=if gap<area.w {gap}else{area.w};
            let y=if gap<area.h {gap}else{area.h};
            let aw=area.w.saturating_sub(gap.saturating_mul(2));
            let ah=area.h.saturating_sub(gap.saturating_mul(2));
            let wanted_w=self.px(logical_w);let wanted_h=self.px(logical_h);
            let w=if wanted_w<aw {wanted_w}else{aw};
            let h=if wanted_h<ah {wanted_h}else{ah};
            Rect{x:area.x+x+(aw-w)/2,y:area.y+y+(ah-h)/2,w,h}
        }
        /// Content and pointer mapping share the same bounded inset. Tiny
        /// frames have an empty body rather than wrapped/overlapping content.
        pub const fn body(self, width: u32, height: u32) -> Rect {
            let side=self.px(RESIZE);
            let x=if side<width {side} else {width};
            let top=self.title().saturating_add(side);
            let y=if top<height {top} else {height};
            Rect { x, y, w:width.saturating_sub(side.saturating_mul(2)),
                h:height.saturating_sub(top.saturating_add(side)) }
        }
        /// Close/minimize/zoom have equal disjoint targets. Callers must
        /// intersect with the actual frame before accepting a hit.
        pub const fn control(self, index: u32) -> Option<Rect> {
            if index>=3 {return None;}
            Some(Rect { x:self.px(1+index*TARGET), y:self.px(1),
                w:self.px(TARGET), h:self.px(TARGET) })
        }
    }
}

/// Visual family classification only. Callers must independently attest live
/// ownership before accepting any surface operation or routing input.
const fn native_family(surface:u64)->Option<crate::app_catalog::App>{
    let mut i=0;
    while i<crate::app_catalog::COUNT {
        let row=&crate::app_catalog::INCARNATIONS[i];
        if row.frame()==surface||row.surface==surface{return Some(row.app);}
        let mut child=0;
        while child<row.children.len(){
            if row.children[child]==surface{return Some(row.app);}
            child+=1;
        }
        i+=1;
    }
    None
}
pub const fn native_label(surface:u64)->Option<&'static str>{
    if surface==200{return Some("Linen");}
    use crate::app_catalog::App;
    match native_family(surface) {
        Some(App::Quil)=>Some("Quil"),
        Some(App::Spindle)=>Some("Spindle"),
        Some(App::Browser)=>Some("Browser"),
        Some(App::Program)=>Some("Program"),
        None=>None,
    }
}
pub const fn native_indicator(surface:u64)->Option<u32>{
    if surface==200{return Some(0x00A6E3A1);}
    use crate::app_catalog::App;
    match native_family(surface) {
        Some(App::Quil)=>Some(0x00CBA6F7),
        Some(App::Spindle)=>Some(0x0089B4FA),
        Some(App::Browser)=>Some(0x005ED9C3),
        Some(App::Program)=>Some(0x00F9E2AF),
        None=>None,
    }
}
pub const NATIVE_CANVAS:u32=0x000C1420;
/// A family ID or hardware key does not establish this exact BOOT ownership.
pub const fn native_canvas(owner:u32,surface:u64)->Option<u32>{
    // Every Quil and Spindle content surface; Browser paints its own canvas.
    if crate::app_catalog::native_pixel_bit(owner,surface)!=0&&!crate::app_catalog::is_browser(owner){Some(NATIVE_CANVAS)}else{None}
}

// IEC sRGB transfer function sampled at every 8-bit channel, rounded to
// millionths of linear light. A fixed table keeps guest drawing integer-only.
const LINEAR_SRGB: [u32; 256] = [
    0, 304, 607, 911, 1214, 1518, 1821, 2125,
    2428, 2732, 3035, 3347, 3677, 4025, 4391, 4777,
    5182, 5605, 6049, 6512, 6995, 7499, 8023, 8568,
    9134, 9721, 10330, 10960, 11612, 12286, 12983, 13702,
    14444, 15209, 15996, 16807, 17642, 18500, 19382, 20289,
    21219, 22174, 23153, 24158, 25187, 26241, 27321, 28426,
    29557, 30713, 31896, 33105, 34340, 35601, 36889, 38204,
    39546, 40915, 42311, 43735, 45186, 46665, 48172, 49707,
    51269, 52861, 54480, 56128, 57805, 59511, 61246, 63010,
    64803, 66626, 68478, 70360, 72272, 74214, 76185, 78187,
    80220, 82283, 84376, 86500, 88656, 90842, 93059, 95307,
    97587, 99899, 102242, 104616, 107023, 109462, 111932, 114435,
    116971, 119538, 122139, 124772, 127438, 130136, 132868, 135633,
    138432, 141263, 144128, 147027, 149960, 152926, 155926, 158961,
    162029, 165132, 168269, 171441, 174647, 177888, 181164, 184475,
    187821, 191202, 194618, 198069, 201556, 205079, 208637, 212231,
    215861, 219526, 223228, 226966, 230740, 234551, 238398, 242281,
    246201, 250158, 254152, 258183, 262251, 266356, 270498, 274677,
    278894, 283149, 287441, 291771, 296138, 300544, 304987, 309469,
    313989, 318547, 323143, 327778, 332452, 337164, 341914, 346704,
    351533, 356400, 361307, 366253, 371238, 376262, 381326, 386429,
    391572, 396755, 401978, 407240, 412543, 417885, 423268, 428690,
    434154, 439657, 445201, 450786, 456411, 462077, 467784, 473531,
    479320, 485150, 491021, 496933, 502886, 508881, 514918, 520996,
    527115, 533276, 539479, 545724, 552011, 558340, 564712, 571125,
    577580, 584078, 590619, 597202, 603827, 610496, 617207, 623960,
    630757, 637597, 644480, 651406, 658375, 665387, 672443, 679542,
    686685, 693872, 701102, 708376, 715694, 723055, 730461, 737910,
    745404, 752942, 760525, 768151, 775822, 783538, 791298, 799103,
    806952, 814847, 822786, 830770, 838799, 846873, 854993, 863157,
    871367, 879622, 887923, 896269, 904661, 913099, 921582, 930111,
    938686, 947307, 955973, 964686, 973445, 982251, 991102, 1000000,
];

/// WCAG relative luminance in millionths; XRGB's unused high byte is ignored.
pub const fn relative_luminance(color: u32) -> u32 {
    (((LINEAR_SRGB[((color >> 16) & 255) as usize] as u64) * 2126
        + (LINEAR_SRGB[((color >> 8) & 255) as usize] as u64) * 7152
        + (LINEAR_SRGB[(color & 255) as usize] as u64) * 722) / 10_000) as u32
}

/// Select the stronger black/white contrast against the actual composed pixel.
/// Comparing ratios by cross multiplication avoids division and guest floats.
/// The better of black and white has contrast at least sqrt(21), above 4.5:1;
/// table rounding is under two millionths of relative luminance.
pub const fn contrast_ink(background: u32) -> u32 {
    let light = relative_luminance(background) as u64 + 50_000;
    if light * light >= 1_050_000 * 50_000 { 0x000000 } else { 0xFFFFFF }
}
