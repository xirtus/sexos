#![allow(dead_code)]
#[path = "../kernel/src/paging_state.rs"] mod state;
use state::*;
fn grants() -> (PagerMappingGrant,FramePoolGrant,FaultView) {
    (PagerMappingGrant {range_id:1,pool_id:1,target_pd:15,base_va:0x4000_0000,length:PAGE*8,rights:6,quota:4},
    FramePoolGrant {pool_id:1,base_va:0x5000_0000,length:PAGE*8},
    FaultView {task_id:0x8015,token:1,target_pd:15,pager_pd:14,page:0x4000_0000,error:6})
}
#[test]
fn exact_pending_fault_with_boot_authorities_produces_bounded_binding() {
    let (map,pool,fault)=grants();
    let binding=validate(map,pool,14,fault,7,6).unwrap();
    assert_eq!(binding.page_index,0); assert_eq!(binding.frame_index,7);
}
#[test]
fn wrong_identity_or_invalid_fault_type_cannot_map() {
    let (map,pool,fault)=grants();
    for bad in [FaultView{token:0,..fault},FaultView{target_pd:16,..fault},FaultView{pager_pd:13,..fault},
        FaultView{page:fault.page+1,..fault},FaultView{page:fault.page-PAGE,..fault},
        FaultView{page:fault.page+map.length,..fault}] { assert!(validate(map,pool,14,bad,0,6).is_err()); }
    for error in [0,2,5,7,12,20,36,u64::MAX] {
        assert!(validate(map,pool,14,FaultView{error,..fault},0,6).is_err());
    }
}
#[test]
fn rights_and_pool_indices_keep_full_width_and_read_fault_cannot_upgrade_grant() {
    let (map,pool,fault)=grants();
    for rights in [0,4,7,0x106,0x1_0000_0006,u64::MAX] {assert!(validate(map,pool,14,fault,0,rights).is_err());}
    for index in [8,0x1_0000_0000,u64::MAX] {assert!(validate(map,pool,14,fault,index,6).is_err());}
    assert!(validate(PagerMappingGrant{rights:READ,..map},pool,14,FaultView{error:4,..fault},0,6).is_err());
    assert!(validate(map,pool,14,fault,0,READ as u64).is_err());
}
#[test]
fn frame_and_virtual_page_identity_cannot_be_reissued() {
    let (map,pool,fault)=grants(); let mut ledger=MappingLedger::new();
    let first=validate(map,pool,14,fault,0,6).unwrap(); ledger.install(first,map.quota).unwrap();
    for bad in [Binding{token:2,..first},Binding{token:2,frame_index:1,..first},Binding{token:1,page_index:1,frame_index:1,..first}] {
        assert_eq!(ledger.install(bad,map.quota),Err(DENIED));
    }
    assert_eq!(ledger.entries.iter().flatten().count(),1);
}
#[test]
fn quota_failure_preserves_free_frame_and_original_bindings() {
    let (map,pool,fault)=grants(); let mut ledger=MappingLedger::new();
    for i in 0..4 { let b=validate(map,pool,14,FaultView{token:i+1,page:fault.page+i*PAGE,..fault},i,6).unwrap();ledger.install(b,map.quota).unwrap(); }
    let fifth=validate(map,pool,14,FaultView{token:5,page:fault.page+4*PAGE,..fault},4,6).unwrap();
    assert_eq!(ledger.install(fifth,map.quota),Err(EXHAUSTED)); assert!(ledger.entries[4].is_none());
}
#[test]
fn finalization_requires_exact_committed_tuple() {
    let (map,pool,fault)=grants();let mut ledger=MappingLedger::new();let binding=validate(map,pool,14,fault,0,6).unwrap();
    ledger.install(binding,8).unwrap();assert_eq!(ledger.finalize(Binding{rights:READ,..binding}),Err(DENIED));
    assert!(!ledger.token(1).unwrap().finalized);ledger.finalize(binding).unwrap();ledger.finalize(binding).unwrap();
    assert!(ledger.token(1).unwrap().finalized);
}
#[test]
fn malformed_boot_metadata_never_grants_frame_or_range_authority() {
    let (map,pool,fault)=grants();
    for bad in [PagerMappingGrant{pool_id:2,..map},PagerMappingGrant{base_va:u64::MAX-4095,..map},
        PagerMappingGrant{length:0,..map},PagerMappingGrant{length:9*PAGE,..map},PagerMappingGrant{quota:0,..map}] {
        assert!(validate(bad,pool,14,fault,0,6).is_err());
    }
}

#[test]
fn independent_client_pages_and_quotas_share_exclusive_frames() {
    let (map,pool,fault)=grants();let mut ledger=MappingLedger::new();
    for i in 0..4 {
        let binding=validate(map,pool,14,FaultView{token:i+1,page:fault.page+i*PAGE,..fault},i,6).unwrap();
        ledger.install(binding,4).unwrap();
    }
    let second=PagerMappingGrant{range_id:2,target_pd:10,base_va:0x6000_0000,..map};
    for i in 0..4 {
        let f=FaultView{token:i+5,target_pd:10,page:second.base_va+i*PAGE,..fault};
        let collision=validate(second,pool,14,f,i,6).unwrap();
        assert_eq!(ledger.install(collision,4),Err(DENIED));
        let binding=validate(second,pool,14,f,i+4,6).unwrap();ledger.install(binding,4).unwrap();
    }
    assert_eq!(ledger.entries.iter().flatten().count(),8);
}

#[test]
fn retired_generation_cannot_be_installed_into_a_different_frame() {
    let (map,pool,fault)=grants();let mut ledger=MappingLedger::new();
    let old=validate(map,pool,14,fault,0,6).unwrap();ledger.install(old,4).unwrap();
    assert_eq!(ledger.retire(old),Err(DENIED)); // must complete the fault first
    ledger.finalize(old).unwrap();ledger.retire(old).unwrap();
    assert_eq!(ledger.retired(0),Some(old));
    assert_eq!(ledger.install(old,4),Err(DENIED));
    assert_eq!(ledger.install(Binding{frame_index:7,..old},4),Err(DENIED));
    assert!(ledger.entries.iter().all(Option::is_none));
}

#[test]
fn tombstone_survives_reassignment_and_only_exact_finalized_owner_can_retire() {
    let (map,pool,fault)=grants();let mut ledger=MappingLedger::new();
    let first=validate(map,pool,14,fault,0,6).unwrap();
    ledger.install(first,4).unwrap();ledger.finalize(first).unwrap();ledger.retire(first).unwrap();
    let next=Binding{token:2,range_id:2,target_pd:10,..first};ledger.install(next,4).unwrap();
    assert_eq!(ledger.retired(0),Some(first));
    assert_eq!(ledger.retire(first),Err(DENIED));
    assert_eq!(ledger.retire(next),Err(DENIED));
    ledger.finalize(next).unwrap();
    assert_eq!(ledger.retire(Binding{rights:2,..next}),Err(DENIED));
    ledger.retire(next).unwrap();assert_eq!(ledger.retired(0),Some(next));
}

#[test]
fn retirement_floor_is_per_range_and_does_not_remove_live_older_binding() {
    let (map,pool,fault)=grants();let mut ledger=MappingLedger::new();
    let older=validate(map,pool,14,fault,0,6).unwrap();
    let newer=Binding{token:100,page_index:1,frame_index:1,..older};
    for binding in [older,newer] {ledger.install(binding,4).unwrap();ledger.finalize(binding).unwrap();}
    ledger.retire(newer).unwrap();assert_eq!(ledger.token(1).unwrap().binding,older);
    assert!(ledger.is_retired(1,99));assert!(!ledger.is_retired(2,99));
    let second=Binding{token:99,range_id:2,target_pd:10,frame_index:2,..older};
    ledger.install(second,4).unwrap();
    assert_eq!(ledger.install(Binding{token:99,frame_index:3,page_index:3,..older},4),Err(DENIED));
}

#[test]
fn exit_cleanup_binds_exact_task_and_includes_unfinished_publication() {
    let (map,pool,fault)=grants();let mut ledger=MappingLedger::new();
    let first=validate(map,pool,14,fault,0,6).unwrap();
    let second=Binding{token:2,page_index:1,frame_index:1,..first};
    let foreign=Binding{token:3,target_pd:10,task_id:0x8010,range_id:2,frame_index:2,..first};
    for b in [first,second,foreign] {ledger.install(b,4).unwrap();}
    ledger.finalize(first).unwrap();
    assert_eq!(ledger.cleanup_plan(1,15,14,0xdead),Err(DENIED));
    assert_eq!(ledger.cleanup_plan(1,15,13,first.task_id),Err(DENIED));
    assert_eq!(ledger.cleanup_plan(1,10,14,first.task_id),Err(DENIED));
    let plan=ledger.cleanup_plan(1,15,14,first.task_id).unwrap();
    assert_eq!(plan.iter().flatten().count(),2);assert_eq!(plan[1],Some(second));
    // This helper is invoked only after the production all-leaf flush boundary.
    let closed=ledger.close_exited_range(1,15,14,first.task_id).unwrap();
    assert_eq!(closed.frames,3);assert_eq!(closed.task_id,first.task_id);
    assert_eq!(ledger.token(3).unwrap().binding,foreign);
    assert!(ledger.token(1).is_none() && ledger.token(2).is_none());
    assert_eq!(ledger.retired(1),Some(second));
    assert_eq!(ledger.install(Binding{token:100,frame_index:3,..first},4),Err(DENIED));
}

#[test]
fn empty_terminal_range_closes_and_cached_cleanup_survives_other_owner_reuse() {
    let (map,pool,fault)=grants();let mut ledger=MappingLedger::new();
    let first=validate(map,pool,14,fault,0,6).unwrap();ledger.install(first,4).unwrap();
    let done=ledger.close_exited_range(1,15,14,first.task_id).unwrap();
    let foreign=Binding{token:2,target_pd:10,task_id:0x8010,range_id:2,..first};
    ledger.install(foreign,4).unwrap();ledger.finalize(foreign).unwrap();
    assert_eq!(ledger.cleanup_record(1),Some(done));
    assert_eq!(ledger.entries[0].unwrap().binding,foreign);
    ledger.retire(foreign).unwrap();
    let empty=ledger.close_exited_range(2,10,14,foreign.task_id).unwrap();
    assert_eq!(empty.frames,0);
    assert_eq!(ledger.install(Binding{token:3,..foreign},4),Err(DENIED));
}
