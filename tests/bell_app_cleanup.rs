// SPDX-License-Identifier: GPL-3.0-only
//! Includes the exact production Bell queue, tables and retirement body. The
//! gate extracts these without replacing any resource transition with a model.
#![allow(dead_code, static_mut_refs)]
include!(env!("BELL_CLEANUP_SOURCE"));
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
unsafe fn reset() {
    BELL_QUEUE=BellQueue::new();BELL_GENERATION=41;
    MUTE_LIST=[0;MUTE_LIST_CAPACITY];MUTE_COUNT=0;
    POLICY_TABLE=[PolicyEntry{target_pd:0,active_flags:0,privacy_level:0,lane_override:0,force_mute:0};POLICY_TABLE_CAPACITY];POLICY_COUNT=0;
    SPAM_BUDGET.slots=[(0,0,0);SPAM_BUDGET_SLOTS];
}
fn entry(id:u64,pd:u32)->BellQueueEntry {
    BellQueueEntry{event_id:id,caller_pd:pd,category:3,requested_lane:2,final_lane:4,
        final_urgency:3,privacy_level:2,redaction_class:1,action_count:1,action_id:id as u8,
        object_ref_count:1,object_ref:(id+1)as u8,dismissed:(id%2)as u8,_pad:[0;2]}
}
fn identity(e:BellQueueEntry)->(u64,u32,[u8;11]) {
    (e.event_id,e.caller_pd,[e.category,e.requested_lane,e.final_lane,e.final_urgency,
    e.privacy_level,e.redaction_class,e.action_count,e.action_id,e.object_ref_count,e.object_ref,e.dismissed])
}
unsafe fn fill(head:usize,count:usize,pattern:usize) -> Vec<BellQueueEntry> {
    BELL_QUEUE.head=head as u16;BELL_QUEUE.count=count as u16;BELL_QUEUE.tail=((head+count)%BELL_QUEUE_CAPACITY) as u16;
    BELL_QUEUE.next_event_id=1000;
    let mut expected=Vec::new();
    for i in 0..count {let pd=if (i+pattern)%3==0{15}else{9};let e=entry(100+i as u64,pd);
        BELL_QUEUE.entries[(head+i)%BELL_QUEUE_CAPACITY]=e;if pd!=15{expected.push(e);}}
    expected
}
unsafe fn assert_queue(expected:&[BellQueueEntry]) {
    assert_eq!(BELL_QUEUE.head,0);assert_eq!(BELL_QUEUE.count as usize,expected.len());
    assert_eq!(BELL_QUEUE.tail as usize,expected.len()%BELL_QUEUE_CAPACITY);
    for (actual,want) in BELL_QUEUE.entries.iter().zip(expected) {assert_eq!(identity(*actual),identity(*want));}
    for e in &BELL_QUEUE.entries[expected.len()..] {assert_eq!(identity(*e),identity(BellQueue::new().entries[0]));}
    assert_eq!(BELL_QUEUE.next_event_id,1000);
}
#[test]fn full_wrapped_ring_preserves_survivor_order_ids_payloads_and_callbacks(){
    let _g=TEST_LOCK.lock().unwrap();unsafe{reset();let want=fill(11,16,0);
        assert_eq!(retire_app_resources(15),16-want.len());assert_queue(&want);assert_eq!(BELL_GENERATION,42);
        assert!(BELL_QUEUE.entries.iter().all(|e|e.caller_pd!=15));
    }
}
#[test]fn every_head_count_and_mixed_sender_pattern_compacts_without_loss(){
    let _g=TEST_LOCK.lock().unwrap();unsafe{for head in 0..16{for count in 0..=16{for pattern in 0..3{
        reset();let want=fill(head,count,pattern);assert_eq!(retire_app_resources(15),count-want.len());assert_queue(&want);
    }}}}
}
#[test]fn policy_mute_and_spam_cleanup_is_exact_and_erases_tail_storage(){
    let _g=TEST_LOCK.lock().unwrap();unsafe{reset();
        MUTE_COUNT=5;MUTE_LIST[..5].copy_from_slice(&[9,15,12,15,7]);
        POLICY_COUNT=5;for(i,pd)in[9,15,12,15,7].into_iter().enumerate(){POLICY_TABLE[i]=PolicyEntry{target_pd:pd,active_flags:i as u8+1,privacy_level:i as u8,lane_override:i as u8,force_mute:1};}
        SPAM_BUDGET.slots[0]=(9,7,90);SPAM_BUDGET.slots[4]=(15,8,100);SPAM_BUDGET.slots[8]=(12,6,110);SPAM_BUDGET.slots[15]=(15,3,120);
        assert_eq!(retire_app_resources(15),0);assert_eq!(MUTE_COUNT,3);assert_eq!(&MUTE_LIST[..3],&[9,12,7]);assert!(MUTE_LIST[3..].iter().all(|x|*x==0));
        assert_eq!(POLICY_COUNT,3);for(i,(pd,flags,privacy))in[(9,1,0),(12,3,2),(7,5,4)].into_iter().enumerate(){let e=POLICY_TABLE[i];assert_eq!((e.target_pd,e.active_flags,e.privacy_level,e.lane_override,e.force_mute),(pd,flags,privacy,privacy,1));}
        assert!(POLICY_TABLE[3..].iter().all(|e|e.target_pd==0&&e.active_flags==0&&e.force_mute==0));
        assert_eq!(SPAM_BUDGET.slots[0],(9,7,90));assert_eq!(SPAM_BUDGET.slots[8],(12,6,110));assert_eq!(SPAM_BUDGET.slots[4],(0,0,0));assert_eq!(SPAM_BUDGET.slots[15],(0,0,0));
    }
}
#[test]fn all_target_entries_become_empty_and_next_event_id_is_not_reused(){
    let _g=TEST_LOCK.lock().unwrap();unsafe{reset();fill(7,16,0);for e in BELL_QUEUE.entries.iter_mut(){e.caller_pd=15;}
        assert_eq!(retire_app_resources(15),16);assert_queue(&[]);
        let(id,dropped)=BELL_QUEUE.push(12,1,1,2,1,0,0,1,77,1,88).unwrap();assert_eq!((id,dropped),(1000,None));assert_eq!(BELL_QUEUE.next_event_id,1001);
        assert_eq!((BELL_QUEUE.entries[0].caller_pd,BELL_QUEUE.entries[0].action_id,BELL_QUEUE.entries[0].object_ref),(12,77,88));
    }
}
#[test]fn target_absent_keeps_full_ring_and_counter_including_dismissed_events(){
    let _g=TEST_LOCK.lock().unwrap();unsafe{reset();fill(13,16,0);let want:Vec<_>=(0..16).map(|i|BELL_QUEUE.entries[(13+i)%16]).collect();
        assert_eq!(retire_app_resources(99),0);assert_queue(&want);assert!(BELL_QUEUE.is_full());
    }
}
#[test]fn repeat_retirement_cannot_remove_new_other_sender_events(){
    let _g=TEST_LOCK.lock().unwrap();unsafe{reset();fill(14,6,0);retire_app_resources(15);
        let(id,_)=BELL_QUEUE.push(12,2,1,1,2,3,2,1,55,1,66).unwrap();assert_eq!(id,1000);
        let want:Vec<_>=BELL_QUEUE.entries[..BELL_QUEUE.count as usize].iter().copied().map(identity).collect();
        assert_eq!(retire_app_resources(15),0);assert_eq!(BELL_QUEUE.next_event_id,1001);
        assert_eq!(BELL_QUEUE.entries[..BELL_QUEUE.count as usize].iter().copied().map(identity).collect::<Vec<_>>(),want);
    }
}
#[test]fn event_ids_near_counter_limit_are_not_renumbered_by_cleanup(){
    let _g=TEST_LOCK.lock().unwrap();unsafe{reset();BELL_QUEUE.head=15;BELL_QUEUE.tail=2;BELL_QUEUE.count=3;BELL_QUEUE.next_event_id=u64::MAX;
        BELL_QUEUE.entries[15]=entry(u64::MAX-3,9);BELL_QUEUE.entries[0]=entry(u64::MAX-2,15);BELL_QUEUE.entries[1]=entry(u64::MAX-1,12);
        assert_eq!(retire_app_resources(15),1);assert_eq!(BELL_QUEUE.next_event_id,u64::MAX);
        assert_eq!(BELL_QUEUE.entries[0].event_id,u64::MAX-3);assert_eq!(BELL_QUEUE.entries[1].event_id,u64::MAX-1);
    }
}
