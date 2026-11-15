"""Actual shared provider transcript, compiled without provider/IPC mocks."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class PolicyTranscript(unittest.TestCase):
    def test_actual_transcript(self):
        source = ROOT / 'crates/sex-pdx/src/restricted_helper.rs'
        rust = '#[path="' + str(source) + '"] mod restricted_helper;\nuse restricted_helper::*;\n' + '#[path="' + str(ROOT / 'crates/sex-pdx/src/restricted_helper_journal.rs') + '"] mod journal;\nuse journal as restricted_helper_journal;\n' + '#[path="' + str(ROOT / 'crates/sex-pdx/src/restricted_helper_provider.rs') + '"] mod provider;\n' + r'''
fn subject(pd:u32,task:u32)->Subject{Subject{pd,task}}
fn auth()->Authority{Authority{caller:subject(3,3),provider_epoch:1,challenge:[100,101],reserved:subject(43,43),inactive:true}}
fn packets()->[Packet;11]{
 let fields=[(Phase::Quarantine,subject(43,43).word(),1),(Phase::EchoChallenge,100,101),(Phase::Coordinator,2,1),(Phase::Owner,subject(42,42).word(),9),(Phase::Nonce,10,11),
 (Phase::PluginHashLo,12,13),(Phase::PluginHashHi,14,15),(Phase::InputHashLo,16,17),(Phase::InputHashHi,18,19),
 (Phase::Bind,subject(43,1073741824).word(),20),(Phase::Seal,subject(43,1073741824).word(),1)];
 fields.map(|(phase,a,b)|Packet{generation:7,phase,a,b})
}
#[test]fn full_order_exact_retry_and_wire_roundtrip(){
 let mut t=Transcript::new();let p=packets();
 for (i,packet)in p.into_iter().enumerate(){assert_eq!(Packet::decode(packet.wire().unwrap()),Some(packet));
 assert_eq!(t.accept(packet,auth(),Some(subject(43,1073741824))),Ok(if i==0{Accepted::Quarantined}else if i==10{Accepted::Sealed}else{Accepted::Staged}));
 assert_eq!(t.accept(packet,auth(),Some(subject(43,1073741824))),Ok(Accepted::Duplicate));assert_eq!(t.sealed(),i==10);}
 let b=t.binding().unwrap();assert_eq!(b.parent,subject(42,42));assert_eq!(b.nonce,[10,11]);assert_eq!(b.plugin_hash,[12,13,14,15]);assert_eq!(b.input_hash,[16,17,18,19]);assert_eq!(b.cookie,9);assert_eq!(b.ticket,20);
}
#[test]fn forged_caller_reservation_live_row_and_epoch_refused(){
 for a in [Authority{caller:subject(3,4),..auth()},Authority{caller:subject(4,3),..auth()},Authority{inactive:false,..auth()},Authority{reserved:subject(44,43),..auth()},Authority{challenge:[0;2],..auth()},Authority{provider_epoch:2,..auth()}]{let mut t=Transcript::new();assert!(t.accept(packets()[0],a,None).is_err());assert!(!t.sealed());}
 let mut t=Transcript::new();t.accept(packets()[0],auth(),None).unwrap();assert!(t.accept(packets()[1],Authority{provider_epoch:2,..auth()},None).is_err());assert!(t.poisoned());assert!(!t.sealed());
}
#[test]fn missing_changed_and_foreign_task_never_seal(){
 let p=packets();
 for skip in 1..11{let mut t=Transcript::new();for (i,packet)in p.into_iter().enumerate(){if i!=skip{let _=t.accept(packet,auth(),Some(subject(43,1073741824)));}}assert!(!t.sealed());}
 let mut t=Transcript::new();t.accept(p[0],auth(),None).unwrap();t.accept(p[1],auth(),None).unwrap();assert_eq!(t.accept(Packet{b:10,..p[1]},auth(),None),Err(Refused::ChangedRetry));assert!(!t.sealed());
 let mut t=Transcript::new();for packet in &p[..9]{t.accept(*packet,auth(),None).unwrap();}assert!(t.accept(p[9],auth(),Some(subject(43,1073741825))).is_err());assert!(!t.sealed());
}
#[test]fn stale_replay_and_bad_challenge_cookie_nonce_bounds(){
 let mut t=Transcript::new();let p=packets();t.accept(p[0],auth(),None).unwrap();assert!(t.accept(Packet{generation:6,..p[1]},auth(),None).is_err());
 assert!(t.accept(Packet{b:102,..p[1]},auth(),None).is_err());assert!(t.poisoned());assert!(!t.sealed());
 let mut t=Transcript::new();for packet in &p[..3]{t.accept(*packet,auth(),None).unwrap();}
 assert!(t.accept(Packet{b:1<<32,..p[3]},auth(),None).is_err());assert!(!t.sealed());t.accept(p[3],auth(),None).unwrap();assert!(t.accept(Packet{a:0,b:0,..p[4]},auth(),None).is_err());assert!(!t.sealed());
 assert!(Packet{generation:0,..p[0]}.wire().is_none());assert!(Packet{generation:MAX_GENERATION+1,..p[0]}.wire().is_none());assert!(Packet::decode([0,0,0]).is_none());assert!(Packet::decode([0xff00_0000_0000_0001,0,0]).is_none());
}

#[test]fn one_word_credits_require_exact_phase_epoch_order_and_admission(){
 let packet=packets()[0];let phase=packet.wire().unwrap()[0];let mut c=ReplyCredits::new(packet,1,[100,101]).unwrap();
 assert_eq!(c.first(1),Some(phase));assert_eq!(c.first(2),None);
 assert!(c.fetch([phase,1,1],1).is_none());assert!(!c.complete());
 assert!(!c.first_delivered(2));assert!(c.first_delivered(1));
 assert!(c.fetch([phase,2,1],1).is_none());assert!(c.fetch([phase,1,2],1).is_none());assert!(c.fetch([phase+1,1,1],1).is_none());
 for (selector,expected)in [(1,packet.a),(2,packet.b),(3,100),(4,101)]{
  let request=[phase,selector,1];assert_eq!(c.fetch(request,1),Some(expected));
  // Kernel admission Full: peeking/retrying must not consume a credit.
  assert!(!c.complete());assert_eq!(c.fetch(request,1),Some(expected));
  assert!(c.fetch_delivered(request,1));assert_eq!(c.fetch(request,1),Some(expected));
  assert!(c.fetch_delivered(request,1));
 }
 assert!(c.complete());assert!(c.fetch([phase,1,1],1).is_none());
 assert!(c.fetch([phase,4,1],2).is_none());assert!(c.fetch([phase,5,1],1).is_none());
 let ordinary=packets()[1];let phase=ordinary.wire().unwrap()[0];let mut c=ReplyCredits::new(ordinary,1,[100,101]).unwrap();c.first_delivered(1);
 for selector in [1,2]{assert!(c.fetch_delivered([phase,selector,1],1));}assert!(c.complete());assert!(c.fetch([phase,3,1],1).is_none());
}

#[test]fn kept_journal_never_restores_positive_rights_and_corruption_denies_all(){
 use journal::*;let page=Journal::new();assert_eq!(page.address()%4096,0);
 assert!(!page.load(Some(1),true).denied(43));assert!(page.load(Some(2),true).all_denied());assert!(page.load(Some(1),false).all_denied());
 let mut ledger=Ledger::empty();assert!(ledger.quarantine(subject(43,1073741824),7));page.persist(&ledger);
 let recovered=page.load(Some(2),true);assert!(recovered.denied(43));assert!(!recovered.denied(42));
 assert!(!ledger.quarantine(subject(43,43),6));assert!(!ledger.quarantine(subject(43,43),7));assert!(ledger.denied(43));
 assert!(ledger.quarantine(subject(43,1073741825),8));assert!(ledger.denied(43));
 let words=ledger.words();for index in [0,1,2,3,8,9,10,511]{let mut broken=words;broken[index]^=1;assert!(Ledger::recover(&broken,Some(2)).all_denied());}
 for prefix in 1..512{let mut partial=[0u64;512];partial[1..prefix].copy_from_slice(&words[1..prefix]);assert!(Ledger::recover(&partial,Some(2)).all_denied());}
 assert!(Ledger::recover(&[0;512],None).all_denied());assert!(Ledger::recover(&words,Some(0)).all_denied());
}
#[test]fn exhausted_tombstones_fail_closed_without_overwrite(){
 let mut ledger=journal::Ledger::empty();for n in 0..journal::CAPACITY{assert!(ledger.quarantine(subject(100+n as u32,100+n as u32),n as u64+1));}
 assert!(!ledger.quarantine(subject(200,200),20));for n in 0..journal::CAPACITY{assert!(ledger.denied(100+n as u32));}
}

struct Fake {epoch:u64,now:u64,full:bool,keep:bool,replies:Vec<u64>}
impl provider::Platform for Fake {
 fn epoch(&mut self)->Option<u64>{Some(self.epoch)}fn keep(&mut self,_:u64)->bool{self.keep}
 fn challenge(&mut self)->Option<[u64;2]>{Some([100,101])}
 fn catalog(&mut self,target:u32)->Option<Subject>{if target==43{Some(subject(43,if self.now==0{43}else{1073741824}))}else{None}}
 fn now(&mut self)->u64{self.now}fn reply(&mut self,w:u64)->bool{if self.full{false}else{self.replies.push(w);true}}
}
fn fake()->Fake{Fake{epoch:1,now:0,full:false,keep:true,replies:Vec::new()}}
fn message(packet:Packet)->provider::Message{provider::Message{caller:subject(3,3),opcode:OP_POLICY,words:packet.wire().unwrap()}}
fn credits(state:&mut provider::Provider,page:&journal::Journal,p:&mut Fake,packet:Packet){
 let phase=packet.wire().unwrap()[0];state.poll(p,|_|true);assert_eq!(p.replies.pop(),Some(phase));
 for selector in 1..=if packet.phase==Phase::Quarantine{4}else{2}{
  state.message(provider::Message{caller:subject(3,3),opcode:OP_FETCH,words:[phase,selector,1]},page,p);
  state.poll(p,|_|true);let expected=match selector{1=>packet.a,2=>packet.b,3=>100,4=>101,_=>unreachable!()};assert_eq!(p.replies.pop(),Some(expected));
 }
}
#[test]fn provider_persist_scrub_and_actual_reply_full_precede_ack(){
 let page=journal::Journal::new();let mut p=fake();let mut state=provider::Provider::new();state.initialize(&page,&mut p);let q=packets()[0];
 assert!(!state.denied(43));state.message(message(q),&page,&mut p);assert!(state.denied(43));assert!(page.load(Some(2),true).denied(43));
 state.poll(&mut p,|_|false);assert!(p.replies.is_empty());assert!(!state.sealed());
 // A next command before quarantine ACK/challenge credits cannot advance.
 state.message(message(packets()[1]),&page,&mut p);p.full=true;state.poll(&mut p,|_|true);assert!(p.replies.is_empty());
 p.full=false;credits(&mut state,&page,&mut p,q);
 for packet in packets().into_iter().skip(1){if packet.phase==Phase::Bind{p.now=1;}state.message(message(packet),&page,&mut p);credits(&mut state,&page,&mut p,packet);}
 assert!(state.sealed());assert!(state.denied(43));assert!(page.load(Some(2),true).denied(43));
 let binding=state.binding().unwrap();assert!(state.job_live(binding,p.now));
 p.now=provider::JOB_TICKS+1;assert!(!state.job_live(binding,p.now));
 state.poll(&mut p,|_|false);assert!(!state.job_live(binding,p.now));
 state.poll(&mut p,|_|true);assert!(!state.job_live(binding,p.now));
 // Replayed final Seal/ACK must not extend or resurrect an expired job lease.
 state.message(message(packets()[10]),&page,&mut p);credits(&mut state,&page,&mut p,packets()[10]);assert!(!state.job_live(binding,p.now));
}
#[test]fn provider_restart_deadline_foreign_fetch_and_epoch_never_resume_positive_state(){
 let page=journal::Journal::new();let mut p=fake();let mut state=provider::Provider::new();state.initialize(&page,&mut p);let q=packets()[0];
 let mut foreign=message(q);foreign.caller=subject(3,4);state.message(foreign,&page,&mut p);assert!(!state.denied(43));
 state.message(message(q),&page,&mut p);state.poll(&mut p,|_|false);p.now=provider::TRANSCRIPT_TICKS;state.poll(&mut p,|_|true);assert!(p.replies.is_empty());assert!(!state.sealed());assert!(state.denied(43));
 p.epoch=2;state.initialize(&page,&mut p);assert!(state.denied(43));assert!(!state.sealed());state.message(message(q),&page,&mut p);state.poll(&mut p,|_|true);assert!(p.replies.is_empty());
 state.message(provider::Message{caller:subject(3,3),opcode:OP_FETCH,words:[q.wire().unwrap()[0],1,1]},&page,&mut p);state.poll(&mut p,|_|true);assert!(p.replies.is_empty());
 p.keep=false;state.initialize(&page,&mut p);assert!(state.denied(42));assert!(state.denied(43));
}

'''
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'transcript.rs'
            path.write_text(rust)
            binary = Path(d) / 'tests'
            for command in (['rustc', '--edition=2021', '--test', str(path), '-o', str(binary)], [str(binary)]):
                result = subprocess.run(command, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    unittest.main()
