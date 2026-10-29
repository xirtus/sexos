//! Production sector write body, with only the authenticated block transport mocked.
#![allow(dead_code)]
extern crate self as sex_pdx;
include!("fixtures/diskfs_transport.rs");
#[test]
fn full_sectors_skip_reads_and_partial_boundaries_preserve_neighbors() {
    let _g = TEST_LOCK.lock().unwrap();
    let mut buf = [0; 4096];
    setup(&mut buf);
    let entry = DiskManifestEntryV1 {
        start_lba: 1,
        len_bytes: 4096,
    };
    let data = vec![0x37; 1024];
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 0, &data, buf.as_mut_ptr() as u64),
        Ok(1024)
    );
    {
        let s = STATE.lock().unwrap();
        let s = s.as_ref().unwrap();
        assert_eq!((s.reads, s.writes), (0, 2));
        assert_eq!(&s.bytes[512..1536], &data);
        assert_eq!(s.bytes[1536], 0xA5);
    }
    setup(&mut buf);
    let data = vec![0x42; 1024];
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 100, &data, buf.as_mut_ptr() as u64),
        Ok(1024)
    );
    let s = STATE.lock().unwrap();
    let s = s.as_ref().unwrap();
    assert_eq!((s.reads, s.writes), (2, 3));
    assert_eq!(&s.bytes[612..1636], &data);
    assert_eq!(s.bytes[611], 0xA5);
    assert_eq!(s.bytes[1636], 0xA5);
}
#[test]
fn errors_and_out_of_bounds_never_report_success() {
    let _g = TEST_LOCK.lock().unwrap();
    let mut buf = [0; 4096];
    setup(&mut buf);
    let entry = DiskManifestEntryV1 {
        start_lba: 1,
        len_bytes: 4096,
    };
    STATE.lock().unwrap().as_mut().unwrap().read_error = 91;
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 1, &[1; 16], buf.as_mut_ptr() as u64),
        Err(91)
    );
    assert_eq!(STATE.lock().unwrap().as_ref().unwrap().writes, 0);
    // A full-sector overwrite ignores an irrelevant read failure but still
    // propagates its real write error, without claiming a durable receipt.
    STATE.lock().unwrap().as_mut().unwrap().write_error = 92;
    assert_eq!(
        DiskFs::diskfs_write_object_entry(entry, 0, &[1; 512], buf.as_mut_ptr() as u64),
        Err(92)
    );
    assert!(
        DiskFs::diskfs_write_object_entry(entry, 4090, &[1; 16], buf.as_mut_ptr() as u64).is_err()
    );
}

#[test]
fn bounded_transfers_match_bytes_for_unaligned_edges_page_tails_and_full_documents(){
 let _g=TEST_LOCK.lock().unwrap();let mut storage=[0xcc;4096+32];
 let (page,canary)=storage.split_at_mut(4096);let buf:&mut[u8;4096]=page.try_into().unwrap();
 let entry=DiskManifestEntryV1{start_lba:400,len_bytes:12288};
 for offset in [0,1,100,511,512,513,4095] {for len in [0,1,17,511,512,513,4095,4096,4097,8192] {
  setup(buf);let data:Vec<u8>=(0..len).map(|i|((i*37+offset as usize)%251)as u8).collect();
  assert_eq!(DiskFs::diskfs_write_object_entry(entry,offset,&data,buf.as_mut_ptr()as u64),Ok(len as u64));
  let mut expected=vec![0xa5;MODEL_BYTES];expected[204800+offset as usize..204800+offset as usize+len].copy_from_slice(&data);
  assert_eq!(STATE.lock().unwrap().as_ref().unwrap().bytes,expected);
  let mut out=vec![0;len];assert_eq!(DiskFs::diskfs_read_object_entry(entry,offset,&mut out,buf.as_mut_ptr()as u64),Ok(len as u64));assert_eq!(out,data);
  assert!(canary.iter().all(|b|*b==0xcc));
 }}
 setup(buf);let data:Vec<u8>=(0..12288).map(|i|(i%251)as u8).collect();
 assert_eq!(DiskFs::diskfs_write_object_entry(entry,0,&data,buf.as_mut_ptr()as u64),Ok(12288));
 assert_eq!(STATE.lock().unwrap().as_ref().unwrap().calls,[(true,204800,4096),(true,208896,4096),(true,212992,4096)]);
 let mut out=vec![0;12288];assert_eq!(DiskFs::diskfs_read_object_entry(entry,0,&mut out,buf.as_mut_ptr()as u64),Ok(12288));assert_eq!(out,data);
 assert_eq!(&STATE.lock().unwrap().as_ref().unwrap().calls[3..],[(false,204800,4096),(false,208896,4096),(false,212992,4096)]);
}
#[test]
fn later_transfer_failure_stops_without_replay_or_success_and_preserves_uncertainty(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];let entry=DiskManifestEntryV1{start_lba:400,len_bytes:12288};setup(&mut buf);
 STATE.lock().unwrap().as_mut().unwrap().fail_write_at=2;
 assert_eq!(DiskFs::diskfs_write_object_entry(entry,0,&[0x37;12288],buf.as_mut_ptr()as u64),Err(92));
 {let s=STATE.lock().unwrap();let s=s.as_ref().unwrap();assert_eq!((s.reads,s.writes),(0,2));assert!(s.bytes[204800..208896].iter().all(|b|*b==0x37));assert!(s.bytes[208896..].iter().all(|b|*b==0xa5));}
 setup(&mut buf);STATE.lock().unwrap().as_mut().unwrap().fail_read_at=2;let mut out=[0x76;12288];
 assert_eq!(DiskFs::diskfs_read_object_entry(entry,0,&mut out,buf.as_mut_ptr()as u64),Err(91));
 assert!(out[..4096].iter().all(|b|*b==0xa5));assert!(out[4096..].iter().all(|b|*b==0x76));
 assert_eq!(STATE.lock().unwrap().as_ref().unwrap().reads,2);
}
#[test]
fn invalid_extents_and_integer_wrap_are_rejected_before_any_transport(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];setup(&mut buf);
 for entry in [DiskManifestEntryV1{start_lba:u64::MAX,len_bytes:4096},DiskManifestEntryV1{start_lba:u64::MAX/512,len_bytes:4096}]{
  assert!(DiskFs::diskfs_read_object_entry(entry,0,&mut[0;512],buf.as_mut_ptr()as u64).is_err());
  assert!(DiskFs::diskfs_write_object_entry(entry,0,&[0;512],buf.as_mut_ptr()as u64).is_err());
 }
 let entry=DiskManifestEntryV1{start_lba:1,len_bytes:4096};
 assert!(DiskFs::diskfs_read_object_entry(entry,u64::MAX,&mut[0;512],buf.as_mut_ptr()as u64).is_err());
 assert!(DiskFs::diskfs_write_object_entry(entry,4090,&[0;16],buf.as_mut_ptr()as u64).is_err());
 let entry=DiskManifestEntryV1{start_lba:DISKFS_WRITE_PROOF_LBA-1,len_bytes:4096};
 assert!(DiskFs::diskfs_write_object_entry(entry,0,&[0;1024],buf.as_mut_ptr()as u64).is_err());
 assert!(STATE.lock().unwrap().as_ref().unwrap().calls.is_empty());
}

#[test]fn failed_batched_write_may_have_modified_bytes_and_never_replays_or_claims_success(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];setup(&mut buf);
 {let mut s=STATE.lock().unwrap();let s=s.as_mut().unwrap();s.fail_write_at=2;s.uncertain_write_prefix=719;}
 let entry=DiskManifestEntryV1{start_lba:400,len_bytes:12288};
 assert_eq!(DiskFs::diskfs_write_object_entry(entry,0,&[0x37;12288],buf.as_mut_ptr()as u64),Err(92));
 let s=STATE.lock().unwrap();let s=s.as_ref().unwrap();assert_eq!((s.reads,s.writes),(0,2));
 assert!(s.bytes[204800..208896+719].iter().all(|b|*b==0x37));assert!(s.bytes[208896+719..].iter().all(|b|*b==0xa5));
}

#[test]fn legacy_writes_keep_sector_routes_while_reads_use_the_typed_page_contract(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];setup(&mut buf);
 let entry=DiskManifestEntryV1{start_lba:128,len_bytes:4096};let data=[0x34;4096];
 assert_eq!(DiskFs::diskfs_write_object_entry(entry,0,&data,buf.as_mut_ptr()as u64),Ok(4096));
 {let s=STATE.lock().unwrap();let s=s.as_ref().unwrap();assert_eq!(s.calls.len(),8);assert!(s.calls.iter().all(|c|c.0&&c.2==512));}
 let mut out=[0;4096];assert_eq!(DiskFs::diskfs_read_object_entry(entry,0,&mut out,buf.as_mut_ptr()as u64),Ok(4096));assert_eq!(out,data);
 assert_eq!(STATE.lock().unwrap().as_ref().unwrap().calls.last(),Some(&(false,128*512,4096)));
}
#[test]fn a_write_crossing_pool_boundaries_never_expands_the_bulk_allowlist(){
 let _g=TEST_LOCK.lock().unwrap();let mut buf=[0;4096];
 for (lba,expected) in [(399,vec![(true,399*512,512),(true,400*512,1024)]),(1807,vec![(true,1807*512,512),(true,1808*512,512),(true,1809*512,512)])]{
  setup(&mut buf);let entry=DiskManifestEntryV1{start_lba:lba,len_bytes:1536};let data=[0x67;1536];
  assert_eq!(DiskFs::diskfs_write_object_entry(entry,0,&data,buf.as_mut_ptr()as u64),Ok(1536));
  let s=STATE.lock().unwrap();let s=s.as_ref().unwrap();assert_eq!(s.calls,expected);
  assert_eq!(&s.bytes[lba as usize*512..lba as usize*512+1536],data);
 }
}
