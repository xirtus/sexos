// SPDX-License-Identifier: GPL-3.0-only
#[path="../crates/sex-pdx/src/app_lifecycle.rs"]mod wire;
#[test]fn source_flag_is_descriptive_and_strict(){
 for state in 0..=wire::ACTIVATION_REBIRTHING {
  let plain=wire::ActivationInfo::decode(42,1073741830,359,state).unwrap();
  let pinned=wire::ActivationInfo::decode(42,1073741830,359,state|wire::ACTIVATION_SOURCE_PINNED).unwrap();
  assert!(!plain.source_pinned);assert!(pinned.source_pinned);assert_eq!(plain.state,pinned.state);
 }
 for flags in [6,255,512,u64::MAX]{assert!(wire::ActivationInfo::decode(42,42,359,flags).is_err());}
 for (pd,task) in [(0,42),(42,0),(1u64<<32,42),(42,1u64<<32)]{assert!(wire::ActivationInfo::decode(pd,task,359,0).is_err());}
}
