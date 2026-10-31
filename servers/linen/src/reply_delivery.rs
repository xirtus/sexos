//! One response retained until the permanent shell peer's mailbox accepts it.
//! The service does not consume its next request while this response is pending.
#[derive(Default)]
pub struct ReplyDelivery { pending: Option<u64> }
impl ReplyDelivery {
    pub const fn new()->Self{Self{pending:None}}
    pub fn queue(&mut self,caller:u32,value:u64)->bool {
        if caller!=3 || self.pending.is_some(){return false}
        self.pending=Some(value);true
    }
    pub fn poll(&mut self,mut send:impl FnMut(u32,u64)->u64)->bool {
        match self.pending {
            Some(value) if send(3,value)==0=>{self.pending=None;true},
            Some(_)=>false,None=>true,
        }
    }
    pub fn pending(&self)->bool{self.pending.is_some()}
}

/// Spindle's legacy async bridge has no leases and never blocks the shell peer.
/// A missing/contended/terminal lifecycle attestation denies its queued request.
pub fn admit(caller:u32,opcode:u64,spindle_status:Option<u64>)->bool {
    caller==3 || (crate::app_catalog::is_spindle(caller) && matches!(opcode,0x42|0x46|0x47) && spindle_status==Some(5))
}
pub fn legacy_reply(caller:u32,value:u64,mut send:impl FnMut(u32,u64)->u64)->bool {
    crate::app_catalog::is_spindle(caller) && send(caller,value)==0
}
