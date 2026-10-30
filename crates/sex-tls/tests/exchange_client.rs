//! Actual TLS-session transport coordinator, ABI and owned job machine. The
//! public-seeded ClientHello and malformed-wire tests prove no authenticated peer.
extern crate self as sex_pdx;
#[path="../../sex-pdx/src/network_job.rs"]pub mod network_job;
#[path="../../../servers/sexnet/src/jobs.rs"]mod jobs;
#[path="../../../apps/kaleidoscope/src/tls_exchange.rs"]mod exchange;
use std::sync::Mutex;
pub const SLOT_NET:u64=18;pub const ERR_SERVICE_NOT_READY:u64=9;
const OWNER:jobs::Identity=jobs::Identity{pd:15,task:15};
static SERIAL:Mutex<()>=Mutex::new(());
static WORLD:Mutex<Option<World>>=Mutex::new(None);
struct Wire{sent:Vec<u8>,incoming:Vec<u8>,at:usize,begins:usize,finishes:usize,aborts:usize,link:jobs::Link}
impl Wire{fn new()->Self{Self{sent:vec![],incoming:vec![],at:0,begins:0,finishes:0,aborts:0,link:jobs::Link::Open}}}
impl jobs::Wire for Wire{
    fn begin(&mut self,_:u32)->Result<(),u32>{self.begins+=1;Ok(())}
    fn receive(&mut self,out:&mut[u8])->Result<usize,u32>{let n=out.len().min(self.incoming.len()-self.at);out[..n].copy_from_slice(&self.incoming[self.at..self.at+n]);self.at+=n;Ok(n)}
    fn send(&mut self,data:&[u8])->Result<usize,u32>{self.sent.extend_from_slice(data);Ok(data.len())}
    fn queued(&self)->usize{0}fn link(&self)->jobs::Link{self.link}
    fn finish_send(&mut self){self.finishes+=1;}fn abort(&mut self){self.aborts+=1;}
}
struct World{busy_write_once:bool,reject_release_once:bool,synthetic:Option<u64>,machine:jobs::Machine,wire:Wire,now:u64,full:usize,calls:Vec<network_job::Request>,accepted:Vec<network_job::Request>}
fn reset(){*WORLD.lock().unwrap()=Some(World{busy_write_once:false,reject_release_once:false,synthetic:None,machine:jobs::Machine::new(),wire:Wire::new(),now:0,full:0,calls:vec![],accepted:vec![]});}
pub fn pdx_call(slot:u64,op:u64,a:u64,b:u64,c:u64)->(u64,u64){
    assert_eq!((slot,op),(SLOT_NET,network_job::OP));let r=network_job::Request::decode([a,b,c]).unwrap();assert!(r.valid());
    let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();w.calls.push(r);
    if w.full!=0{w.full-=1;return(ERR_SERVICE_NOT_READY,0);}
    assert!(w.accepted.last().is_none_or(|last|r.seq>last.seq),"accepted operation replayed");
    // Explicit error-only fault injection; never simulates TLS authentication
    // or a successful application response. Other operations use actual Machine.
    if w.busy_write_once&&r.kind==network_job::WRITE{w.busy_write_once=false;w.synthetic=Some(network_job::error(r.seq,network_job::BUSY));w.accepted.push(r);return(0,0);}
    if w.reject_release_once&&r.kind==network_job::RELEASE{w.reject_release_once=false;w.synthetic=Some(network_job::error(r.seq,network_job::IO));w.accepted.push(r);return(0,0);}
    let observation=jobs::Resolution{owner:OWNER,generation:3,address:[10,0,2,2],observed_tick:0,expires_tick:60_000};
    assert!(w.machine.handle_resolved(OWNER,r,w.now,false,Ok(observation),&mut w.wire));w.accepted.push(r);(0,0)
}
fn ready()->sex_tls::Session{
    let mut seed=[73;32];let mut s=sex_tls::Session::exchange(&mut seed,1_793_367_907,1).unwrap();assert_eq!(seed,[0;32]);
    for _ in 0..32{if s.poll()==sex_tls::State::HelloPending{return s;}}panic!("no actual ClientHello");
}
fn receipt()->u64{let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();if let Some(word)=w.synthetic.take(){return word;}let(owner,word)=w.machine.pending_reply().unwrap();assert_eq!(owner,OWNER);w.machine.reply_accepted();word}
fn turn(c:&mut exchange::Client,now:u64,deliver:bool){
    {let mut guard=WORLD.lock().unwrap();guard.as_mut().unwrap().now=now;}c.advance(now);
    {let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();w.machine.advance(now,&mut w.wire);}
    if deliver{let pending={let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();w.synthetic.is_some()||w.machine.pending_reply().is_some()};if pending{c.reply(receipt(),now);}}
}
fn finish_cleanup(c:&mut exchange::Client,now:u64){for at in now..now+20{turn(c,at,true);if !c.busy(){return;}}panic!("cleanup did not finish: {:?}",c.view());}
fn until_kind(c:&mut exchange::Client,kind:u8,start:u64)->u64{
    for now in start..start+1000{turn(c,now,false);let pending={let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();w.synthetic.is_some()||w.machine.pending_reply().is_some()};if pending{
        let last=WORLD.lock().unwrap().as_ref().unwrap().accepted.last().unwrap().kind;
        if last==kind{return now;}c.reply(receipt(),now);
    }}panic!("missing operation {kind}");
}
#[test]fn idle_has_no_route_effect_and_start_requires_fresh_session_fence_and_dns_generation(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();c.advance(0);assert_eq!(c.view(),exchange::View::Idle);assert!(!c.busy());
    for(seq,generation)in[(0,3),(1,0),(1,network_job::MAX_TICKET+1),(network_job::MAX_SEQ-3,3)]{assert!(!c.start(ready(),seq,generation));assert_eq!(c.view(),exchange::View::Unavailable);}
    let mut seed=[73;32];let unpolled=sex_tls::Session::exchange(&mut seed,1_793_367_907,1).unwrap();assert!(!c.start(unpolled,1,3));assert!(WORLD.lock().unwrap().as_ref().unwrap().calls.is_empty());
    assert!(c.start(ready(),10,3));assert!(!c.start(ready(),11,3));c.cancel();assert!(!c.busy());assert!(!c.start(ready(),9,3));
}
#[test]fn actual_clienthello_stages_once_and_plain_stall_never_authenticates_or_autofinishes(){
    let _serial=SERIAL.lock().unwrap();reset();let mut expected_session=ready();let mut expected=[0;4096];let n=expected_session.take_outgoing(&mut expected);
    let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));for now in 0..500{turn(&mut c,now,true);}
    let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.wire.sent,&expected[..n]);assert_eq!(w.wire.sent[0],22);assert_eq!(w.wire.sent[5],1);assert_eq!((w.wire.begins,w.wire.finishes),(1,0));
    assert_eq!(c.tx_bytes(),n);assert_eq!(c.rx_bytes(),0);assert_eq!(c.view(),exchange::View::Pending);assert!(!c.peer_authenticated());assert!(c.stats().is_some());assert!(!c.take_authenticated()&&!c.take_response_verified());
}
#[test]fn only_full_admission_retries_exact_same_request_before_effect(){
    let _serial=SERIAL.lock().unwrap();reset();WORLD.lock().unwrap().as_mut().unwrap().full=2;let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));
    for now in 0..3{turn(&mut c,now,true);}let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.calls.len(),3);assert!(w.calls.iter().all(|r|*r==w.calls[0]));assert_eq!(w.accepted.len(),1);assert_eq!(w.machine.owner(),Some(OWNER));
}
#[test]fn full_before_begin_has_finite_deadline_and_no_service_resource(){
    let _serial=SERIAL.lock().unwrap();reset();WORLD.lock().unwrap().as_mut().unwrap().full=usize::MAX;let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));
    turn(&mut c,100,false);turn(&mut c,2099,false);turn(&mut c,2100,false);turn(&mut c,3000,false);assert!(!c.busy());assert_eq!(c.view(),exchange::View::Unavailable);assert_eq!(c.next_sequence(),2);
    let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.calls.len(),2);assert!(w.accepted.is_empty());assert_eq!(w.machine.owner(),None);
}
#[test]fn accepted_begin_timeout_never_replays_and_late_ticket_allows_only_cleanup(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,100,false);
    for now in [2100,2101,3000]{turn(&mut c,now,false);}assert_eq!(c.view(),exchange::View::Uncertain);assert!(c.busy());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),1);
    c.reply(receipt(),3001);finish_cleanup(&mut c,3002);assert_eq!(c.view(),exchange::View::Unavailable);
    let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.accepted.iter().map(|r|r.kind).collect::<Vec<_>>(),vec![network_job::BEGIN_DUPLEX_RESOLVED,network_job::CANCEL,network_job::RELEASE]);assert_eq!(w.machine.owner(),None);
}
#[test]fn accepted_begin_cancel_waits_for_correlated_ticket_and_cannot_publish_late_data(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,1,false);c.cancel();assert!(c.busy());assert!(!c.start(ready(),2,3));
    c.reply(receipt(),2);finish_cleanup(&mut c,3);assert_eq!(c.view(),exchange::View::Cancelled);assert!(!c.peer_authenticated());assert_eq!(c.tx_bytes(),0);assert!(c.start(ready(),c.next_sequence(),3));
}
#[test]fn accepted_write_timeout_retains_only_correlation_and_never_replays_mutation(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));let now=until_kind(&mut c,network_job::WRITE,0);
    turn(&mut c,now+2000,false);assert_eq!(c.view(),exchange::View::Uncertain);let calls=WORLD.lock().unwrap().as_ref().unwrap().calls.len();turn(&mut c,now+2001,false);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),calls);
    c.reply(receipt(),now+2002);finish_cleanup(&mut c,now+2003);assert_eq!(c.view(),exchange::View::Unavailable);assert_eq!(c.tx_bytes(),0);assert!(!c.take_authenticated()&&!c.take_response_verified());
}
#[test]fn wrong_tag_sequence_and_before_admission_receipts_are_ignored(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));c.reply(network_job::reply(1,1),0);assert!(c.busy());turn(&mut c,1,false);
    for word in [network_job::reply(2,1),(0x7Cu64<<56)|(1<<32)|1]{c.reply(word,2);assert_eq!(c.ticket(),0);assert!(c.busy());}
    c.reply(receipt(),3);assert_eq!(c.ticket(),1);c.cancel();finish_cleanup(&mut c,4);
}
#[test]fn deadline_or_backwards_time_in_reply_itself_prevents_initial_mutation(){
    let _serial=SERIAL.lock().unwrap();for now in [99,2100]{reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,100,false);c.reply(receipt(),now);finish_cleanup(&mut c,2101);
        assert_eq!(c.view(),exchange::View::Unavailable);assert_eq!(c.tx_bytes(),0);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().accepted.iter().filter(|r|r.kind==network_job::WRITE).count(),0);}
}
#[test]fn actual_malformed_tls_peer_is_rejected_and_custody_released(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));let now=until_kind(&mut c,network_job::START,0);c.reply(receipt(),now);
    WORLD.lock().unwrap().as_mut().unwrap().wire.incoming=vec![22,3,3,0,4,255,0,0,0];for at in now+1..now+100{turn(&mut c,at,true);if !c.busy(){break;}}
    assert!(!c.busy());assert_eq!(c.view(),exchange::View::Unavailable);assert!(!c.peer_authenticated());assert!(!c.take_authenticated()&&!c.take_response_verified());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner(),None);
}
#[test]fn sequence_exhaustion_reserves_cancel_release_and_never_wraps(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),network_job::MAX_SEQ-4,3));finish_cleanup(&mut c,0);assert_eq!(c.next_sequence(),network_job::MAX_SEQ+1);assert_eq!(c.view(),exchange::View::Unavailable);
    let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();let last=&w.accepted[w.accepted.len()-2..];assert_eq!((last[0].kind,last[0].seq),(network_job::CANCEL,network_job::MAX_SEQ-1));assert_eq!((last[1].kind,last[1].seq),(network_job::RELEASE,network_job::MAX_SEQ));assert_eq!(w.machine.owner(),None);
}
#[test]fn terminal_close_stops_locally_and_exact_parent_retirement_reclaims_service(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,1,false);let word=receipt();c.close();c.reply(word,2);turn(&mut c,3,false);
    assert!(!c.busy());assert!(!c.start(ready(),2,3));assert_eq!(c.view(),exchange::View::Cancelled);let mut guard=WORLD.lock().unwrap();let w=guard.as_mut().unwrap();assert_eq!(w.calls.len(),1);assert_eq!(w.machine.owner(),Some(OWNER));
    assert!(!w.machine.retire(jobs::Identity{pd:15,task:16},&mut w.wire));assert!(w.machine.retire(OWNER,&mut w.wire));assert_eq!(w.machine.owner(),None);
}
#[test]fn invalid_begin_success_cannot_guess_a_ticket_or_forget_uncertain_service(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,1,false);let _actual=receipt();c.reply(network_job::reply(1,0),2);
    assert!(c.busy());assert_eq!(c.view(),exchange::View::Uncertain);turn(&mut c,3,false);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),1);c.close();
}
#[test]fn release_must_have_positive_ack_before_cleanup_finishes(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,0,true);c.cancel();let now=until_kind(&mut c,network_job::RELEASE,1);
    assert!(c.busy());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().machine.owner(),None);let calls=WORLD.lock().unwrap().as_ref().unwrap().calls.len();turn(&mut c,now+1,false);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),calls);
    c.reply(receipt(),now+2);assert!(!c.busy());assert_eq!(c.view(),exchange::View::Cancelled);
}

#[test]fn service_busy_is_fresh_sequence_retry_with_the_exact_unacknowledged_chunk(){
    let _serial=SERIAL.lock().unwrap();reset();WORLD.lock().unwrap().as_mut().unwrap().busy_write_once=true;
    let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,0,true);turn(&mut c,1,true);assert_eq!(c.tx_bytes(),0);turn(&mut c,2,true);assert_eq!(c.tx_bytes(),8);
    let writes={let guard=WORLD.lock().unwrap();guard.as_ref().unwrap().accepted.iter().filter(|r|r.kind==network_job::WRITE).copied().collect::<Vec<_>>()};
    assert_eq!(writes.len(),2);assert_eq!(writes[1].seq,writes[0].seq+1);assert_eq!((writes[1].offset,writes[1].length,writes[1].a),(writes[0].offset,writes[0].length,writes[0].a));c.cancel();finish_cleanup(&mut c,3);assert!(c.private_scrubbed_for_test());
}
#[test]fn pre_effect_full_write_cancel_scrubs_chunk_and_does_not_submit_it_later(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,0,true);WORLD.lock().unwrap().as_mut().unwrap().full=1;turn(&mut c,1,false);c.cancel();assert!(c.private_scrubbed_for_test());finish_cleanup(&mut c,2);
    let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();assert_eq!(w.accepted.iter().filter(|r|r.kind==network_job::WRITE).count(),0);assert_eq!(c.view(),exchange::View::Cancelled);
}
#[test]fn accepted_read_timeout_never_feeds_late_ciphertext_into_discarded_tls_state(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));let now=until_kind(&mut c,network_job::START,0);c.reply(receipt(),now);
    WORLD.lock().unwrap().as_mut().unwrap().wire.incoming=vec![22,3,3,0,4,255,0,0,0];let now=until_kind(&mut c,network_job::READ,now+1);turn(&mut c,now+2000,false);assert_eq!(c.view(),exchange::View::Uncertain);assert!(c.private_scrubbed_for_test());
    c.reply(receipt(),now+2001);finish_cleanup(&mut c,now+2002);assert_eq!(c.rx_bytes(),0);assert_eq!(c.view(),exchange::View::Unavailable);assert!(!c.take_authenticated()&&!c.take_response_verified());
}
#[test]fn impossible_service_status_counters_are_rejected_before_ciphertext_reads(){
    let _serial=SERIAL.lock().unwrap();for state in [network_job::State::Complete,network_job::State::Receiving]{reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));let now=until_kind(&mut c,network_job::STATUS,0);let actual=receipt();let seq=((actual>>32)&network_job::MAX_SEQ as u64)as u32;
        let len=if state==network_job::State::Complete{0}else{network_job::MAX_DUPLEX_BYTES+1};c.reply(network_job::reply(seq,network_job::status(state,len)),now);finish_cleanup(&mut c,now+1);assert_eq!(c.view(),exchange::View::Unavailable);assert!(c.private_scrubbed_for_test());}
}
#[test]fn failed_release_keeps_uncertain_resource_fence_until_parent_terminal_cleanup(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,0,true);WORLD.lock().unwrap().as_mut().unwrap().reject_release_once=true;c.cancel();
    for now in 1..10{turn(&mut c,now,true);}assert!(c.busy());assert_eq!(c.view(),exchange::View::Uncertain);assert!(c.private_scrubbed_for_test());assert!(!c.start(ready(),c.next_sequence(),3));let count=WORLD.lock().unwrap().as_ref().unwrap().calls.len();turn(&mut c,100,false);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),count);c.close();
}
#[test]fn public_cancel_and_close_scrub_owned_session_and_retained_payload_on_every_phase(){
    let _serial=SERIAL.lock().unwrap();for kind in [network_job::BEGIN_DUPLEX_RESOLVED,network_job::WRITE,network_job::START,network_job::STATUS]{reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));let now=until_kind(&mut c,kind,0);c.cancel();assert!(c.private_scrubbed_for_test());c.reply(receipt(),now+1);finish_cleanup(&mut c,now+2);assert!(c.private_scrubbed_for_test());assert_eq!(c.view(),exchange::View::Cancelled);c.close();assert!(c.private_scrubbed_for_test());}
}

#[test]fn prerequisite_rejection_is_local_and_preserves_busy_and_terminal_custody(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();c.reject_prerequisites();assert_eq!(c.view(),exchange::View::Unavailable);assert_eq!(c.next_sequence(),1);assert!(c.private_scrubbed_for_test());assert!(WORLD.lock().unwrap().as_ref().unwrap().calls.is_empty());
    assert!(c.start(ready(),10,3));turn(&mut c,0,false);let seq=c.next_sequence();c.reject_prerequisites();assert_eq!(c.next_sequence(),seq);assert_eq!(c.view(),exchange::View::Pending);assert!(c.busy());assert!(!c.private_scrubbed_for_test());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),1);
    c.reply(receipt(),1);c.cancel();finish_cleanup(&mut c,2);let seq=c.next_sequence();c.reject_prerequisites();assert_eq!(c.next_sequence(),seq);assert_eq!(c.view(),exchange::View::Unavailable);
    c.close();c.reject_prerequisites();assert_eq!(c.view(),exchange::View::Cancelled);assert_eq!(c.next_sequence(),seq);assert!(c.private_scrubbed_for_test());
}

#[test]fn idle_cancel_clears_previous_released_attempt_metadata_but_busy_cancel_keeps_current_ticket(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),10,3));turn(&mut c,0,true);assert_eq!(c.ticket(),1);assert!(c.stats().is_some());c.cancel();assert_eq!(c.ticket(),1);finish_cleanup(&mut c,1);
    assert_eq!(c.ticket(),1);assert_eq!(c.view(),exchange::View::Cancelled);let seq=c.next_sequence();let calls=WORLD.lock().unwrap().as_ref().unwrap().calls.len();
    c.cancel();assert_eq!(c.next_sequence(),seq);assert_eq!(c.ticket(),0);assert_eq!(c.stats(),None);assert_eq!((c.tx_bytes(),c.rx_bytes()),(0,0));assert!(!c.peer_authenticated());assert_eq!(c.view(),exchange::View::Cancelled);assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),calls);
    assert!(c.start(ready(),seq,3));turn(&mut c,10,true);assert_eq!(c.ticket(),2);c.cancel();assert_eq!(c.ticket(),2);finish_cleanup(&mut c,11);c.close();let ticket=c.ticket();c.cancel();assert_eq!(c.ticket(),ticket);assert_eq!(c.view(),exchange::View::Cancelled);
}

fn actual_tls_failure(c:&mut exchange::Client,start:u64)->u64{
    let now=until_kind(c,network_job::START,start);c.reply(receipt(),now);
    WORLD.lock().unwrap().as_mut().unwrap().wire.incoming=vec![22,3,3,0,4,255,0,0,0];
    for at in now+1..now+100{
        turn(c,at,false);
        if c.failure().is_some(){return at;}
        let pending={let guard=WORLD.lock().unwrap();let w=guard.as_ref().unwrap();w.synthetic.is_some()||w.machine.pending_reply().is_some()};
        if pending{c.reply(receipt(),at);}
    }
    panic!("actual malformed TLS task produced no retained failure");
}
#[test]fn actual_tls_failure_reason_survives_scrub_and_positive_cancel_release(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));assert_eq!(c.failure(),None);
    let now=actual_tls_failure(&mut c,0);let reason=c.failure().unwrap();assert_eq!(reason,sex_tls::Failure::Handshake);c.reject_prerequisites();assert_eq!(c.failure(),Some(reason));assert!(!c.start(ready(),c.next_sequence(),3));assert_eq!(c.failure(),Some(reason));assert!(c.private_scrubbed_for_test());assert!(c.busy());assert_ne!(c.view(),exchange::View::Verified);
    c.cancel();assert_eq!(c.failure(),Some(reason));assert!(c.private_scrubbed_for_test());
    c.reply(receipt(),now+1);let release_at=until_kind(&mut c,network_job::RELEASE,now+2);assert_eq!(c.failure(),Some(reason));assert!(c.busy());
    c.reply(receipt(),release_at+1);assert!(!c.busy());assert_eq!(c.view(),exchange::View::Cancelled);assert_eq!(c.failure(),Some(reason));assert!(c.private_scrubbed_for_test());
}
#[test]fn retained_tls_reason_resets_on_new_attempt_idle_cancel_prerequisite_failure_and_close(){
    let _serial=SERIAL.lock().unwrap();for reset_kind in 0..5{
        reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));let now=actual_tls_failure(&mut c,0);c.reply(receipt(),now+1);finish_cleanup(&mut c,now+2);
        assert!(c.failure().is_some());assert_eq!(c.view(),exchange::View::Unavailable);let seq=c.next_sequence();
        match reset_kind{
            0=>{assert!(c.start(ready(),seq,3));assert_eq!(c.failure(),None);c.cancel();},
            1=>c.cancel(),2=>c.reject_prerequisites(),3=>c.close(),_=>{assert!(!c.start(ready(),seq,0));},
        }
        assert_eq!(c.failure(),None);assert_eq!(c.next_sequence(),seq);assert_ne!(c.view(),exchange::View::Verified);
    }
}
#[test]fn transport_timeout_and_user_cancellation_never_invent_a_tls_failure_reason(){
    let _serial=SERIAL.lock().unwrap();for timeout in [false,true]{reset();let mut c=exchange::Client::new();assert!(c.start(ready(),1,3));turn(&mut c,0,false);
        if timeout{turn(&mut c,2000,false);assert_eq!(c.view(),exchange::View::Uncertain);}else{c.cancel();}
        assert_eq!(c.failure(),None);c.reply(receipt(),2001);finish_cleanup(&mut c,2002);assert_eq!(c.failure(),None);assert_ne!(c.view(),exchange::View::Verified);
    }
}

#[test]fn failed_released_attempt_cannot_supply_metadata_to_new_unadmitted_prerequisite_failure(){
    let _serial=SERIAL.lock().unwrap();reset();let mut c=exchange::Client::new();assert!(c.start(ready(),10,3));let now=actual_tls_failure(&mut c,0);c.reply(receipt(),now+1);finish_cleanup(&mut c,now+2);
    assert!(!c.busy());assert!(c.ticket()>0);assert!(c.stats().is_some());assert!(c.failure().is_some());assert!(c.tx_bytes()>0&&c.rx_bytes()>0);let seq=c.next_sequence();let calls=WORLD.lock().unwrap().as_ref().unwrap().calls.len();
    c.reject_prerequisites();assert_eq!(c.view(),exchange::View::Unavailable);assert_eq!(c.ticket(),0);assert_eq!(c.stats(),None);assert_eq!(c.failure(),None);assert_eq!((c.tx_bytes(),c.rx_bytes()),(0,0));assert!(!c.peer_authenticated());assert!(c.private_scrubbed_for_test());assert_eq!(c.next_sequence(),seq);assert!(!c.busy());assert_eq!(WORLD.lock().unwrap().as_ref().unwrap().calls.len(),calls);
}
