//! Actual guard before the historical NIC register/DMA diagnostics.
include!(env!("NIC_BOOT_PROFILE_SOURCE"));
#[test]fn normal_and_nonexplicit_settings_never_activate_controller_diagnostics(){
 for setting in [None,Some(""),Some("0"),Some("true"),Some("01"),Some("1 ")]{
  for(vendor,device)in[(0x8086,0x100e),(0x8086,0x10d3),(0x1af4,0x1000),(0x10ec,0x8168)]{assert!(!legacy_nic_diagnostics(vendor,device,setting));}
 }
}
#[test]fn explicit_legacy_profile_is_limited_to_supported_intel_register_layouts(){
 for(vendor,device)in[(0x8086,0x100e),(0x8086,0x10d3)]{assert!(legacy_nic_diagnostics(vendor,device,Some("1")));}
 for(vendor,device)in[(0x1af4,0x1000),(0x10ec,0x8168),(0x8086,0x100f),(0,0),(0xffff,0xffff)]{assert!(!legacy_nic_diagnostics(vendor,device,Some("1")));}
}
