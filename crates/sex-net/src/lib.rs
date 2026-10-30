// SPDX-License-Identifier: GPL-3.0-only
//! Private bounded protocol state. No authority, host I/O, mappings or Unix ABI.
#![no_std]

use smoltcp::{iface::{Config,Interface,SocketHandle,SocketSet,SocketStorage},
    phy::{self,Device,DeviceCapabilities,Medium},socket::tcp,
    time::{Duration,Instant},wire::{EthernetAddress,IpAddress,IpCidr}};
pub use smoltcp::socket::tcp::State;
pub use smoltcp::iface::SocketStorage as Storage;
pub const MAX_FRAME:usize=1514;
pub const FLOW_BYTES:usize=4096;
pub const TURN_BYTES:usize=256;
pub const LOCAL:[u8;4]=[10,0,2,15];
pub const PORT:u16=46076;
pub const CLIENT_BASE:u16=48000;
pub const CLIENT_REMOTE_PORT:u16=46078;
const BASIS:u64=0xcbf29ce484222325;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Error{Mac,Clock,Busy,Length,Stopped,Protocol}
pub struct Frame{pub length:usize,pub bytes:[u8;MAX_FRAME]}
impl Frame{
 fn empty()->Self{Self{length:0,bytes:[0;MAX_FRAME]}}
 /// Trusted stack output classification, not caller-supplied authority. Unique
 /// finite source ports retain old job identity even after socket cancellation.
 pub fn client_ticket(&self)->Option<u32>{
  let b=&self.bytes[..self.length];
  if b.len()<54||b[12..14]!=[8,0]||b[14]>>4!=4||b[23]!=6||b[26..30]!=LOCAL{return None;}
  let header=(b[14]&15)as usize*4;
  if header<20||b.len()<14+header+20{return None;}
  let port=u16::from_be_bytes([b[14+header],b[15+header]]);
  if port>CLIENT_BASE{Some((port-CLIENT_BASE)as u32)}else{None}
 }
}
struct Adapter{rx:Option<Frame>,tx:Option<Frame>}
struct Receive(Frame);
struct Transmit<'a>(&'a mut Option<Frame>);
impl phy::RxToken for Receive{
    fn consume<R,F>(self,f:F)->R where F:FnOnce(&[u8])->R{f(&self.0.bytes[..self.0.length])}
}
impl phy::TxToken for Transmit<'_>{
    fn consume<R,F>(self,length:usize,f:F)->R where F:FnOnce(&mut[u8])->R{
        // Device MTU constrains trusted stack output, before any DMA exposure.
        assert!(length<=MAX_FRAME&&self.0.is_none(),"TRANSPORT_TX_CAPACITY");
        let mut frame=Frame::empty();frame.length=length;let result=f(&mut frame.bytes[..length]);*self.0=Some(frame);result
    }
}
impl Device for Adapter{
    type RxToken<'a>=Receive where Self:'a;
    type TxToken<'a>=Transmit<'a> where Self:'a;
    fn receive(&mut self,_:Instant)->Option<(Receive,Transmit<'_>)>{
        if self.tx.is_some(){return None;}
        Some((Receive(self.rx.take()?),Transmit(&mut self.tx)))
    }
    fn transmit(&mut self,_:Instant)->Option<Transmit<'_>>{if self.tx.is_some(){None}else{Some(Transmit(&mut self.tx))}}
    fn capabilities(&self)->DeviceCapabilities{let mut caps=DeviceCapabilities::default();caps.medium=Medium::Ethernet;caps.max_transmission_unit=MAX_FRAME;caps.max_burst_size=Some(1);caps}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Statistics{
    /// Bytes consumed into private echo custody, not delivered/committed receipts.
    pub received:u64,pub queued:u64,pub receive_hash:u64,pub queue_hash:u64,
    pub flow_received:u64,pub flow_queued:u64,pub flow_receive_hash:u64,pub flow_queue_hash:u64,
    pub flow:u64,pub packets:u64,
}
impl Statistics{fn new()->Self{Self{received:0,queued:0,receive_hash:BASIS,queue_hash:BASIS,flow_received:0,flow_queued:0,flow_receive_hash:BASIS,flow_queue_hash:BASIS,flow:0,packets:0}}}
fn hash(mut value:u64,bytes:&[u8])->u64{for b in bytes{value=(value^*b as u64).wrapping_mul(0x100000001b3);}value}
/// One private listener and one bounded outbound controlled-peer flow. The stack's
/// handle is internal data, never an app-visible capability or file descriptor.
pub struct Engine<'a>{iface:Interface,flows:SocketSet<'a>,handle:SocketHandle,client_handle:SocketHandle,adapter:Adapter,last:u64,
    stopped:bool,failed:Option<Error>,pending:[u8;TURN_BYTES],pending_length:usize,pending_offset:usize,
    active_flow:bool,stats:Statistics}
impl<'a> Engine<'a>{
    /// `seed` is explicit input. A predictable development seed does not provide
    /// hostile-peer sequence protection, entropy or TLS security.
    pub fn new(mac:[u8;6],now:u64,seed:u64,rx:&'a mut[u8;FLOW_BYTES],tx:&'a mut[u8;FLOW_BYTES],client_rx:&'a mut[u8;FLOW_BYTES],client_tx:&'a mut[u8;FLOW_BYTES],storage:&'a mut[SocketStorage<'a>;2])->Result<Self,Error>{
        if mac==[0;6]||mac[0]&1!=0{return Err(Error::Mac);}
        let time=Instant::from_millis(i64::try_from(now).map_err(|_|Error::Clock)?);
        rx.fill(0);tx.fill(0);client_rx.fill(0);client_tx.fill(0);
        let mut adapter=Adapter{rx:None,tx:None};let mut config=Config::new(EthernetAddress(mac).into());config.random_seed=seed;
        let mut iface=Interface::new(config,&mut adapter,time);
        iface.update_ip_addrs(|ips|{ips.push(IpCidr::new(IpAddress::v4(LOCAL[0],LOCAL[1],LOCAL[2],LOCAL[3]),24)).expect("ONE_STATIC_ADDRESS");});
        let mut connection=tcp::Socket::new(tcp::SocketBuffer::new(&mut rx[..]),tcp::SocketBuffer::new(&mut tx[..]));
        connection.set_timeout(Some(Duration::from_secs(5)));connection.set_ack_delay(None);connection.set_nagle_enabled(false);
        connection.listen((IpAddress::v4(LOCAL[0],LOCAL[1],LOCAL[2],LOCAL[3]),PORT)).map_err(|_|Error::Protocol)?;
        let mut flows=SocketSet::new(&mut storage[..]);let handle=flows.add(connection);
        let mut client=tcp::Socket::new(tcp::SocketBuffer::new(&mut client_rx[..]),tcp::SocketBuffer::new(&mut client_tx[..]));
        client.set_timeout(Some(Duration::from_secs(5)));client.set_ack_delay(None);client.set_nagle_enabled(false);
        let client_handle=flows.add(client);
        Ok(Self{iface,flows,handle,client_handle,adapter,last:now,stopped:false,failed:None,pending:[0;TURN_BYTES],pending_length:0,pending_offset:0,active_flow:false,stats:Statistics::new()})
    }
    pub fn state(&self)->State{self.flows.get::<tcp::Socket>(self.handle).state()}
    pub fn statistics(&self)->Statistics{self.stats}
    pub fn failure(&self)->Option<Error>{self.failed}
    pub fn can_receive(&self)->bool{!self.stopped&&self.adapter.rx.is_none()}
    pub fn submit_frame(&mut self,bytes:&[u8])->Result<(),Error>{
        if self.stopped{return Err(Error::Stopped);}if self.adapter.rx.is_some(){return Err(Error::Busy);}
        if !(14..=MAX_FRAME).contains(&bytes.len()){return Err(Error::Length);}
        let mut frame=Frame::empty();frame.length=bytes.len();frame.bytes[..bytes.len()].copy_from_slice(bytes);self.adapter.rx=Some(frame);Ok(())
    }
    pub fn take_frame(&mut self)->Option<Frame>{self.adapter.tx.take()}
    pub fn has_frame(&self)->bool{self.adapter.tx.is_some()}
    pub fn stop(&mut self,reason:Error){
        self.stopped=true;self.failed=Some(reason);self.flows.get_mut::<tcp::Socket>(self.handle).abort();self.flows.get_mut::<tcp::Socket>(self.client_handle).abort();
        self.adapter.rx=None;self.adapter.tx=None;self.pending.fill(0);self.pending_length=0;self.pending_offset=0;
    }
    /// One fixed controlled-peer endpoint, reachable on the existing static
    /// subnet. Ticket is private service data; no address/capability input.
    pub fn begin_client(&mut self,ticket:u32)->Result<(),Error>{
        if self.stopped{return Err(Error::Stopped);}
        let port=u16::try_from(ticket).ok().and_then(|n|CLIENT_BASE.checked_add(n)).filter(|_|ticket!=0).ok_or(Error::Length)?;
        self.client_abort();
        self.flows.get_mut::<tcp::Socket>(self.client_handle).connect(self.iface.context(),
            (IpAddress::v4(10,0,2,2),CLIENT_REMOTE_PORT),
            (IpAddress::v4(LOCAL[0],LOCAL[1],LOCAL[2],LOCAL[3]),port)).map_err(|_|Error::Protocol)
    }
    pub fn client_state(&self)->State{self.flows.get::<tcp::Socket>(self.client_handle).state()}
    pub fn client_queued(&self)->usize{self.flows.get::<tcp::Socket>(self.client_handle).send_queue()}
    pub fn client_send(&mut self,bytes:&[u8])->Result<usize,Error>{
        if self.stopped{return Err(Error::Stopped);}if bytes.len()>TURN_BYTES{return Err(Error::Length);}
        let flow=self.flows.get_mut::<tcp::Socket>(self.client_handle);if !flow.can_send(){return Ok(0);}
        flow.send_slice(bytes).map_err(|_|Error::Protocol)
    }
    pub fn client_receive(&mut self,out:&mut[u8])->Result<usize,Error>{
        if self.stopped{return Err(Error::Stopped);}if out.len()>TURN_BYTES{return Err(Error::Length);}
        let flow=self.flows.get_mut::<tcp::Socket>(self.client_handle);if !flow.can_recv(){return Ok(0);}
        flow.recv_slice(out).map_err(|_|Error::Protocol)
    }
    pub fn client_finish(&mut self){self.flows.get_mut::<tcp::Socket>(self.client_handle).close();}
    pub fn client_abort(&mut self){
        self.flows.get_mut::<tcp::Socket>(self.client_handle).abort();
        if self.adapter.tx.as_ref().is_some_and(|f|f.client_ticket().is_some()){self.adapter.tx=None;}
    }

    /// Processes at most one frame and one256-byte echo transfer per turn.
    /// Private single-frame backpressure bounds stack egress independently of
    /// retransmission timers. No DMA submission occurs in this library.
    pub fn advance(&mut self,now:u64){
        if self.stopped{return;}if now<self.last||now>i64::MAX as u64{self.stop(Error::Clock);return;}self.last=now;
        let time=Instant::from_millis(now as i64);self.iface.poll_maintenance(time);
        let had_rx=self.adapter.rx.is_some();self.iface.poll_ingress_single(time,&mut self.adapter,&mut self.flows);
        if had_rx&&self.adapter.rx.is_none(){self.stats.packets=self.stats.packets.saturating_add(1);}
        let connection=self.flows.get_mut::<tcp::Socket>(self.handle);
        if !self.active_flow&&matches!(connection.state(),State::Established|State::CloseWait){
            let Some(flow)=self.stats.flow.checked_add(1)else{self.stop(Error::Protocol);return;};
            self.stats.flow=flow;self.stats.flow_received=0;self.stats.flow_queued=0;self.stats.flow_receive_hash=BASIS;self.stats.flow_queue_hash=BASIS;self.active_flow=true;
        }
        if self.pending_length>self.pending_offset&&connection.can_send(){
            match connection.send_slice(&self.pending[self.pending_offset..self.pending_length]){
                Ok(n)=>{let bytes=&self.pending[self.pending_offset..self.pending_offset+n];self.stats.queued=self.stats.queued.saturating_add(n as u64);self.stats.flow_queued=self.stats.flow_queued.saturating_add(n as u64);self.stats.queue_hash=hash(self.stats.queue_hash,bytes);self.stats.flow_queue_hash=hash(self.stats.flow_queue_hash,bytes);self.pending_offset+=n;
                    if self.pending_offset==self.pending_length{self.pending.fill(0);self.pending_length=0;self.pending_offset=0;}},
                Err(_)=>{self.stop(Error::Protocol);return;},
            }
        }else if self.pending_length==0&&connection.can_recv(){
            match connection.recv_slice(&mut self.pending){
                Ok(n)=>{self.pending_length=n;self.stats.received=self.stats.received.saturating_add(n as u64);self.stats.flow_received=self.stats.flow_received.saturating_add(n as u64);self.stats.receive_hash=hash(self.stats.receive_hash,&self.pending[..n]);self.stats.flow_receive_hash=hash(self.stats.flow_receive_hash,&self.pending[..n]);},
                Err(_)=>{self.stop(Error::Protocol);return;},
            }
        }
        if connection.state()==State::CloseWait&&!connection.can_recv()&&self.pending_length==0&&connection.send_queue()==0{connection.close();}
        if connection.state()==State::Closed{
            self.pending.fill(0);self.pending_length=0;self.pending_offset=0;self.active_flow=false;
            if connection.listen((IpAddress::v4(LOCAL[0],LOCAL[1],LOCAL[2],LOCAL[3]),PORT)).is_err(){self.stop(Error::Protocol);return;}
        }
        self.iface.poll_egress(time,&mut self.adapter,&mut self.flows);
    }
}

#[cfg(test)] mod tests;
