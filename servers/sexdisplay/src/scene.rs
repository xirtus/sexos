// SPDX-License-Identifier: GPL-3.0-only
//! Display-owned canonical and sealed-candidate inputs for one render invocation.
//!
//! This is data isolation, not capability authority, an epoch/replay engine,
//! framebuffer publication, or permission to retain data past lifecycle ACK.
//! Main must release this snapshot before acknowledging resource cleanup.
use crate::{bitmap_stores::{PublishedStores,Stores},surface_groups::FrameGroups,
    surface_lifecycle::{Surface,MAX_SURFACES,SURFACE_EMPTY},
    RenderTokensV1,DEFAULT_RENDER_TOKENS,MAX_FB_W,MAX_FB_H};
use silkbar_model::{SilkBar,DEFAULT_SILK_BAR};
const BROWSERS:usize=sex_pdx::app_catalog::count(sex_pdx::app_catalog::App::Browser);

/// Required candidate text pixels, descriptive only; not publication authority.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum TextCoverage{NoPixels,Body(crate::damage::Rect)}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct TextProjection{
    pub(super) context:crate::text_transaction::Context,
    pub(super) coverage:TextCoverage,
}

/// Deliberately neither Copy nor Clone: large owned payload stays in its fixed
/// Display backing. No framebuffer, producer pointer, receipt, or Stage lives here.
pub struct SceneSnapshot {
    pub(super) surfaces:[Surface;MAX_SURFACES],
    // Descriptive captured multipart contexts, never reply authority.
    pub(super) text_projections:[Option<TextProjection>;MAX_SURFACES],
    pub(super) groups:FrameGroups,
    pub(super) bitmaps:PublishedStores<BROWSERS>,
    pub(super) bar:SilkBar,
    pub(super) tokens:RenderTokensV1,
    pub(super) focus:u64,
    pub(super) frame:u64,
    pub(super) blur_rows:[u32;crate::BAR_BG_H],
    pub(super) width:usize,
    pub(super) height:usize,
    valid:bool,
}
impl SceneSnapshot {
    /// Initialize existing static storage directly; never return a captured
    /// aggregate or construct a second large image on the service stack.
    pub const EMPTY:Self=Self{surfaces:[SURFACE_EMPTY;MAX_SURFACES],text_projections:[None;MAX_SURFACES],groups:FrameGroups::new(),
        bitmaps:PublishedStores::EMPTY,bar:DEFAULT_SILK_BAR,tokens:DEFAULT_RENDER_TOKENS,
        focus:0,frame:0,blur_rows:[0;crate::BAR_BG_H],width:0,height:0,valid:false};

    /// Inputs are already committed/admitted Display data; sealed unreleased
    /// text is projected into this owned candidate without changing canonical data. Copying does not
    /// establish liveness, grant a route, or perform another protocol admission.
    /// All visible fields become eligible only after the final valid write.
    pub fn capture(&mut self,surfaces:&[Surface;MAX_SURFACES],groups:&FrameGroups,
        bitmaps:&Stores<BROWSERS>,texts:&[crate::text_transaction::Stage;MAX_SURFACES],bar:&SilkBar,tokens:RenderTokensV1,
        focus:u64,frame:u64,width:usize,height:usize)->bool{
        self.scrub();
        if width==0||height==0||width>MAX_FB_W||height>MAX_FB_H
            ||width.checked_mul(height).is_none(){return false;}
        self.blur_rows=crate::scene_bar_blur_rows(height);
        if !bitmaps.snapshot_into(&mut self.bitmaps){self.scrub();return false;}
        // Slice copy writes the existing owned array directly. It does not
        // materialize/return a [Surface;16] temporary or borrow producer storage.
        self.surfaces.copy_from_slice(surfaces);
        for (index,stage) in texts.iter().enumerate(){
            if let Some(pending)=stage.pending(){
                let context=pending.context();
                let surface=&mut self.surfaces[index];
                if !surface.active||surface.owner_pd!=context.owner()||surface.surface_id!=context.surface()
                    ||!crate::apply_pending_text(surface,&pending){self.scrub();return false;}
                let coverage=crate::text_projection_requirement(&surfaces[index],surface,width,height);
                self.text_projections[index]=Some(TextProjection{context,coverage});
            }
        }
        self.groups=*groups;self.bar=*bar;self.tokens=tokens;
        self.focus=focus;self.frame=frame;self.width=width;self.height=height;
        self.valid=true;true
    }

    pub fn is_valid(&self)->bool{self.valid}

    /// Remove retained text, selection, scalar layout and bitmap pixels before
    /// releasing a scene. Volatile value writes cover every Surface field, not
    /// just text_len: hidden/tail bytes and cursor/fill geometry are erased too.
    /// Metadata resets are descriptive only; source lifecycle tombstones remain
    /// owned by the separate live admission state.
    pub fn scrub(&mut self){
        unsafe{core::ptr::write_volatile(&mut self.valid,false);}
        for surface in &mut self.surfaces{
            unsafe{core::ptr::write_volatile(surface,SURFACE_EMPTY);}
        }
        for context in &mut self.text_projections{unsafe{core::ptr::write_volatile(context,None);}}
        self.bitmaps.scrub();
        self.groups=FrameGroups::new();self.bar=DEFAULT_SILK_BAR;
        self.tokens=DEFAULT_RENDER_TOKENS;self.focus=0;self.frame=0;self.width=0;self.height=0;
        for row in &mut self.blur_rows{unsafe{core::ptr::write_volatile(row,0);}}
    }
}
