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
        pub const fn new()->Self {Self{sequence:0,pending:None}}
        pub fn begin(&mut self,step:i64)->bool {
            if self.pending.is_some(){return false;}
            self.sequence=(self.sequence%SEQUENCE_MASK)+1;
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
    pub struct ScalePublisher { pending:Option<(u32,u64,bool)> }
    impl ScalePublisher {
        pub const fn new()->Self {Self{pending:None}}
        pub fn admit(&mut self,target:u32,sequence:u64,scale:u32)->bool {
            if self.pending.is_some(){return false;}
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
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Rect { pub x: u32, pub y: u32, pub w: u32, pub h: u32 }
    impl Rect {
        pub const fn contains(self, x: u32, y: u32) -> bool {
            x >= self.x && y >= self.y && x-self.x < self.w && y-self.y < self.h
        }
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Metrics { scale: u32 }
    impl Metrics {
        pub const fn new(scale: u32) -> Self { Self { scale: clamp_scale(scale) } }
        pub const fn scale(self) -> u32 { self.scale }
        pub const fn px(self, logical: u32) -> u32 { logical.saturating_mul(self.scale) }
        pub const fn bar(self) -> u32 { self.px(BAR) }
        pub const fn title(self) -> u32 { self.px(TITLE) }
        pub const fn workarea(self, width: u32, height: u32) -> Rect {
            let y=if self.bar()<height {self.bar()} else {height};
            Rect { x:0, y, w:width, h:height-y }
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
