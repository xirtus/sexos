#[path="../servers/sext/src/workspace.rs"] mod workspace;
#[repr(align(4096))] struct Bytes([u8;8192]);
#[test] fn scrubs_entire_pool_and_reuses_actual_storage_without_overrun(){
    let mut bytes=Box::new(Bytes([0xa5;8192]));
    let mut w=unsafe{workspace::Workspace::admit(bytes.0.as_mut_ptr() as u64,8192)};
    assert!(bytes.0.iter().all(|b|*b==0));
    for i in 1..=1000 {assert_eq!(unsafe{w.record(10,i)},i);}
    w.rewind();assert_eq!(unsafe{w.record(14,123)},1001);
    assert_eq!(u64::from_ne_bytes(bytes.0[..8].try_into().unwrap()),14);
    assert_eq!(u64::from_ne_bytes(bytes.0[8..16].try_into().unwrap()),123);
    assert!(bytes.0[8184..].iter().all(|b|*b==0));
}
#[test] #[should_panic] fn refuses_unaligned_alias(){unsafe{workspace::Workspace::admit(1,4096);}}
#[test] #[should_panic] fn refuses_partial_page(){unsafe{workspace::Workspace::admit(4096,4097);}}
