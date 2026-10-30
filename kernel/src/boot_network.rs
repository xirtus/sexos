//! Seeded once before BOOT sealing. No runtime setters or capability creation.
use super::boot_network_state::Info;
static INFO:spin::Once<Info>=spin::Once::new();
pub fn init(owner:u32){
    assert!(INFO.get().is_none(),"BOOT_NETWORK_ALREADY_SEEDED");
    if owner!=0{
        let info=Info::new(owner,sex_pdx::network_config::Config::CONTROLLED).expect("BOOT_NETWORK_CONFIG");
        INFO.call_once(||info);
        crate::serial_println!("[boot.network.config] owner={} version=1 profile=1 origin=boot_descriptor immutable=1 runtime_mint=0",owner);
    }
}
pub fn query(caller:u32)->Option<[u64;4]>{INFO.get()?.query(caller)}
