// SPDX-License-Identifier: GPL-3.0-only
//! Bounded output obligations against the latest committed Display state.
//! These joins record neither an input/causal stream nor capability authority.
//! Main is single-task and does not admit new work during render/publication;
//! only under that condition may successful publication clear the whole batch.
use crate::damage::{Damage,Rect};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum Content{None,Settle,Cursor,Damage(Rect),Area,Full}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Plan{pub content:Content,pub strip:bool}
impl Plan{
    pub const fn full()->Self{Self{content:Content::Full,strip:false}}
    pub const fn is_empty(&self)->bool{matches!(self.content,Content::None)&&!self.strip}
    pub const fn is_full(&self)->bool{matches!(self.content,Content::Full)}
    pub const fn clock_dirty(&self)->bool{self.is_full()||self.strip}
    pub const fn cursor_only(&self)->bool{matches!(self.content,Content::Cursor)&&!self.strip}
    /// Republish an unchanged baseline for captured data admission, without
    /// claiming that a cursor, clock or visible body pixel was redrawn.
    pub const fn settle_only(&self)->bool{matches!(self.content,Content::Settle)&&!self.strip}
    /// Coverage only, not publication or authority. Callers must first clip
    /// required damage to the captured frame geometry. Endpoint comparisons
    /// avoid addition/subtraction overflow, even for adversarial coordinates.
    pub const fn covers_body(&self,required:Rect)->bool{
        if required.x>=required.right||required.y<51||required.y>=required.bottom{return false;}
        match self.content{
            Content::Full|Content::Area=>true,
            Content::Damage(actual)=>actual.x<actual.right&&actual.y>=51&&actual.y<actual.bottom
                &&actual.x<=required.x&&actual.y<=required.y
                &&actual.right>=required.right&&actual.bottom>=required.bottom,
            Content::None|Content::Settle|Content::Cursor=>false,
        }
    }
    pub const fn label(&self)->&'static str{
        if self.is_full(){"full"}else if self.strip{"strip"}else{match self.content{
            Content::Area=>"area",Content::Damage(_)=>"text",Content::Cursor=>"cursor",Content::Settle=>"settle",Content::None=>"none",Content::Full=>"full"}}
    }
    pub const fn with_atlas(self,enabled:bool)->Self{
        if enabled&&matches!(self.content,Content::Settle|Content::Damage(_)){Self{content:Content::Area,strip:self.strip}}else{self}
    }
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Dirty{content:Content,strip:bool,width:usize,height:usize}
impl Dirty{
    pub const fn new(width:usize,height:usize)->Self{Self{content:Content::None,strip:false,width,height}}
    pub fn request_full(&mut self){self.content=Content::Full;self.strip=false;}
    pub fn request_area(&mut self){if !matches!(self.content,Content::Full){self.content=Content::Area;}}
    pub fn request_settle(&mut self){if matches!(self.content,Content::None){self.content=Content::Settle;}}
    pub fn request_cursor(&mut self){if matches!(self.content,Content::None|Content::Settle){self.content=Content::Cursor;}}
    pub fn request_strip(&mut self){if !matches!(self.content,Content::Full){self.strip=true;}}
    fn geometry(&mut self,width:usize,height:usize){
        if self.width!=width||self.height!=height{
            if !matches!(self.content,Content::None)||self.strip{self.request_full();}
            self.width=width;self.height=height;
        }
    }
    pub fn request_damage(&mut self,rect:Rect,width:usize,height:usize){
        // Ordered endpoints allow subtraction without wrap. The actual shared
        // Damage clips saturating extents to the framebuffer and below row50.
        if rect.x>=rect.right||rect.y>=rect.bottom{return;}
        let mut damage=Damage::new();
        damage.add((rect.x,rect.y,rect.right-rect.x,rect.bottom-rect.y),width,height);
        let Some(rect)=damage.rect()else{return;};
        self.geometry(width,height);
        match self.content{
            Content::Full|Content::Area=>{},
            Content::Damage(old)=>{
                damage.add((old.x,old.y,old.right-old.x,old.bottom-old.y),width,height);
                self.content=Content::Damage(damage.rect().expect("NONEMPTY_CLIPPED_DAMAGE"));
            }
            Content::None|Content::Settle|Content::Cursor=>self.content=Content::Damage(rect),
        }
    }
    /// A changed geometry invalidates a nonempty partial obligation. Persist
    /// that promotion through failed attempts; this does not resize the BOOT
    /// backing. The actual output store still rejects a geometry mismatch.
    pub fn plan(&mut self,width:usize,height:usize)->Plan{
        self.geometry(width,height);Plan{content:self.content,strip:self.strip}
    }
    /// False preserves both output obligations and their remembered geometry.
    /// No intake may interleave a successful plan's render and this completion.
    pub fn complete(&mut self,published:bool){
        if published{self.content=Content::None;self.strip=false;}
    }
}
