//! Immutable BOOT metadata. Caller role is an additional filter, not a grant.
use sex_pdx::network_config::Config;
#[derive(Clone,Copy)]
pub struct Info { owner:u32, config:Config }
impl Info {
    pub fn new(owner:u32,config:Config)->Option<Self>{
        if owner==0||!config.valid(){None}else{Some(Self{owner,config})}
    }
    pub fn query(self,caller:u32)->Option<[u64;4]>{
        if caller!=self.owner||caller==0{None}else{Some(self.config.to_words())}
    }
}
