// SPDX-License-Identifier: GPL-3.0-only
//! Finite Display-owned bitmap backing, one independent Stage per catalogued
//! Browser. Catalog data selects storage only; startup must supply each actual
//! held Binding and Main must still check owned surfaces and live/terminal proof.
use crate::{bitmap_binding::Binding,bitmap_transaction::{Change,Stage}};
use sex_pdx::{app_catalog::{self,App},bitmap_batch::Request};
const CATALOG_COUNT:usize=app_catalog::count(App::Browser);

pub struct Stores<const N:usize>{stages:[Stage;N]}
impl<const N:usize> Stores<N>{
    /// Initialize static backing directly. Installing a Binding below changes
    /// only its existing Stage's metadata, never constructs/copies pixel arrays.
    pub const UNBOUND:Self=Self{stages:[const{Stage::UNBOUND};N]};
    fn catalog_index(binding:Binding)->Option<usize>{
        if N!=CATALOG_COUNT{return None}
        let mut ordinal=0;let mut found=None;
        for row in app_catalog::INCARNATIONS.iter(){
            let browser=matches!(row.app,App::Browser);
            // Reject ambiguous domains or any frame/content/child SID alias.
            if row.target==binding.target()||row.surface==binding.surface()
                ||row.frame()==binding.surface()||row.children.contains(&binding.surface()){
                if !browser||row.target!=binding.target()||row.surface!=binding.surface()
                    ||row.lifecycle_slot!=binding.slot()||binding.task()==0
                    ||binding.coordinator()!=3||found.is_some(){return None}
                found=Some(ordinal);
            }
            if browser{ordinal+=1;}
        }
        if ordinal!=N{return None}found
    }
    pub fn initialize_once(&mut self,binding:Binding)->bool{
        let Some(index)=Self::catalog_index(binding)else{return false};
        self.stages[index].initialize_once(binding)
    }
    // Pixel/packet lookup scans only the finite installed Browser stores (one
    // currently), not every catalog surface or a mutable current-app selector.
    fn installed_index(&self,owner:u32,sid:u64)->Option<usize>{
        if N!=CATALOG_COUNT{return None}
        let mut found=None;
        for(index,stage)in self.stages.iter().enumerate(){
            if stage.matches_surface(owner,sid){
                if found.is_some(){return None}found=Some(index);
            }
        }
        found
    }
    /// Retained association even after retirement, not proof of live admission.
    pub fn binding_for(&self,owner:u32,sid:u64)->Option<Binding>{
        self.stages[self.installed_index(owner,sid)?].binding()
    }
    pub fn matches_surface(&self,owner:u32,sid:u64)->bool{
        self.binding_for(owner,sid).is_some()
    }
    pub fn receive(&mut self,owner:u32,task:u32,sid:u64,request:Request)->Option<Change>{
        let index=self.installed_index(owner,sid)?;
        self.stages[index].receive(owner,task,sid,request)
    }
    pub fn sequence(&self,owner:u32,sid:u64)->Option<u32>{
        Some(self.stages[self.installed_index(owner,sid)?].sequence())
    }
    /// Local bitmap coordinates. Main independently clips to the actual winning
    /// surface/body/framebuffer before sampling; Stage enforces pixel geometry.
    pub fn sample(&self,owner:u32,sid:u64,x:i64,y:i64)->Option<u32>{
        self.stages[self.installed_index(owner,sid)?].sample(owner,sid,x,y)
    }
    pub fn reset_surface(&mut self,owner:u32,sid:u64){
        if let Some(index)=self.installed_index(owner,sid){self.stages[index].reset_surface(owner,sid);}
    }
    /// Main must already hold exact terminal lifecycle proof. A wrong task
    /// cannot erase a peer; tombstones remain installed and cannot be rebound.
    pub fn retire(&mut self,owner:u32,task:u32){
        for stage in &mut self.stages{stage.retire(owner,task);}
    }
    /// At most N attempts per turn, one per Stage. Full at one owner never
    /// prevents another Stage's attempt or changes any receipt's destination.
    pub fn poll_reply(&mut self,mut send:impl FnMut(u32,u64)->u64){
        for stage in &mut self.stages{stage.poll_reply(&mut send);}
    }
    /// Installed metadata count includes permanently retired tombstones.
    pub fn initialized_count(&self)->usize{
        self.stages.iter().filter(|stage|stage.binding().is_some()).count()
    }
    #[cfg(test)]
    pub fn scrubbed(&self,owner:u32,sid:u64)->Option<bool>{
        Some(self.stages[self.installed_index(owner,sid)?].scrubbed())
    }
    #[cfg(test)]
    pub fn pixels_scrubbed(&self,owner:u32,sid:u64)->Option<bool>{
        Some(self.stages[self.installed_index(owner,sid)?].pixels_scrubbed())
    }
}
