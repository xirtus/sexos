"""Actual shared provider transcript, compiled without provider/IPC mocks."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class PolicyTranscript(unittest.TestCase):
    def test_actual_transcript(self):
        source = ROOT / 'crates/sex-pdx/src/restricted_helper.rs'
        rust = '#[path="' + str(source) + '"] mod wire;\nuse wire::*;\n' + r'''
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
