// SPDX-License-Identifier: GPL-3.0-only
//! Private bounded protocol state. No authority, host I/O, mappings or Unix ABI.
#![no_std]
extern crate alloc;

#[cfg(test)] extern crate std;

use smoltcp::{iface::{Config as InterfaceConfig,Interface,SocketHandle,SocketSet,SocketStorage},
    phy::{self,Device,DeviceCapabilities,Medium},socket::{dhcpv4,tcp,udp},
    time::{Duration,Instant},wire::{EthernetAddress,IpAddress,IpCidr,Ipv4Address}};
pub use smoltcp::socket::tcp::State;
pub use smoltcp::iface::SocketStorage as Storage;
pub use smoltcp::socket::udp::PacketMetadata as DnsMetadata;
pub mod dns_wire;
pub const MAX_FRAME:usize=1514;
pub const FLOW_BYTES:usize=4096;
pub const TURN_BYTES:usize=256;
pub const NEIGHBOR_CAPACITY:usize=8;
pub const ROUTE_CAPACITY:usize=2;
const _:()=assert!(smoltcp::config::IFACE_NEIGHBOR_CACHE_COUNT==NEIGHBOR_CAPACITY);
const _:()=assert!(smoltcp::config::IFACE_MAX_ROUTE_COUNT==ROUTE_CAPACITY);
// Historical fixture aliases only. Runtime engines/classifiers use their
// validated copied BOOT configuration, never these baseline constants.
pub const LOCAL:[u8;4]=sex_net_config::Config::CONTROLLED.local;
pub const PORT:u16=sex_net_config::Config::CONTROLLED.listen_port;
pub const CLIENT_BASE:u16=sex_net_config::Config::CONTROLLED.source_base;
pub const CLIENT_REMOTE_PORT:u16=sex_net_config::Config::CONTROLLED.http_port;
pub const DNS_BYTES:usize=512;
pub const DNS_REMOTE_PORT:u16=sex_net_config::Config::CONTROLLED.dns_port;
const BASIS:u64=0xcbf29ce484222325;
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Error{Config,Mac,Clock,Busy,Length,Stopped,Protocol}
pub struct Frame{pub length:usize,pub bytes:[u8;MAX_FRAME]}
impl Frame{
 fn empty()->Self{Self{length:0,bytes:[0;MAX_FRAME]}}
 /// Trusted stack output classification, not caller-supplied authority. Unique
 /// finite source ports retain old job identity even after socket cancellation.
 pub fn client_ticket(&self,config:&sex_net_config::Config)->Option<u32>{
  if !config.valid(){return None;}
  let b=self.bytes.get(..self.length)?;
  if b.len()<54||b[12..14]!=[8,0]||b[14]>>4!=4||b[23]!=6||b[26..30]!=config.local{return None;}
  let header=(b[14]&15)as usize*4;
  if header<20||b.len()<14+header+20{return None;}
  let port=u16::from_be_bytes([b[14+header],b[15+header]]);
  if port>config.source_base{Some((port-config.source_base)as u32)}else{None}
 }
 /// UDP and TCP port namespaces are distinct. This trusted stack-output tag
 /// retains the old DNS job ticket after the UDP socket is aborted or rebound.
 pub fn dns_ticket(&self,config:&sex_net_config::Config)->Option<u32>{
  if !config.valid(){return None;}
  let b=self.bytes.get(..self.length)?;
  if b.len()<42||b[12..14]!=[8,0]||b[14]>>4!=4||b[23]!=17||b[26..30]!=config.local{return None;}
  let header=(b[14]&15)as usize*4;
  if header<20||b.len()<14+header+8{return None;}
  let port=u16::from_be_bytes([b[14+header],b[15+header]]);
  if port>config.source_base{Some((port-config.source_base)as u32)}else{None}
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
/// One private listener, one outbound TCP flow and one fixed DNS UDP socket. The stack's
/// handle is internal data, never an app-visible capability or file descriptor.
pub struct Engine<'a>{network:sex_net_config::Config,iface:Interface,flows:SocketSet<'a>,handle:SocketHandle,client_handle:SocketHandle,dns_handle:SocketHandle,adapter:Adapter,last:u64,
    stopped:bool,failed:Option<Error>,pending:[u8;TURN_BYTES],pending_length:usize,pending_offset:usize,
    active_flow:bool,stats:Statistics,open:Option<Open>}
/// The open-internet side: its own TCP flow, DNS socket and DHCP client, attached the
/// first time something asks for them, so nothing else ever sees their traffic.
struct Open{tcp:alloc::vec::Vec<SocketHandle>,dns:SocketHandle,dhcp:SocketHandle,lease:Option<Lease>,next_port:u16,query_id:u16,query_name:[u8;255],query_len:usize,resolver:[u8;4]}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Lease{pub address:[u8;4],pub prefix:u8,pub gateway:Option<[u8;4]>,pub dns:Option<[u8;4]>}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum OpenState{Opening,Open,PeerClosed,Closed}
pub const OPEN_PORT_BASE:u16=40000;
/// Each open-internet flow's receive window and send queue. Flows are made as they are
/// asked for (`open_add_flow`), so their number is bounded by memory, not a table.
pub const OPEN_RX_BYTES:usize=64*1024;
pub const OPEN_TX_BYTES:usize=16*1024;
pub struct OpenBuffers<'a>{pub dns_rx_meta:&'a mut[udp::PacketMetadata],pub dns_rx:&'a mut[u8],pub dns_tx_meta:&'a mut[udp::PacketMetadata],pub dns_tx:&'a mut[u8]}
impl<'a> Engine<'a>{
    /// `seed` is explicit input. A predictable development seed does not provide
    /// hostile-peer sequence protection, entropy or TLS security. The immutable
    /// checked BOOT gateway supplies exactly one private default route. No app
    /// can change the route table or obtain authority by naming a destination.
    pub fn new(network:sex_net_config::Config,mac:[u8;6],now:u64,seed:u64,rx:&'a mut[u8;FLOW_BYTES],tx:&'a mut[u8;FLOW_BYTES],client_rx:&'a mut[u8;FLOW_BYTES],client_tx:&'a mut[u8;FLOW_BYTES],dns_rxmeta:&'a mut[DnsMetadata;1],dns_rx:&'a mut[u8;DNS_BYTES],dns_txmeta:&'a mut[DnsMetadata;1],dns_tx:&'a mut[u8;DNS_BYTES])->Result<Self,Error>{
        // Configuration is copied data, not an authority source. Reject before
        // touching borrowed storage or constructing any interface/socket state.
        if !network.valid(){return Err(Error::Config);}
        if mac==[0;6]||mac[0]&1!=0{return Err(Error::Mac);}
        let time=Instant::from_millis(i64::try_from(now).map_err(|_|Error::Clock)?);
        rx.fill(0);tx.fill(0);client_rx.fill(0);client_tx.fill(0);dns_rx.fill(0);dns_tx.fill(0);
        let mut adapter=Adapter{rx:None,tx:None};let mut config=InterfaceConfig::new(EthernetAddress(mac).into());config.random_seed=seed;
        let mut iface=Interface::new(config,&mut adapter,time);
        iface.update_ip_addrs(|ips|{ips.push(IpCidr::new(IpAddress::v4(network.local[0],network.local[1],network.local[2],network.local[3]),network.prefix)).expect("ONE_STATIC_ADDRESS");});
        iface.routes_mut().add_default_ipv4_route(Ipv4Address::new(network.gateway[0],network.gateway[1],network.gateway[2],network.gateway[3])).map_err(|_|Error::Config)?;
        let mut connection=tcp::Socket::new(tcp::SocketBuffer::new(&mut rx[..]),tcp::SocketBuffer::new(&mut tx[..]));
        connection.set_timeout(Some(Duration::from_secs(5)));connection.set_ack_delay(None);connection.set_nagle_enabled(false);
        connection.listen((IpAddress::v4(network.local[0],network.local[1],network.local[2],network.local[3]),network.listen_port)).map_err(|_|Error::Protocol)?;
        let mut flows=SocketSet::new(alloc::vec::Vec::new());let handle=flows.add(connection);
        let mut client=tcp::Socket::new(tcp::SocketBuffer::new(&mut client_rx[..]),tcp::SocketBuffer::new(&mut client_tx[..]));
        client.set_timeout(Some(Duration::from_secs(5)));client.set_ack_delay(None);client.set_nagle_enabled(false);
        let client_handle=flows.add(client);
        let dns=udp::Socket::new(udp::PacketBuffer::new(&mut dns_rxmeta[..],&mut dns_rx[..]),
            udp::PacketBuffer::new(&mut dns_txmeta[..],&mut dns_tx[..]));
        let dns_handle=flows.add(dns);
        Ok(Self{network,iface,flows,handle,client_handle,dns_handle,open:None,adapter,last:now,stopped:false,failed:None,pending:[0;TURN_BYTES],pending_length:0,pending_offset:0,active_flow:false,stats:Statistics::new()})
    }
    pub fn state(&self)->State{self.flows.get::<tcp::Socket>(self.handle).state()}
    /// Start the open-internet side. Asks for an address on the local network.
    pub fn open_attach(&mut self,b:OpenBuffers<'a>)->Result<(),Error>{
        if self.open.is_some(){return Ok(());}
        let OpenBuffers{dns_rx_meta,dns_rx,dns_tx_meta,dns_tx}=b;
        let tcp=alloc::vec::Vec::new();
        let dns=udp::Socket::new(udp::PacketBuffer::new(dns_rx_meta,dns_rx),udp::PacketBuffer::new(dns_tx_meta,dns_tx));
        let dns=self.flows.add(dns);let dhcp=self.flows.add(dhcpv4::Socket::new());
        self.open=Some(Open{tcp,dns,dhcp,lease:None,next_port:OPEN_PORT_BASE,query_id:1,query_name:[0;255],query_len:0,resolver:[0;4]});
        Ok(())
    }
    pub fn open_lease(&self)->Option<Lease>{self.open.as_ref()?.lease}
    /// Open-internet flows made so far.
    pub fn open_flows(&self)->usize{self.open.as_ref().map_or(0,|o|o.tcp.len())}
    /// Makes one more open-internet flow, with its own window, and gives its number.
    pub fn open_add_flow(&mut self)->Result<usize,Error>{
        let open=self.open.as_mut().ok_or(Error::Config)?;
        // Out of memory is an answer (the caller waits for a flow), never a fall.
        let zeroed=|n:usize|->Result<alloc::vec::Vec<u8>,Error>{let mut v=alloc::vec::Vec::new();v.try_reserve_exact(n).map_err(|_|Error::Length)?;v.resize(n,0);Ok(v)};
        let (rx,tx)=(zeroed(OPEN_RX_BYTES)?,zeroed(OPEN_TX_BYTES)?);
        open.tcp.try_reserve(1).map_err(|_|Error::Length)?;
        let mut flow=tcp::Socket::new(tcp::SocketBuffer::new(rx),tcp::SocketBuffer::new(tx));
        flow.set_timeout(Some(Duration::from_secs(30)));flow.set_nagle_enabled(false);
        // Acknowledge at once: a delayed ACK holds the sender to one window per delay.
        flow.set_ack_delay(None);
        open.tcp.push(self.flows.add(flow));
        Ok(open.tcp.len()-1)
    }
    fn open_poll_dhcp(&mut self){
        let Some(open)=self.open.as_mut() else{return};
        let event=self.flows.get_mut::<dhcpv4::Socket>(open.dhcp).poll();
        match event{
            Some(dhcpv4::Event::Configured(config))=>{
                let address=config.address.address().octets();
                let lease=Lease{address,prefix:config.address.prefix_len(),gateway:config.router.map(|r|r.octets()),dns:config.dns_servers.first().map(|d|d.octets())};
                self.iface.update_ip_addrs(|ips|{ips.clear();let _=ips.push(IpCidr::Ipv4(config.address));});
                if let Some(router)=config.router{let _=self.iface.routes_mut().add_default_ipv4_route(router);}
                self.network.local=address;
                open.lease=Some(lease);
            }
            Some(dhcpv4::Event::Deconfigured)=>{open.lease=None;}
            None=>{}
        }
    }
    /// Ask the leased resolver about `host`. `open_resolved` gives the answer.
    pub fn open_resolve(&mut self,host:&[u8])->Result<(),Error>{
        let local=self.network.local;
        let open=self.open.as_mut().ok_or(Error::Config)?;
        let lease=open.lease.ok_or(Error::Config)?;
        let resolver=lease.dns.ok_or(Error::Config)?;
        open.query_len=dns_wire::encode_name(host,&mut open.query_name).ok_or(Error::Length)?;
        open.query_id=open.query_id.wrapping_add(1).max(1);open.resolver=resolver;
        let mut query=[0u8;DNS_BYTES];
        let length=dns_wire::query_name(open.query_id,&open.query_name[..open.query_len],&mut query).map_err(|_|Error::Protocol)?;
        let port=OPEN_PORT_BASE-1;
        let socket=self.flows.get_mut::<udp::Socket>(open.dns);
        socket.close();
        socket.bind((IpAddress::v4(local[0],local[1],local[2],local[3]),port)).map_err(|_|Error::Protocol)?;
        socket.send_slice(&query[..length],(IpAddress::v4(resolver[0],resolver[1],resolver[2],resolver[3]),53)).map_err(|_|Error::Protocol)
    }
    /// The next answer to the current question, if one arrived. Err for a final failure.
    pub fn open_resolved(&mut self)->Result<Option<[u8;4]>,Error>{
        let open=self.open.as_mut().ok_or(Error::Config)?;
        let socket=self.flows.get_mut::<udp::Socket>(open.dns);
        while socket.can_recv(){
            let (bytes,meta)=socket.recv().map_err(|_|Error::Protocol)?;
            if meta.endpoint.addr!=IpAddress::v4(open.resolver[0],open.resolver[1],open.resolver[2],open.resolver[3])||meta.endpoint.port!=53{continue;}
            match dns_wire::parse_name(open.query_id,&open.query_name[..open.query_len],bytes){
                Ok(Some(answer))=>return Ok(Some(answer.address)),
                Ok(None)=>continue,
                Err(_)=>return Err(Error::Protocol),
            }
        }
        Ok(None)
    }
    pub fn open_connect(&mut self,flow:usize,address:[u8;4],remote:u16)->Result<(),Error>{
        let local=self.network.local;
        let open=self.open.as_mut().ok_or(Error::Config)?;
        if open.lease.is_none(){return Err(Error::Config);}
        open.next_port=if open.next_port>=OPEN_PORT_BASE+7000{OPEN_PORT_BASE}else{open.next_port+1};
        let port=open.next_port;
        let flow=self.flows.get_mut::<tcp::Socket>(*open.tcp.get(flow).ok_or(Error::Length)?);
        flow.abort();
        flow.connect(self.iface.context(),(IpAddress::v4(address[0],address[1],address[2],address[3]),remote),
            (IpAddress::v4(local[0],local[1],local[2],local[3]),port)).map_err(|_|Error::Protocol)
    }
    pub fn open_state(&self,flow:usize)->OpenState{
        let Some(open)=self.open.as_ref() else{return OpenState::Closed};
        let Some(&handle)=open.tcp.get(flow) else{return OpenState::Closed};
        let flow=self.flows.get::<tcp::Socket>(handle);
        match flow.state(){
            State::SynSent|State::SynReceived=>OpenState::Opening,
            State::Established=>OpenState::Open,
            _ if flow.can_recv()=>OpenState::PeerClosed,
            State::CloseWait|State::FinWait1|State::FinWait2|State::Closing|State::LastAck|State::TimeWait=>OpenState::PeerClosed,
            _=>OpenState::Closed,
        }
    }
    pub fn open_send(&mut self,flow:usize,bytes:&[u8])->Result<usize,Error>{
        let open=self.open.as_ref().ok_or(Error::Config)?;
        let flow=self.flows.get_mut::<tcp::Socket>(*open.tcp.get(flow).ok_or(Error::Length)?);
        // A flow the peer or we closed takes nothing more: say so, rather than 0 forever.
        if !flow.may_send(){return Err(Error::Protocol);}
        if !flow.can_send(){return Ok(0);}
        flow.send_slice(bytes).map_err(|_|Error::Protocol)
    }
    pub fn open_receive(&mut self,flow:usize,out:&mut[u8])->Result<usize,Error>{
        let open=self.open.as_ref().ok_or(Error::Config)?;
        let flow=self.flows.get_mut::<tcp::Socket>(*open.tcp.get(flow).ok_or(Error::Length)?);
        if !flow.can_recv(){return Ok(0);}
        flow.recv_slice(out).map_err(|_|Error::Protocol)
    }
    pub fn open_close(&mut self,flow:usize){
        if let Some(&handle)=self.open.as_ref().and_then(|o|o.tcp.get(flow)){self.flows.get_mut::<tcp::Socket>(handle).abort();}
    }
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
        self.flows.get_mut::<udp::Socket>(self.dns_handle).close();
        self.adapter.rx=None;self.adapter.tx=None;self.pending.fill(0);self.pending_length=0;self.pending_offset=0;
    }
    /// One fixed BOOT endpoint, reached directly on-link or through the checked
    /// gateway. Ticket is private service data; no address/capability input.
    pub fn begin_client(&mut self,ticket:u32)->Result<(),Error>{
        self.begin_protocol_client(ticket,false)
    }
    /// Only the parent's exact live-owner duplex admission selects this
    /// immutable BOOT profile. This accepts no numeric endpoint selector.
    pub fn begin_client_tls(&mut self,ticket:u32)->Result<(),Error>{
        self.begin_protocol_client(ticket,true)
    }
    fn begin_protocol_client(&mut self,ticket:u32,tls:bool)->Result<(),Error>{
        if self.stopped{return Err(Error::Stopped);}
        let (address,remote_port)=if tls {self.network.tls_endpoint().ok_or(Error::Config)?}
            else {(self.network.http,self.network.http_port)};
        let port=u16::try_from(ticket).ok().and_then(|n|self.network.source_base.checked_add(n)).filter(|_|ticket!=0).ok_or(Error::Length)?;
        self.client_abort();
        self.flows.get_mut::<tcp::Socket>(self.client_handle).connect(self.iface.context(),
            (IpAddress::v4(address[0],address[1],address[2],address[3]),remote_port),
            (IpAddress::v4(self.network.local[0],self.network.local[1],self.network.local[2],self.network.local[3]),port)).map_err(|_|Error::Protocol)
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
    /// The stack may subsequently collect a RST carrying the old source port.
    /// The caller must retain the per-frame live-ticket fence before DMA; abort
    /// never relabels an old frame as shared listener/gateway traffic.
    pub fn client_abort(&mut self){
        self.flows.get_mut::<tcp::Socket>(self.client_handle).abort();
        if self.adapter.tx.as_ref().is_some_and(|f|f.client_ticket(&self.network).is_some()){self.adapter.tx=None;}
    }

    /// Emit one fixed-name DNS query, with no transport-level retry. The finite
    /// ticket selects both the DNS transaction ID and a never-wrapping UDP port;
    /// job admission/replay fencing remains the caller's private responsibility.
    pub fn begin_dns(&mut self,ticket:u32)->Result<(),Error>{
        if self.stopped{return Err(Error::Stopped);}
        if ticket==0||ticket>sex_net_config::MAX_TICKET{return Err(Error::Length);}
        let id=ticket as u16;
        let port=self.network.source_base.checked_add(id).ok_or(Error::Length)?;
        let mut query=[0u8;DNS_BYTES];
        let length=dns_wire::query(id,&mut query).map_err(|_|Error::Protocol)?;
        self.dns_abort();
        let socket=self.flows.get_mut::<udp::Socket>(self.dns_handle);
        socket.bind((IpAddress::v4(self.network.local[0],self.network.local[1],self.network.local[2],self.network.local[3]),port)).map_err(|_|Error::Protocol)?;
        if socket.send_slice(&query[..length],(IpAddress::v4(self.network.dns[0],self.network.dns[1],self.network.dns[2],self.network.dns[3]),self.network.dns_port)).is_err(){
            socket.close();return Err(Error::Protocol);
        }
        Ok(())
    }
    /// Consume at most one datagram (at most512 bytes). Source/local tuple
    /// mismatch is discarded without copying bytes into the caller's buffer.
    pub fn dns_receive(&mut self,out:&mut[u8;DNS_BYTES])->Result<Option<usize>,Error>{
        if self.stopped{return Err(Error::Stopped);}
        let socket=self.flows.get_mut::<udp::Socket>(self.dns_handle);
        if !socket.can_recv(){return Ok(None);}
        let(bytes,meta)=socket.recv().map_err(|_|Error::Protocol)?;
        if meta.endpoint.addr!=IpAddress::v4(self.network.dns[0],self.network.dns[1],self.network.dns[2],self.network.dns[3])||meta.endpoint.port!=self.network.dns_port
            ||meta.local_address!=Some(IpAddress::v4(self.network.local[0],self.network.local[1],self.network.local[2],self.network.local[3])){return Ok(None);}
        if bytes.len()>out.len(){return Err(Error::Length);}
        out[..bytes.len()].copy_from_slice(bytes);Ok(Some(bytes.len()))
    }
    /// Close resets socket packet metadata; borrowed stack backing stays private
    /// service storage. This does not claim that close scrubs the backing bytes.
    pub fn dns_abort(&mut self){
        self.flows.get_mut::<udp::Socket>(self.dns_handle).close();
        if self.adapter.tx.as_ref().is_some_and(|f|f.dns_ticket(&self.network).is_some()){self.adapter.tx=None;}
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
            if connection.listen((IpAddress::v4(self.network.local[0],self.network.local[1],self.network.local[2],self.network.local[3]),self.network.listen_port)).is_err(){self.stop(Error::Protocol);return;}
        }
        self.iface.poll_egress(time,&mut self.adapter,&mut self.flows);
        self.open_poll_dhcp();
    }
}

#[cfg(test)] mod tests;
