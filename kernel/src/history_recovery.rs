//! Read-only transition: exact capability slot and immutable BOOT metadata.
pub fn query(pd:&crate::capability::ProtectionDomain,slot:u64,kind:u64)->[u64;5]{
 let Some(slot)=crate::capability_slot::decode_capability_slot(slot)else{return [1,0,0,0,0]};
 match pd.find_capability(slot).map(|c|c.data){Some(crate::capability::CapabilityData::HistoryRecovery(g))=>g.query(pd.id,kind),_=>[1,0,0,0,0]}
}
