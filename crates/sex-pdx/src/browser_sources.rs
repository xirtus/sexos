// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Descriptive Net-local source-slot plans for catalogued Browser incarnations.
//! These values neither grant capabilities nor identify a task. BOOT must check
//! actual table vacancy, construct/read back exact typed grants with registered
//! holder/consumer tasks, then seal; services must check their actual held Info.
//! Provider PD13 is an identity, never a grant count or an inferred task ID.
use crate::app_catalog::{self, App};

pub const BROWSER_COUNT:usize=app_catalog::count(App::Browser);
// The current one-indexed kernel capability table bound. This validates only
// the descriptive plan's full-width IDs, not vacancy or authority at runtime.
const MAX_SLOT:u64=1024;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Plan {
    target:u32,surface:u64,lifecycle_slot:usize,
    entropy_slot:u64,clock_slot:u64,trust_slot:u64,
}
impl Plan {
    pub const fn target(self)->u32{self.target}
    pub const fn surface(self)->u64{self.surface}
    pub const fn lifecycle_slot(self)->usize{self.lifecycle_slot}
    pub const fn entropy_slot(self)->u64{self.entropy_slot}
    pub const fn clock_slot(self)->u64{self.clock_slot}
    pub const fn trust_slot(self)->u64{self.trust_slot}
}

// No public constructor or arbitrary ordinal-to-slot selector. Unknown catalog
// ordinals fail compilation rather than wrapping, truncating or aliasing slots.
const fn slots(ordinal:u8)->[u64;3]{
    match ordinal {
        0=>[60,61,62],
        // Reserved successors use disjoint holder-local source slots.
        // BOOT inserts and verifies the actual capabilities before sealing.
        1=>[70,71,72],
        2=>[73,74,75],
        _=>panic!("unsupported Browser source-plan ordinal"),
    }
}
const fn build()->[Plan;BROWSER_COUNT]{
    let empty=Plan{target:0,surface:0,lifecycle_slot:0,entropy_slot:0,clock_slot:0,trust_slot:0};
    let mut out=[empty;BROWSER_COUNT];let mut i=0;let mut n=0;
    assert!(BROWSER_COUNT>0,"missing original Browser source-plan row");
    while i<app_catalog::INCARNATIONS.len(){
        let row=&app_catalog::INCARNATIONS[i];
        if matches!(row.app,App::Browser){
            assert!(row.ordinal as usize==n,"Browser source-plan ordinal order");
            assert!(row.target!=0&&row.surface!=0,"zero Browser source-plan association");
            assert!(row.lifecycle_slot>0&&row.lifecycle_slot as u64<=MAX_SLOT,"Browser lifecycle slot range");
            let triple=slots(row.ordinal);let mut a=0;
            while a<3 {
                let slot=triple[a];
                assert!(slot>0&&slot<=MAX_SLOT,"Browser source slot range");
                // Net's non-catalog fixed inventory (root/self/signal/NIC/DMA).
                // Actual BOOT preflight must still detect all table occupants.
                assert!(slot!=1&&slot!=2&&slot!=3&&slot!=19&&slot!=32,"Net fixed slot collision");
                let mut b=0;while b<a {assert!(slot!=triple[b],"source triple aliases");b+=1;}
                let mut j=0;while j<app_catalog::INCARNATIONS.len(){
                    let other=&app_catalog::INCARNATIONS[j];
                    assert!(slot!=other.lifecycle_slot as u64,"source/lifecycle slot collision");
                    if i!=j {
                        assert!(row.target!=other.target,"Browser target aliases another catalog row");
                        assert!(row.surface!=other.surface,"Browser surface aliases another catalog row");
                    }
                    j+=1;
                }
                let mut p=0;while p<n {
                    assert!(slot!=out[p].entropy_slot&&slot!=out[p].clock_slot&&slot!=out[p].trust_slot,"source plans alias");
                    p+=1;
                }
                a+=1;
            }
            out[n]=Plan{target:row.target,surface:row.surface,lifecycle_slot:row.lifecycle_slot,
                entropy_slot:triple[0],clock_slot:triple[1],trust_slot:triple[2]};n+=1;
        }
        i+=1;
    }
    assert!(n==BROWSER_COUNT,"Browser source-plan count");out
}
/// Only actual immutable catalog rows produce plans. Future candidates in slots
/// do not increase this array's length without an explicit catalog change.
pub const PLANS:[Plan;BROWSER_COUNT]=build();
pub const fn by_index(index:usize)->Option<&'static Plan>{
    if index<BROWSER_COUNT {Some(&PLANS[index])}else{None}
}
pub const fn by_target(target:u32)->Option<&'static Plan>{
    let mut i=0;while i<BROWSER_COUNT {if PLANS[i].target==target{return Some(&PLANS[i]);}i+=1;}None
}
