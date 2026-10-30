// SPDX-License-Identifier: GPL-3.0-only
//! Private TLS protocol state, not a transport or authority provider.
//!
//! One finite task and two finite byte queues live in the caller's ELF-owned
//! arena. There is no socket, clock, entropy syscall, endpoint selector, or
//! global allocator of frames here. The caller transfers an already-released
//! seed, a correlated UTC snapshot, and a BOOT-attested fixed trust profile.
//! The controlled lab root proves no public Internet trust. A generated
//! ClientHello or local certificate fixture check proves no peer.
#![no_std]
#![feature(allocator_api)]

extern crate alloc;
use alloc::{alloc::{alloc_zeroed, dealloc}, rc::Rc};
use core::{alloc::Layout, cell::RefCell, future::{poll_fn, Future}, marker::PhantomData,
    pin::Pin, ptr::NonNull, task::{Context, Poll, Waker}};
use embedded_tls::{Aes128GcmSha256,
    CryptoProvider, MaxFragmentLength, TlsConfig, TlsConnection, TlsContext,
    TlsError, TlsVerifier};
use rand_core::SeedableRng;
mod verifier;
mod probe_fixture;
pub use verifier::controlled_self_check;

pub const RECORD_READ: usize = 4096;
pub const RECORD_WRITE: usize = 2048;
pub const QUEUE_BYTES: usize = 4096;
pub const MAX_TASK_BYTES: usize = 32768;
pub const EXPECTED_NAME: &str = sex_trust_config::EXPECTED_NAME;
/// Public finite application protocol for the controlled interoperability
/// endpoint. This is not a browser document or public Internet trust profile.
pub const EXCHANGE_REQUEST: &[u8] = b"SexOS guest encrypted request round93\n";
pub const EXCHANGE_RESPONSE: &[u8] = b"SexOS controlled encrypted reply round93\n";

fn erase(bytes: &mut [u8]) {
    for byte in bytes { unsafe { core::ptr::write_volatile(byte, 0); } }
}
struct Seed([u8;32]);
impl Drop for Seed { fn drop(&mut self) { erase(&mut self.0); } }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Preparing, HelloPending, PeerAuthenticated, ExchangeVerified, Rejected }
/// Public protocol classification only; contains no peer bytes or secrets.
/// A transport timeout without a decoder result is deliberately not a failure
/// class and cannot establish certificate/handshake rejection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure { Certificate, Signature, Handshake, Record, Application, Protocol }
impl Failure {
    fn from_error(error:TlsError)->Self {
        match error {
            TlsError::InvalidCertificate|TlsError::InvalidCertificateEntry|TlsError::InvalidCertificateRequest=>Self::Certificate,
            TlsError::InvalidSignature|TlsError::InvalidSignatureScheme=>Self::Signature,
            TlsError::MissingHandshake|TlsError::InvalidHandshake|TlsError::HandshakeAborted(..)|TlsError::AbortHandshake(..)
                |TlsError::InvalidCipherSuite|TlsError::InvalidSupportedVersions|TlsError::InvalidSessionIdLength
                |TlsError::InvalidExtensionsLength|TlsError::InvalidKeyShare=>Self::Handshake,
            TlsError::InvalidRecord|TlsError::UnknownContentType|TlsError::DecodeError|TlsError::ParseError(_)=>Self::Record,
            TlsError::InvalidApplicationData=>Self::Application,
            _=>Self::Protocol,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Memory, Clock, Policy, Capacity, Terminal }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub task_bytes: usize, pub private_bytes: usize, pub tx_bytes: usize,
    pub utc_snapshot: u32,
}

/// An erased, pinned future allocation whose exact backing is wiped after its
/// destructor and before release to the private byte allocator. Never moves a
/// polled future. Stack/register/cache copies are outside this erasure claim.
struct Task {
    pointer: NonNull<u8>, layout: Layout,
    poll: unsafe fn(*mut u8, &mut Context<'_>) -> Poll<Result<(), TlsError>>,
    destroy: unsafe fn(*mut u8),
    _local: PhantomData<Rc<()>>,
}
impl Task {
    fn new<F: Future<Output=Result<(), TlsError>> + 'static>(future: F) -> Result<Self, Error> {
        let layout = Layout::new::<F>();
        if layout.size() == 0 || layout.size() > MAX_TASK_BYTES { return Err(Error::Memory); }
        let pointer = NonNull::new(unsafe { alloc_zeroed(layout) }).ok_or(Error::Memory)?;
        unsafe { pointer.as_ptr().cast::<F>().write(future); }
        unsafe fn poll<F: Future<Output=Result<(), TlsError>>>(p: *mut u8, cx: &mut Context<'_>) -> Poll<Result<(), TlsError>> {
            Pin::new_unchecked(&mut *p.cast::<F>()).poll(cx)
        }
        unsafe fn destroy<F>(p: *mut u8) { core::ptr::drop_in_place(p.cast::<F>()); }
        Ok(Self { pointer, layout, poll: poll::<F>, destroy: destroy::<F>, _local: PhantomData })
    }
    fn poll(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), TlsError>> {
        unsafe { (self.poll)(self.pointer.as_ptr(), cx) }
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.pointer.as_ptr());
            erase(core::slice::from_raw_parts_mut(self.pointer.as_ptr(), self.layout.size()));
            dealloc(self.pointer.as_ptr(), self.layout);
        }
    }
}

struct Pipe {
    rx: [u8;QUEUE_BYTES], tx: [u8;QUEUE_BYTES], rx_len: usize, tx_len: usize,
    turn_used: bool, waiting: bool,
    authenticated: bool, verified: bool,
}
impl Pipe {
    fn new() -> Self { Self { rx:[0;QUEUE_BYTES], tx:[0;QUEUE_BYTES], rx_len:0, tx_len:0, turn_used:false, waiting:false, authenticated:false, verified:false } }
    fn scrub(&mut self) { erase(&mut self.rx); erase(&mut self.tx); self.rx_len=0; self.tx_len=0; self.waiting=false; self.authenticated=false; self.verified=false; }
}
impl Drop for Pipe { fn drop(&mut self) { self.scrub(); } }
struct Transport(Rc<RefCell<Pipe>>);
impl embedded_io::ErrorType for Transport { type Error = embedded_io::ErrorKind; }
impl embedded_io_async::Read for Transport {
    async fn read(&mut self, out: &mut [u8]) -> Result<usize, Self::Error> {
        poll_fn(|_| {
            if out.is_empty() { return Poll::Ready(Ok(0)); }
            let mut pipe=self.0.borrow_mut();
            if pipe.rx_len==0 { pipe.waiting=true; return Poll::Pending; }
            if pipe.turn_used { return Poll::Pending; }
            pipe.turn_used=true; pipe.waiting=false;
            let n=out.len().min(pipe.rx_len).min(512);
            out[..n].copy_from_slice(&pipe.rx[..n]);
            let old=pipe.rx_len; pipe.rx.copy_within(n..old,0); pipe.rx_len=old-n;
            erase(&mut pipe.rx[old-n..old]); Poll::Ready(Ok(n))
        }).await
    }
}
impl embedded_io_async::Write for Transport {
    async fn write(&mut self, input: &[u8]) -> Result<usize, Self::Error> {
        poll_fn(|_| {
            if input.is_empty() { return Poll::Ready(Ok(0)); }
            let mut pipe=self.0.borrow_mut();
            if pipe.turn_used || pipe.tx_len==QUEUE_BYTES { return Poll::Pending; }
            pipe.turn_used=true;
            let n=input.len().min(QUEUE_BYTES-pipe.tx_len).min(512); let at=pipe.tx_len;
            pipe.tx[at..at+n].copy_from_slice(&input[..n]); pipe.tx_len+=n;
            Poll::Ready(Ok(n))
        }).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> { Ok(()) }
}

// Always return a concrete verifier. Err here can make upstream skip checks.
struct Provider { rng:rand_chacha::ChaCha20Rng, verifier:verifier::Verifier }
impl CryptoProvider for Provider {
    type CipherSuite=Aes128GcmSha256;
    type Signature=[u8;0];
    fn rng(&mut self) -> impl rand_core::CryptoRngCore { &mut self.rng }
    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Aes128GcmSha256>,TlsError> { Ok(&mut self.verifier) }
}

pub struct Session { task:Option<Task>, pipe:Rc<RefCell<Pipe>>, state:State, stats:Stats, failure:Option<Failure> }
impl Session {
    /// Always erases the transferred caller seed, including rejected starts.
    /// Seconds are input data, not new time authority or an advancing clock.
    pub fn prepare(seed:&mut[u8;32], seconds:u32, profile:u32) -> Result<Self,Error> {
        Self::prepare_mode(seed,seconds,profile,false)
    }
    /// Real protocol mode still has no transport/endpoint authority. The
    /// caller must move bytes exclusively through an admitted held duplex job.
    pub fn exchange(seed:&mut[u8;32], seconds:u32, profile:u32) -> Result<Self,Error> {
        Self::prepare_mode(seed,seconds,profile,true)
    }
    fn prepare_mode(seed:&mut[u8;32], seconds:u32, profile:u32, exchange:bool) -> Result<Self,Error> {
        let private=Seed(*seed); erase(seed);
        if !(1_577_836_800..=4_102_444_799).contains(&seconds) { return Err(Error::Clock); }
        let verifier=verifier::Verifier::new(profile,seconds).map_err(|_|Error::Policy)?;
        let pipe=Rc::try_new(RefCell::new(Pipe::new())).map_err(|_|Error::Memory)?;
        let transport=Transport(pipe.clone());
        let shared=pipe.clone();
        let future=async move {
            let mut provider=Provider { rng:rand_chacha::ChaCha20Rng::from_seed(private.0), verifier };
            drop(private);
            let mut read=[0u8;RECORD_READ]; let mut write=[0u8;RECORD_WRITE];
            let config=TlsConfig::new().with_server_name(EXPECTED_NAME).with_max_fragment_length(MaxFragmentLength::Bits10);
            let mut tls=TlsConnection::<_,Aes128GcmSha256>::new(transport,&mut read,&mut write);
            tls.open(TlsContext::new(&config,&mut provider)).await?;
            provider.verifier.finished()?;
            // Local preparation can never publish peer authentication.
            if !exchange { return Err(TlsError::MissingHandshake); }
            shared.borrow_mut().authenticated=true;
            let mut written=0;
            while written<EXCHANGE_REQUEST.len() {
                let n=tls.write(&EXCHANGE_REQUEST[written..]).await?;
                if n==0 { return Err(TlsError::InvalidApplicationData); }
                written=written.checked_add(n).filter(|n|*n<=EXCHANGE_REQUEST.len()).ok_or(TlsError::InvalidApplicationData)?;
            }
            tls.flush().await?;
            let mut response=[0u8;64];let mut received=0;
            while received<EXCHANGE_RESPONSE.len() {
                let n=tls.read(&mut response[received..]).await?;
                if n==0 { return Err(TlsError::InvalidApplicationData); }
                received=received.checked_add(n).filter(|n|*n<=EXCHANGE_RESPONSE.len()).ok_or(TlsError::InvalidApplicationData)?;
                if response[..received]!=EXCHANGE_RESPONSE[..received] { return Err(TlsError::InvalidApplicationData); }
            }
            erase(&mut response);
            // The close_notify is encrypted while the connection is open.
            // Peer close-control tail is later drained by the byte-custody
            // client, not interpreted as authenticated application content.
            tls.close().await.map_err(|(_,error)|error)?;
            shared.borrow_mut().verified=true;
            Ok(())
        };
        let task=Task::new(future)?;
        let stats=Stats { task_bytes:task.layout.size(), private_bytes:task.layout.size()+core::mem::size_of::<Pipe>(), tx_bytes:0, utc_snapshot:seconds };
        Ok(Self { task:Some(task),pipe,state:State::Preparing,stats,failure:None })
    }
    /// One explicit poll, at most one512-byte transport copy. The caller must
    /// poll again on its own bounded turn; this driver has no hidden executor.
    pub fn poll(&mut self) -> State {
        let Some(task)=self.task.as_mut() else { return self.state; };
        self.pipe.borrow_mut().turn_used=false;
        match task.poll(&mut Context::from_waker(Waker::noop())) {
          Poll::Ready(Ok(())) if self.pipe.borrow().verified=>{
            // Future/key backing is destroyed and wiped; retain outgoing
            // ciphertext until the caller receives positive WRITE receipts.
            self.task=None;self.state=State::ExchangeVerified;
          },
          Poll::Ready(result)=>{
            self.failure=result.err().map(Failure::from_error);
            self.task=None;self.pipe.borrow_mut().scrub();self.state=State::Rejected;
          },
          Poll::Pending=>{
            let pipe=self.pipe.borrow();
            if pipe.authenticated {self.state=State::PeerAuthenticated;}
            else if pipe.waiting && pipe.tx_len>0 { self.state=State::HelloPending; }
          },
        }
        self.stats.tx_bytes=self.pipe.borrow().tx_len; self.state
    }
    pub fn state(&self) -> State { self.state }
    pub fn failure(&self) -> Option<Failure> { self.failure }
    pub fn stats(&self) -> Stats { self.stats }
    pub fn outgoing_len(&self)->usize {self.pipe.borrow().tx_len}
    pub fn incoming_capacity(&self)->usize {if self.task.is_some(){QUEUE_BYTES-self.pipe.borrow().rx_len}else{0}}
    /// Protocol bytes remain data; draining them grants no endpoint authority.
    pub fn take_outgoing(&mut self, out:&mut[u8]) -> usize {
        let mut pipe=self.pipe.borrow_mut(); let n=out.len().min(pipe.tx_len); let old=pipe.tx_len;
        out[..n].copy_from_slice(&pipe.tx[..n]); pipe.tx.copy_within(n..old,0); pipe.tx_len=old-n;
        erase(&mut pipe.tx[old-n..old]); self.stats.tx_bytes=pipe.tx_len; n
    }
    /// All-or-nothing finite ingress. Terminal sessions cannot accept data.
    pub fn offer_incoming(&mut self, input:&[u8]) -> Result<(),Error> {
        if self.task.is_none() { return Err(Error::Terminal); }
        let mut pipe=self.pipe.borrow_mut(); let at=pipe.rx_len;
        if input.len()>QUEUE_BYTES-at { return Err(Error::Capacity); }
        pipe.rx[at..at+input.len()].copy_from_slice(input); pipe.rx_len+=input.len(); Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> Session {
        let mut seed=[19;32]; let mut s=Session::prepare(&mut seed,1_793_367_907,sex_trust_config::PROFILE).unwrap();
        assert_eq!(seed,[0;32]);
        for _ in 0..16 { if s.poll()==State::HelloPending { return s; } }
        panic!("library failed to yield with a private ClientHello");
    }
    #[test] fn actual_library_hello_is_finite_and_not_authenticated() {
        let mut s=prepared(); let stats=s.stats();
        assert!(stats.task_bytes<=MAX_TASK_BYTES && stats.private_bytes<49152);
        let mut wire=[0;QUEUE_BYTES]; let n=s.take_outgoing(&mut wire);
        assert!(n>100 && n<1024); assert_eq!(wire[0],22);
        assert_eq!(u16::from_be_bytes([wire[3],wire[4]]) as usize,n-5);
        assert_eq!(wire[5],1); // independently decoded TLS handshake ClientHello
        assert!(wire[..n].windows(EXPECTED_NAME.len()).any(|w|w==EXPECTED_NAME.as_bytes()));
        for _ in 0..32 { assert_eq!(s.poll(),State::HelloPending); }
        assert_eq!(s.take_outgoing(&mut wire),0); // no replay or automatic retry
        assert_eq!(s.failure(),None); // waiting is not a decoder rejection
    }
    #[test] fn invalid_time_always_consumes_and_erases_transferred_seed() {
        for now in [0,1_577_836_799,4_102_444_800,u32::MAX] {
            let mut seed=[43;32]; assert!(matches!(Session::prepare(&mut seed,now,sex_trust_config::PROFILE),Err(Error::Clock))); assert_eq!(seed,[0;32]);
        }
    }
    #[test] fn oversized_ingress_has_no_partial_effect_and_malformed_peer_is_terminal() {
        let mut s=prepared(); let mut wire=[0;QUEUE_BYTES];s.take_outgoing(&mut wire);
        assert_eq!(s.offer_incoming(&[7;QUEUE_BYTES+1]),Err(Error::Capacity));
        assert_eq!(s.pipe.borrow().rx_len,0);
        s.offer_incoming(&[22,3,3,0,4,255,0,0,0]).unwrap();
        for _ in 0..8 { s.poll(); }
        assert_eq!(s.state(),State::Rejected);assert_eq!(s.stats().tx_bytes,0);
        assert_eq!(s.failure(),Some(Failure::Handshake));
        for _ in 0..3 {assert_eq!(s.poll(),State::Rejected);assert_eq!(s.failure(),Some(Failure::Handshake));}
        assert_eq!(s.offer_incoming(&[0]),Err(Error::Terminal));
        assert!(s.pipe.borrow().rx.iter().chain(s.pipe.borrow().tx.iter()).all(|b|*b==0));
    }
    fn server_hello() -> alloc::vec::Vec<u8> {
        use p256::elliptic_curve::sec1::ToEncodedPoint;
        let point=p256::SecretKey::from_slice(&[1;32]).unwrap().public_key().to_encoded_point(false);
        let mut body=alloc::vec![3,3];body.extend_from_slice(&[7;32]);body.extend_from_slice(&[0,0x13,1,0]);
        let mut ext=alloc::vec![0,43,0,2,3,4,0,51,0,69,0,23,0,65];ext.extend_from_slice(point.as_bytes());
        body.extend_from_slice(&(ext.len()as u16).to_be_bytes());body.extend_from_slice(&ext);body
    }
    fn server_record(body:&[u8],claimed:usize)->alloc::vec::Vec<u8>{
        let mut handshake=alloc::vec![2,0,(claimed>>8)as u8,claimed as u8];handshake.extend_from_slice(body);
        let mut record=alloc::vec![22,3,3];record.extend_from_slice(&(handshake.len()as u16).to_be_bytes());record.extend_from_slice(&handshake);record
    }
    #[test] fn actual_owned_driver_rejects_unoffered_server_selections_before_authentication(){
        for (offset,value)in[(0,2),(1,4),(34,1),(36,2),(37,1),(45,3),(53,24)]{
            let mut s=prepared();let mut out=[0;QUEUE_BYTES];s.take_outgoing(&mut out);
            let mut body=server_hello();body[offset]=value;
            s.offer_incoming(&server_record(&body,body.len())).unwrap();
            for _ in 0..16{s.poll();}
            assert_eq!(s.state(),State::Rejected,"selection offset {offset}");
            assert_eq!(s.failure(),Some(Failure::Handshake));assert_eq!(s.outgoing_len(),0);
            assert!(s.task.is_none());assert!(s.pipe.borrow().rx.iter().chain(s.pipe.borrow().tx.iter()).all(|b|*b==0));
        }
    }
    #[test] fn actual_owned_driver_checks_server_handshake_extent_and_waits_without_false_auth(){
        let body=server_hello();
        let mut good=prepared();let mut out=[0;QUEUE_BYTES];good.take_outgoing(&mut out);
        good.offer_incoming(&server_record(&body,body.len())).unwrap();
        for _ in 0..16{good.poll();}
        assert!(!matches!(good.state(),State::Rejected|State::PeerAuthenticated|State::ExchangeVerified));assert_eq!(good.failure(),None);
        let mut tail=server_record(&body,body.len());tail.push(0xff);
        let length=(tail.len()-5)as u16;tail[3..5].copy_from_slice(&length.to_be_bytes());
        let mut s=prepared();s.take_outgoing(&mut out);s.offer_incoming(&tail).unwrap();
        for _ in 0..16{s.poll();}assert_eq!(s.state(),State::Rejected);assert_eq!(s.failure(),Some(Failure::Record));
        for claimed in [body.len()-1,body.len()+1]{let mut s=prepared();s.take_outgoing(&mut out);
            s.offer_incoming(&server_record(&body,claimed)).unwrap();for _ in 0..16{s.poll();}
            assert_eq!(s.state(),State::Rejected);assert!(s.failure().is_some());
        }
    }
    #[test] fn unknown_policy_consumes_seed_without_allocating_session() {
        for profile in [0,2,u32::MAX] {
            let mut seed=[43;32];
            assert!(matches!(Session::prepare(&mut seed,1_793_367_907,profile),Err(Error::Policy)));
            assert_eq!(seed,[0;32]);
        }
    }
}
