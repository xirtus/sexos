#![allow(dead_code)]
extern crate self as sex_pdx;
pub use sex_trust_config as trust_config;
#[path="../crates/sex-pdx/src/app_catalog.rs"]pub mod app_catalog;
#[path="../crates/sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../crates/sex-pdx/src/entropy_seed.rs"]pub mod entropy_seed;
#[path="../crates/sex-pdx/src/entropy_source.rs"]pub mod entropy_source;
#[path="../crates/sex-pdx/src/certificate_time.rs"]pub mod certificate_time;
#[path="../crates/sex-pdx/src/clock_source.rs"]pub mod clock_source;
#[path="../crates/sex-pdx/src/trust_policy.rs"]pub mod trust_policy;
#[path="../crates/sex-pdx/src/trust_source.rs"]pub mod trust_source;
#[path="../servers/sexnet/src/entropy.rs"]mod entropy;
#[path="../servers/sexnet/src/certificate_time.rs"]mod clock;
#[path="../servers/sexnet/src/trust_policy.rs"]mod trust;
#[path="../servers/sexnet/src/source_denial.rs"]mod source_denial;
use source_denial::{Identity,Kind,Outbox,OWNERS_PER_KIND,TOTAL_SLOTS};
const KINDS:[Kind;3]=[Kind::Entropy,Kind::CertificateTime,Kind::Trust];
const OWNER:Identity=Identity{pd:12,task:120};
fn decode(kind:Kind,word:u64,seq:u32)->Result<u32,u32>{
    match kind{Kind::Entropy=>entropy_seed::decode_reply(word,seq),
        Kind::CertificateTime=>certificate_time::decode_reply(word,seq),Kind::Trust=>trust_policy::decode_reply(word,seq)}
}
fn denied(kind:Kind)->u32{match kind{Kind::Entropy=>entropy_seed::DENIED,Kind::CertificateTime=>certificate_time::DENIED,Kind::Trust=>trust_policy::DENIED}}
fn stale(kind:Kind)->u32{match kind{Kind::Entropy=>entropy_seed::STALE,Kind::CertificateTime=>certificate_time::STALE,Kind::Trust=>trust_policy::STALE}}
fn max_seq(kind:Kind)->u32{match kind{Kind::Entropy=>entropy_seed::MAX_SEQ,Kind::CertificateTime=>certificate_time::MAX_SEQ,Kind::Trust=>trust_policy::MAX_SEQ}}

#[test]fn denial_storage_budget_is_finite_and_contains_no_machine_backing(){
    assert_eq!(OWNERS_PER_KIND,19);assert_eq!(TOTAL_SLOTS,57);
    println!("[source.denial.host.storage] bytes={} slots={} owners_per_kind={}",core::mem::size_of::<Outbox>(),TOTAL_SLOTS,OWNERS_PER_KIND);
    assert!(core::mem::size_of::<Outbox>()<4096);
}
#[test]fn every_kind_uses_actual_typed_error_word_and_finite_sequence(){
    for kind in KINDS{let mut out=Outbox::new();for seq in [1,max_seq(kind)]{
        assert!(out.deny(kind,OWNER,seq));let(owner,word)=out.pending_reply().unwrap();assert_eq!(owner,OWNER);
        assert_eq!(decode(kind,word,seq),Err(denied(kind)));out.reply_accepted();
    }}
}
#[test]fn full_and_duplicate_keep_exact_receipt_while_different_pending_sequence_backpressures(){
    for kind in KINDS{let mut out=Outbox::new();assert!(out.deny(kind,OWNER,10));let original=out.pending_reply();
        for _ in 0..4{assert!(out.deny(kind,OWNER,10));assert!(!out.deny(kind,OWNER,9));assert!(!out.deny(kind,OWNER,11));
            assert_eq!(out.pending_reply(),original);out.reply_deferred();assert_eq!(out.pending_reply(),original);}
        out.reply_accepted();assert!(out.pending_reply().is_none());assert!(out.deny(kind,OWNER,10));assert_eq!(out.pending_reply(),original);
        out.reply_accepted();assert!(out.deny(kind,OWNER,11));assert_eq!(decode(kind,out.pending_reply().unwrap().1,11),Err(denied(kind)));
    }
}
#[test]fn stale_replies_never_lower_or_replace_latest_denied_cache_including_last_sequence(){
    for kind in KINDS{let mut out=Outbox::new();let high=max_seq(kind);
        assert!(out.deny(kind,OWNER,high));let latest=out.pending_reply();out.reply_accepted();
        for seq in [high-1,1,high-2]{
            assert!(out.deny(kind,OWNER,seq));let old=out.pending_reply();assert_eq!(decode(kind,old.unwrap().1,seq),Err(stale(kind)));
            assert!(out.deny(kind,OWNER,seq));assert_eq!(out.pending_reply(),old);assert!(!out.deny(kind,OWNER,high));
            out.reply_accepted();
            if seq+1<high{assert!(out.deny(kind,OWNER,seq+1));assert_eq!(decode(kind,out.pending_reply().unwrap().1,seq+1),Err(stale(kind)));out.reply_accepted();}
            assert!(out.deny(kind,OWNER,high));assert_eq!(out.pending_reply(),latest);out.reply_accepted();
        }
    }
}
#[test]fn all_eight_unsupported_spindles_and_protocols_receive_fair_full_retained_turns(){
    let mut out=Outbox::new();let targets=[12,17,24,25,26,27,28,29];
    for kind in KINDS{for pd in targets{assert!(out.deny(kind,Identity{pd,task:pd*10},1));}}
    let mut first=Vec::new();for _ in 0..24{first.push(out.pending_reply().unwrap());out.reply_deferred();}
    assert_eq!(first.iter().map(|(id,word)|(id.pd,id.task,*word)).collect::<std::collections::HashSet<_>>().len(),24);
    for expected in &first{assert_eq!(out.pending_reply(),Some(*expected));out.reply_deferred();}
    for expected in &first{assert_eq!(out.pending_reply(),Some(*expected));out.reply_accepted();}
    assert!(out.pending_reply().is_none());
    for kind in KINDS{for pd in targets{assert!(out.deny(kind,Identity{pd,task:pd*10},1));}}
    for expected in &first{assert_eq!(out.pending_reply(),Some(*expected));out.reply_accepted();}
}
#[test]fn capacity_is_per_kind_and_never_evicts_pending_or_cached_owner_fences(){
    let mut out=Outbox::new();let owners:Vec<_>=(0..OWNERS_PER_KIND).map(|n|Identity{pd:100+n as u32,task:1000+n as u32}).collect();
    let extra=Identity{pd:999,task:9990};
    for owner in &owners{assert!(out.deny(Kind::Entropy,*owner,10));}
    let held=out.pending_reply();assert!(!out.deny(Kind::Entropy,extra,1));assert_eq!(out.pending_reply(),held);
    assert!(out.deny(Kind::CertificateTime,extra,1)); // independent protocol partition
    for _ in 0..OWNERS_PER_KIND+1{out.reply_accepted();}
    assert!(out.pending_reply().is_none());assert!(!out.deny(Kind::Entropy,extra,1));
    assert!(!out.retire(Identity{task:owners[0].task+1,..owners[0]}));assert!(!out.deny(Kind::Entropy,extra,1));
    assert!(out.retire(owners[0]));assert!(out.deny(Kind::Entropy,extra,1));
    // A different cached owner remains stale-fenced despite the freed slot.
    assert!(out.deny(Kind::Entropy,owners[1],9));
    let mut found=false;while let Some((owner,word))=out.pending_reply(){if owner==owners[1]{assert_eq!(decode(Kind::Entropy,word,9),Err(entropy_seed::STALE));found=true;}out.reply_accepted();}
    assert!(found);
}
#[test]fn retirement_is_exact_and_clears_only_that_identity_across_all_partitions(){
    let mut out=Outbox::new();let peer=Identity{task:121,..OWNER};
    for kind in KINDS{assert!(out.deny(kind,OWNER,10));assert!(out.deny(kind,peer,20));}
    assert!(!out.retire(Identity{task:122,..OWNER}));assert!(out.retire(OWNER));assert!(!out.retire(OWNER));
    for _ in 0..3{assert_eq!(out.pending_reply().unwrap().0,peer);out.reply_accepted();}
    assert!(out.pending_reply().is_none());
    for kind in KINDS{assert!(out.deny(kind,peer,19));let actual=out.pending_reply().unwrap();assert_eq!(actual.0,peer);assert_eq!(decode(kind,actual.1,19),Err(stale(kind)));out.reply_accepted();}
    for kind in KINDS{assert!(out.deny(kind,OWNER,1));let actual=out.pending_reply().unwrap();assert_eq!(actual.0,OWNER);assert_eq!(decode(kind,actual.1,1),Err(denied(kind)));out.reply_accepted();}
}
#[test]fn invalid_identity_zero_and_high_sequence_aliases_have_no_effect(){
    let mut out=Outbox::new();assert!(out.deny(Kind::Trust,OWNER,10));let before=out.pending_reply();
    for kind in KINDS{
        for owner in [Identity{pd:0,..OWNER},Identity{task:0,..OWNER}]{assert!(!out.deny(kind,owner,1));assert!(!out.retire(owner));assert_eq!(out.pending_reply(),before);}
        for seq in [0,max_seq(kind)+1,1<<31,u32::MAX]{assert!(!out.deny(kind,OWNER,seq));assert_eq!(out.pending_reply(),before);}
    }
}

#[test]fn eight_denial_callers_leave_all_three_actual_authorized_machines_and_clock_health_untouched(){
    let mut denied_out=Outbox::new();
    let e_owner=entropy_source::Identity{pd:15,task:150};
    let c_owner=clock_source::Identity{pd:15,task:150};
    let t_owner=trust_source::Identity{pd:15,task:150};
    let mut e=entropy::Machine::new();let mut c=clock::Machine::new();let mut t=trust::Machine::new();
    let begin=entropy_seed::Request{seq:1,kind:entropy_seed::BEGIN,offset:0,length:32,ticket:0,a:0};
    assert!(e.handle(e_owner,begin,1,Some(entropy_source::Info{consumer:e_owner,available:true})));
    let seconds=clock_source::MIN_SECONDS+100;let mut clock_calls=0;let mut trust_calls=0;
    assert!(c.handle(c_owner,certificate_time::Request{seq:1},Some(clock_source::Info{consumer:c_owner,source:clock_source::CMOS_UTC}),||{
        clock_calls+=1;Ok(clock_source::Snapshot{seconds,attempts:1})
    }));
    assert!(t.handle(t_owner,trust_policy::Request{seq:1},||{
        trust_calls+=1;Ok(trust_source::Info{consumer:t_owner,profile:trust_config::PROFILE,
            root_bytes:trust_config::ROOT_DER.len(),version:trust_config::VERSION})
    }));
    let entropy_before=e.snapshot();let time_before=c.snapshot();
    let entropy_pending=e.pending_reply();let time_pending=c.pending_reply();let trust_pending=t.pending_reply();
    // This is a host composition test: the parent route must choose this lane
    // after held identity + typed decode; the outbox has no Machine reference.
    for pd in [12,17,24,25,26,27,28,29]{for kind in KINDS{
        assert!(denied_out.deny(kind,Identity{pd,task:pd*10},1));
    }}
    assert_eq!(e.snapshot(),entropy_before);assert_eq!(c.snapshot(),time_before);
    assert_eq!(e.pending_reply(),entropy_pending);assert_eq!(c.pending_reply(),time_pending);assert_eq!(t.pending_reply(),trust_pending);
    assert_eq!((clock_calls,trust_calls),(1,1));assert!(e.private_scrubbed());
    // No unsupported identity consumed any of the six normal owner fences.
    // Exact retire(false) is an observable probe of the actual private caches.
    for pd in [12,17,24,25,26,27,28,29]{
        assert!(!e.retire(entropy_source::Identity{pd,task:pd*10}));
        assert!(!c.retire(clock_source::Identity{pd,task:pd*10}));
        assert!(!t.retire(trust_source::Identity{pd,task:pd*10}));
    }
    assert_eq!(e.snapshot(),entropy_before);assert_eq!(c.snapshot(),time_before);
    assert_eq!(t.pending_reply(),trust_pending);
    let ticket=entropy_seed::decode_reply(entropy_pending.unwrap().1,1).unwrap();e.reply_accepted();
    // Authorized CANCEL/RELEASE remain in the original Machine and can still
    // quiesce normally despite every unsupported caller's denial being Full.
    for(seq,kind)in[(2,entropy_seed::CANCEL),(3,entropy_seed::RELEASE)]{
        assert!(e.handle(e_owner,entropy_seed::Request{seq,kind,offset:0,length:0,ticket,a:0},seq as u64,None));
        assert_eq!(entropy_seed::decode_reply(e.pending_reply().unwrap().1,seq),Ok(0));e.reply_accepted();
    }
    assert!(!e.busy()&&e.private_scrubbed());
    assert!(e.handle(e_owner,entropy_seed::Request{seq:4,..begin},4,Some(entropy_source::Info{consumer:e_owner,available:true})));
    assert_eq!(entropy_seed::decode_reply(e.pending_reply().unwrap().1,4),Ok(ticket+1));
    // Denial retirement cannot reset the existing boot-global UTC health.
    c.reply_accepted();assert!(c.handle(c_owner,certificate_time::Request{seq:2},Some(clock_source::Info{consumer:c_owner,source:clock_source::CMOS_UTC}),||{
        clock_calls+=1;Ok(clock_source::Snapshot{seconds:seconds-1,attempts:1})
    }));
    let rollback=c.snapshot();assert_eq!(rollback,clock::Snapshot{high_water:Some(seconds),rollback:true});
    assert_eq!(certificate_time::decode_reply(c.pending_reply().unwrap().1,2),Err(certificate_time::ROLLBACK));
    for pd in [12,17,24,25,26,27,28,29]{assert!(denied_out.retire(Identity{pd,task:pd*10}));}
    assert_eq!(c.snapshot(),rollback);assert_eq!((clock_calls,trust_calls),(2,1));
    assert_eq!(t.pending_reply(),trust_pending);assert!(denied_out.pending_reply().is_none());
}
