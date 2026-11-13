// SPDX-License-Identifier: GPL-3.0-only
//! Private bounded protocol state. No authority, host I/O, mappings or Unix ABI.
#![no_std]
extern crate alloc;

#[cfg(test)] extern crate std;

use smoltcp::{iface::{Config as InterfaceConfig,Interface,SocketHandle,SocketSet,SocketStorage},
    phy::{self,Device,DeviceCapabilities,Medium},socket::{dhcpv4,tcp,udp},
    time::{Duration,Instant},wire::{EthernetAddress,HardwareAddress,IpAddress,IpCidr,IpListenEndpoint,Ipv4Address,Ipv6Address}};
pub use smoltcp::socket::tcp::State;
pub use smoltcp::iface::SocketStorage as Storage;
pub use smoltcp::socket::udp::PacketMetadata as DnsMetadata;
pub mod dns_wire;
pub mod v6config;
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
pub struct Engine<'a>{network:sex_net_config::Config,seed:u64,iface:Interface,flows:SocketSet<'a>,handle:SocketHandle,client_handle:SocketHandle,dns_handle:SocketHandle,adapter:Adapter,last:u64,
    stopped:bool,failed:Option<Error>,pending:[u8;TURN_BYTES],pending_length:usize,pending_offset:usize,
    active_flow:bool,stats:Statistics,open:Option<Open>}
/// The open-internet side: its own TCP flow, DNS socket and DHCP client, attached the
/// first time something asks for them, so nothing else ever sees their traffic.
struct Open{tcp:alloc::vec::Vec<SocketHandle>,udp:alloc::vec::Vec<SocketHandle>,dns:SocketHandle,dhcp:SocketHandle,lease:Option<Lease>,next_port:u16,next_udp_port:u16,query:Question,
    /// Each TCP flow's listening group (port and the group's own number), so several flows
    /// can wait on one port as its backlog.
    groups:alloc::vec::Vec<Option<(u16,usize)>>,six:Six,cache:alloc::vec::Vec<Cached>}
/// What the IPv6 network said about resolvers: RDNSS entries from router advertisements
/// with when each runs out, and stateless DHCPv6's answer.
struct Six{rdnss:[Option<([u8;16],u64)>;v6config::MAX_RESOLVERS],dhcp6:SocketHandle,ask:bool,xid:u32,started:u64,next_at:u64,timeout:u64,
    dhcp_resolvers:[Option<[u8;16]>;v6config::MAX_RESOLVERS],refresh_at:Option<u64>}
/// The name being looked up: its A question (`id`) and, when IPv6 can be used, its AAAA
/// question (`id+1`), and what came back for each.
struct Question{id:u16,name:[u8;255],len:usize,resolver:Address,six:bool,v4:Option<[u8;4]>,v6:Option<[u8;16]>,done4:bool,done6:bool,first_at:u64,
    /// The shortest TTL among the answers, and whether the answer is in the cache yet.
    ttl:u32,kept:bool}
/// One remembered lookup: its name as DNS labels, what each family gave, when it runs
/// out, and whether AAAA was asked (an IPv4-only answer can't stand for both).
struct Cached{name:[u8;255],len:usize,six:bool,v4:Option<[u8;4]>,v6:Option<[u8;16]>,until:u64}
/// Names remembered at once; the one running out soonest makes room.
pub const CACHE_ENTRIES:usize=512;
/// TTLs are kept between these, in seconds; a name with no address is kept the
/// negative time (RFC 2308 suggests the SOA minimum; most resolvers cap it near this).
pub const CACHE_MIN_TTL:u32=5;
pub const CACHE_MAX_TTL:u32=86400;
pub const CACHE_NEGATIVE_TTL:u32=30;
impl Six{
    fn start(&mut self,now:u64,seed:u64){
        self.ask=true;self.started=now;self.next_at=now;self.timeout=0;
        self.xid=((seed as u32)^(now as u32).wrapping_mul(0x9e37_79b9)^self.xid.wrapping_add(1))&0xff_ffff;
    }
    fn advert(&mut self,advert:v6config::Advert,now:u64){
        for (address,life) in advert.resolvers.iter().flatten(){
            let until=if *life==u32::MAX{u64::MAX}else{now.saturating_add(*life as u64*1000)};
            let entry=self.rdnss.iter().position(|e|e.is_some_and(|(a,_)|a==*address))
                .or_else(|| self.rdnss.iter().position(|e|e.is_none_or(|(_,u)|u<=now)));
            if let Some(i)=entry{self.rdnss[i]=if *life==0{None}else{Some((*address,until))};}
        }
        // M or O: ask DHCPv6 for the other configuration, once per answer.
        if (advert.other||advert.managed)&&!self.ask&&self.refresh_at.is_none(){self.start(now,0);}
    }
}
/// An address of either family. Data, never authority.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum Address{V4([u8;4]),V6([u8;16])}
impl Address{
    fn ip(self)->IpAddress{match self{Address::V4(a)=>IpAddress::v4(a[0],a[1],a[2],a[3]),Address::V6(a)=>IpAddress::Ipv6(Ipv6Address::from(a))}}
    fn of(ip:IpAddress)->Self{match ip{IpAddress::Ipv4(a)=>Address::V4(a.octets()),IpAddress::Ipv6(a)=>Address::V6(a.octets())}}
}
/// What a lookup in both families found.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Answers{pub v6:Option<[u8;16]>,pub v4:Option<[u8;4]>}
/// After the first family's answer, how long to wait for the other's (RFC 8305's
/// resolution delay).
pub const RESOLUTION_DELAY_MS:u64=50;
fn link_local(mac:[u8;6])->[u8;16]{[0xfe,0x80,0,0,0,0,0,0,mac[0]^2,mac[1],mac[2],0xff,0xfe,mac[3],mac[4],mac[5]]}
fn is_link_local(a:&[u8;16])->bool{a[0]==0xfe&&a[1]&0xc0==0x80}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub struct Lease{pub address:[u8;4],pub prefix:u8,pub gateway:Option<[u8;4]>,pub dns:Option<[u8;4]>}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]pub enum OpenState{Opening,Open,PeerClosed,Closed}
pub const OPEN_PORT_BASE:u16=40000;
/// Ports a program may bind or listen on: 1 to this, except 68 (the DHCP client's).
/// The ports above are the service's own: its resolver, its outbound flows and jobs.
pub const PROGRAM_PORT_MAX:u16=OPEN_PORT_BASE-2;
/// Ports a UDP socket bound to "any" gets, after the outbound TCP flows' and below the
/// job tickets'.
pub const UDP_PORT_BASE:u16=OPEN_PORT_BASE+7001;
pub const UDP_PORT_LAST:u16=OPEN_PORT_BASE+7999;
/// Each program UDP socket's queues: datagrams and bytes each way.
pub const UDP_PACKETS:usize=64;
pub const UDP_BYTES:usize=64*1024;
/// The largest datagram payload sent, so it fits one Ethernet frame unfragmented.
pub const UDP_PAYLOAD_V4:usize=1472;
pub const UDP_PAYLOAD_V6:usize=1452;
pub fn program_port(port:u16)->bool{(1..=PROGRAM_PORT_MAX).contains(&port)&&port!=68}
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
        Ok(Self{network,seed,iface,flows,handle,client_handle,dns_handle,open:None,adapter,last:now,stopped:false,failed:None,pending:[0;TURN_BYTES],pending_length:0,pending_offset:0,active_flow:false,stats:Statistics::new()})
    }
    pub fn state(&self)->State{self.flows.get::<tcp::Socket>(self.handle).state()}
    /// Start the open-internet side. Asks for an address on the local network.
    pub fn open_attach(&mut self,b:OpenBuffers<'a>)->Result<(),Error>{
        if self.open.is_some(){return Ok(());}
        let OpenBuffers{dns_rx_meta,dns_rx,dns_tx_meta,dns_tx}=b;
        let tcp=alloc::vec::Vec::new();
        let dns=udp::Socket::new(udp::PacketBuffer::new(dns_rx_meta,dns_rx),udp::PacketBuffer::new(dns_tx_meta,dns_tx));
        let dns=self.flows.add(dns);let dhcp=self.flows.add(dhcpv4::Socket::new());
        let meta=||alloc::vec![udp::PacketMetadata::EMPTY;2];
        let mut dhcp6=udp::Socket::new(udp::PacketBuffer::new(meta(),alloc::vec![0u8;1024]),udp::PacketBuffer::new(meta(),alloc::vec![0u8;256]));
        dhcp6.bind(IpListenEndpoint{addr:None,port:v6config::CLIENT_PORT}).map_err(|_|Error::Protocol)?;
        let dhcp6=self.flows.add(dhcp6);
        // IPv6 starts with the open side, so nothing else ever sees its traffic: the
        // interface is made again with a link-local address from the MAC and SLAAC on,
        // which asks the routers for a prefix and a default route.
        #[allow(irrefutable_let_patterns)]
        let HardwareAddress::Ethernet(mac)=self.iface.hardware_addr() else{return Err(Error::Mac)};
        let mut config=InterfaceConfig::new(mac.into());config.random_seed=self.seed;config.slaac=true;
        let addresses:alloc::vec::Vec<IpCidr>=self.iface.ip_addrs().to_vec();
        let mut iface=Interface::new(config,&mut self.adapter,Instant::from_millis(self.last as i64));
        iface.update_ip_addrs(|ips|{for a in addresses{let _=ips.push(a);}let _=ips.push(IpCidr::new(IpAddress::Ipv6(Ipv6Address::from(link_local(mac.0))),64));});
        let g=self.network.gateway;iface.routes_mut().add_default_ipv4_route(Ipv4Address::new(g[0],g[1],g[2],g[3])).map_err(|_|Error::Config)?;
        self.iface=iface;
        let query=Question{id:1,name:[0;255],len:0,resolver:Address::V4([0;4]),six:false,v4:None,v6:None,done4:true,done6:true,first_at:0,ttl:u32::MAX,kept:true};
        let six=Six{rdnss:[None;v6config::MAX_RESOLVERS],dhcp6,ask:false,xid:0,started:0,next_at:0,timeout:0,dhcp_resolvers:[None;v6config::MAX_RESOLVERS],refresh_at:None};
        self.open=Some(Open{tcp,udp:alloc::vec::Vec::new(),dns,dhcp,lease:None,next_port:OPEN_PORT_BASE,next_udp_port:UDP_PORT_BASE,query,groups:alloc::vec::Vec::new(),six,cache:alloc::vec::Vec::new()});
        Ok(())
    }
    /// The global IPv6 address SLAAC gave, if any.
    pub fn open_global6(&self)->Option<[u8;16]>{
        self.open.as_ref()?;
        self.iface.ip_addrs().iter().find_map(|c|match c{IpCidr::Ipv6(c)=>Some(c.address().octets()).filter(|a|!is_link_local(a)&&a[0]!=0xff&&*a!=[0;16]),_=>None})
    }
    /// The IPv6 default router, if one advertised itself.
    pub fn open_router6(&self)->Option<[u8;16]>{
        self.open.as_ref()?;
        match self.iface.routes().get_default_ipv6_route()?.via_router{IpAddress::Ipv6(v)=>Some(v.octets()),_=>None}
    }
    /// The open side can reach the internet: it has a DHCP lease or a global IPv6 address.
    pub fn open_ready(&self)->bool{self.open_lease().is_some()||self.open_global6().is_some()}
    /// The machine's addresses on the open side, with their prefix lengths: the leased
    /// IPv4 address and every IPv6 address.
    pub fn open_addresses(&self)->alloc::vec::Vec<(Address,u8)>{
        let leased=self.open_lease().is_some();
        self.iface.ip_addrs().iter().filter(|c|leased||matches!(c,IpCidr::Ipv6(_))).map(|c|(Address::of(c.address()),c.prefix_len())).collect()
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
        open.tcp.try_reserve(1).map_err(|_|Error::Length)?;open.groups.try_reserve(1).map_err(|_|Error::Length)?;
        let mut flow=tcp::Socket::new(tcp::SocketBuffer::new(rx),tcp::SocketBuffer::new(tx));
        flow.set_timeout(Some(Duration::from_secs(30)));flow.set_nagle_enabled(false);
        // Acknowledge at once: a delayed ACK holds the sender to one window per delay.
        flow.set_ack_delay(None);
        open.tcp.push(self.flows.add(flow));open.groups.push(None);
        Ok(open.tcp.len()-1)
    }
    fn open_poll_dhcp(&mut self){
        let Some(open)=self.open.as_mut() else{return};
        let event=self.flows.get_mut::<dhcpv4::Socket>(open.dhcp).poll();
        match event{
            Some(dhcpv4::Event::Configured(config))=>{
                let address=config.address.address().octets();
                let lease=Lease{address,prefix:config.address.prefix_len(),gateway:config.router.map(|r|r.octets()),dns:config.dns_servers.first().map(|d|d.octets())};
                // The lease replaces the IPv4 address; the IPv6 ones stay.
                self.iface.update_ip_addrs(|ips|{ips.retain(|c|!matches!(c,IpCidr::Ipv4(_)));let _=ips.push(IpCidr::Ipv4(config.address));});
                if let Some(router)=config.router{let _=self.iface.routes_mut().add_default_ipv4_route(router);}
                self.network.local=address;
                // Another network may answer names differently.
                if open.lease.is_some_and(|l|l!=lease){open.cache.clear();}
                open.lease=Some(lease);
            }
            Some(dhcpv4::Event::Deconfigured)=>{open.lease=None;open.cache.clear();}
            None=>{}
        }
    }
    /// Ask the leased resolver about `host`, IPv4 only. `open_resolved` gives the answer.
    pub fn open_resolve(&mut self,host:&[u8])->Result<(),Error>{self.open_ask(host,false)}
    /// Ask about `host` in both families: AAAA too when the machine has a global IPv6
    /// address to use one from. `open_resolved_all` gives the answers.
    pub fn open_resolve_all(&mut self,host:&[u8])->Result<(),Error>{let six=self.open_global6().is_some();self.open_ask(host,six)}
    /// The resolver lookups go to: the leased IPv4 one, else the first IPv6 one the
    /// router advertised (RDNSS) or DHCPv6 gave.
    pub fn open_resolver(&self)->Option<Address>{
        let open=self.open.as_ref()?;
        if let Some(d)=open.lease.and_then(|l|l.dns){return Some(Address::V4(d));}
        self.open_resolvers6().into_iter().flatten().next().map(Address::V6)
    }
    /// The IPv6 resolvers known now: RDNSS entries still in their lifetime, then DHCPv6's.
    pub fn open_resolvers6(&self)->[Option<[u8;16]>;v6config::MAX_RESOLVERS]{
        let mut out=[None;v6config::MAX_RESOLVERS];
        let Some(open)=self.open.as_ref() else{return out};
        let now=self.last;
        let found=open.six.rdnss.iter().flatten().filter(|(_,until)|*until>now).map(|(a,_)|*a).chain(open.six.dhcp_resolvers.iter().flatten().copied());
        let mut n=0;
        for a in found{if n<out.len()&&!out[..n].contains(&Some(a)){out[n]=Some(a);n+=1;}}
        out
    }
    /// Names in the cache now.
    pub fn open_cache_len(&self)->usize{self.open.as_ref().map_or(0,|o|o.cache.len())}
    /// Forgets every remembered lookup (the network changed).
    pub fn open_cache_flush(&mut self){if let Some(o)=self.open.as_mut(){o.cache.clear();}}
    fn open_ask(&mut self,host:&[u8],six:bool)->Result<(),Error>{
        let local=self.network.local;
        let now=self.last;
        let mut name=[0u8;255];
        let len=dns_wire::encode_name(host,&mut name).ok_or(Error::Length)?;
        {
            let open=self.open.as_mut().ok_or(Error::Config)?;
            open.cache.retain(|c|c.until>now);
            if let Some(c)=open.cache.iter().find(|c|c.len==len&&c.name[..len]==name[..len]&&(c.six||!six)){
                // Answered from memory: nothing goes on the wire.
                let q=&mut open.query;
                q.name=name;q.len=len;q.six=six;q.v4=c.v4;q.v6=if six{c.v6}else{None};q.done4=true;q.done6=true;q.first_at=now.max(1);q.kept=true;
                return Ok(());
            }
        }
        let resolver=self.open_resolver().ok_or(Error::Config)?;
        let open=self.open.as_mut().ok_or(Error::Config)?;
        let q=&mut open.query;
        q.name=name;q.len=len;q.ttl=u32::MAX;q.kept=false;
        // The A question takes an odd id and the AAAA question the next, neither zero.
        q.id=match q.id.wrapping_add(2){n if n>=0xFFFE=>1,n=>n|1};
        q.resolver=resolver;q.six=six;q.v4=None;q.v6=None;q.done4=false;q.done6=!six;q.first_at=0;
        let port=OPEN_PORT_BASE-1;
        let socket=self.flows.get_mut::<udp::Socket>(open.dns);
        socket.close();
        let bind=match resolver{Address::V4(_)=>IpListenEndpoint{addr:Some(IpAddress::v4(local[0],local[1],local[2],local[3])),port},Address::V6(_)=>IpListenEndpoint{addr:None,port}};
        socket.bind(bind).map_err(|_|Error::Protocol)?;
        let to=(resolver.ip(),53);
        for (id,kind) in [(q.id,dns_wire::A),(q.id+1,dns_wire::AAAA)]{
            if kind==dns_wire::AAAA&&!six{continue;}
            let mut query=[0u8;DNS_BYTES];
            let length=dns_wire::query_type(id,&q.name[..q.len],kind,&mut query).map_err(|_|Error::Protocol)?;
            socket.send_slice(&query[..length],to).map_err(|_|Error::Protocol)?;
        }
        Ok(())
    }
    /// Takes the replies that came for the current question.
    fn open_collect(&mut self){
        let now=self.last;
        let Some(open)=self.open.as_mut() else{return};
        let socket=self.flows.get_mut::<udp::Socket>(open.dns);
        let q=&mut open.query;
        let mut ttl=q.ttl;
        while let Ok((bytes,meta))=socket.recv(){
            if meta.endpoint.addr!=q.resolver.ip()||meta.endpoint.port!=53||bytes.len()<2{continue;}
            let id=u16::from_be_bytes([bytes[0],bytes[1]]);
            let kind=if id==q.id&&!q.done4{dns_wire::A}else if id==q.id+1&&q.six&&!q.done6{dns_wire::AAAA}else{continue};
            let found=match dns_wire::parse_type(id,&q.name[..q.len],kind,bytes){
                Ok(None)=>continue,
                Ok(Some(f))=>{ttl=ttl.min(f.ttl);Some(f.address)},
                // A family with no usable answer is done with none; the other may still have one.
                Err(_)=>None,
            };
            if kind==dns_wire::A{q.done4=true;q.v4=found.map(|a|[a[0],a[1],a[2],a[3]]);}else{q.done6=true;q.v6=found;}
            if found.is_some()&&q.first_at==0{q.first_at=now.max(1);}
        }
        q.ttl=ttl;
        if q.done4&&q.done6&&!q.kept{
            q.kept=true;
            let seconds=if q.v4.is_none()&&q.v6.is_none(){CACHE_NEGATIVE_TTL}else{q.ttl.clamp(CACHE_MIN_TTL,CACHE_MAX_TTL)};
            let entry=Cached{name:q.name,len:q.len,six:q.six,v4:q.v4,v6:q.v6,until:now.saturating_add(seconds as u64*1000)};
            let cache=&mut open.cache;
            cache.retain(|c|!(c.len==entry.len&&c.name[..c.len]==entry.name[..entry.len]));
            if cache.len()>=CACHE_ENTRIES{
                if let Some(i)=cache.iter().enumerate().min_by_key(|(_,c)|c.until).map(|(i,_)|i){cache.swap_remove(i);}
            }
            if cache.try_reserve(1).is_ok(){cache.push(entry);}
        }
    }
    /// The IPv4 answer to the current question, if it arrived. Err for a final failure.
    pub fn open_resolved(&mut self)->Result<Option<[u8;4]>,Error>{
        self.open_collect();
        let q=&self.open.as_ref().ok_or(Error::Config)?.query;
        match (q.v4,q.done4){(Some(a),_)=>Ok(Some(a)),(None,true)=>Err(Error::Protocol),_=>Ok(None)}
    }
    /// The answers to the current question in both families, once both came or one
    /// came and the other has had `RESOLUTION_DELAY_MS` more. Err when neither has any.
    pub fn open_resolved_all(&mut self)->Result<Option<Answers>,Error>{
        self.open_collect();
        let now=self.last;
        let q=&self.open.as_ref().ok_or(Error::Config)?.query;
        let answers=Answers{v6:q.v6,v4:q.v4};
        let any=q.v4.is_some()||q.v6.is_some();
        if q.done4&&q.done6{return if any{Ok(Some(answers))}else{Err(Error::Protocol)};}
        if any&&now.saturating_sub(q.first_at)>=RESOLUTION_DELAY_MS{return Ok(Some(answers));}
        Ok(None)
    }
    fn tcp_handle(&self,flow:usize)->Result<SocketHandle,Error>{self.open.as_ref().ok_or(Error::Config)?.tcp.get(flow).copied().ok_or(Error::Length)}
    /// Connects flow `flow` to `address`. IPv4 needs the lease; IPv6 takes its source
    /// address by RFC 6724 from the ones SLAAC gave.
    pub fn open_connect(&mut self,flow:usize,address:Address,remote:u16)->Result<(),Error>{
        let local=self.network.local;
        let six=self.open_global6().is_some();
        let open=self.open.as_mut().ok_or(Error::Config)?;
        match address{
            Address::V4(_) if open.lease.is_none()=>return Err(Error::Config),
            Address::V6(a) if !six&&!is_link_local(&a)=>return Err(Error::Config),
            _=>{}
        }
        open.next_port=if open.next_port>=OPEN_PORT_BASE+7000{OPEN_PORT_BASE}else{open.next_port+1};
        let port=open.next_port;
        let handle=*open.tcp.get(flow).ok_or(Error::Length)?;
        open.groups[flow]=None;
        let flow=self.flows.get_mut::<tcp::Socket>(handle);
        flow.abort();
        let local=match address{Address::V4(_)=>IpListenEndpoint{addr:Some(IpAddress::v4(local[0],local[1],local[2],local[3])),port},Address::V6(_)=>IpListenEndpoint{addr:None,port}};
        flow.connect(self.iface.context(),(address.ip(),remote),local).map_err(|_|Error::Protocol)
    }
    /// Makes flow `flow` listen on `port`, for either family. Another flow listening there
    /// already, or a port that is not a program's, is `Error::Busy`.
    pub fn open_listen(&mut self,flow:usize,port:u16)->Result<(),Error>{self.open_listen_in(flow,port,None)}
    /// Makes flow `flow` one more listener in group `group` on `port`: a backlog. Every
    /// flow of a group waits on the port at once, so that many peers can connect before
    /// the program accepts any. A listener on the port in another group is `Error::Busy`.
    pub fn open_listen_group(&mut self,flow:usize,port:u16,group:usize)->Result<(),Error>{self.open_listen_in(flow,port,Some(group))}
    fn open_listen_in(&mut self,flow:usize,port:u16,group:Option<usize>)->Result<(),Error>{
        if !program_port(port){return Err(Error::Busy);}
        let handle=self.tcp_handle(flow)?;
        let open=self.open.as_mut().ok_or(Error::Config)?;
        let flows=&self.flows;
        let other=open.tcp.iter().zip(&open.groups).any(|(h,g)|*h!=handle&&{let s=flows.get::<tcp::Socket>(*h);s.state()==State::Listen&&s.listen_endpoint().port==port}
            &&(group.is_none()||*g!=group.map(|n|(port,n))));
        if other{return Err(Error::Busy);}
        open.groups[flow]=group.map(|n|(port,n));
        let flow=self.flows.get_mut::<tcp::Socket>(handle);
        flow.abort();
        flow.listen(IpListenEndpoint{addr:None,port}).map_err(|_|Error::Protocol)
    }
    /// A flow of `group` a peer has connected to, waiting to be accepted. The caller takes
    /// it with `open_take`, and may put a fresh flow in the backlog in its place.
    pub fn open_accepted(&self,group:usize)->Option<usize>{
        let open=self.open.as_ref()?;
        open.groups.iter().enumerate().find(|(i,g)|g.is_some_and(|(_,n)|n==group)&&self.open_connected(*i)).map(|(i,_)|i)
    }
    /// The flows still listening in `group`.
    pub fn open_backlog(&self,group:usize)->usize{
        let Some(open)=self.open.as_ref() else{return 0};
        open.groups.iter().zip(&open.tcp).filter(|(g,h)|g.is_some_and(|(_,n)|n==group)&&matches!(self.flows.get::<tcp::Socket>(**h).state(),State::Listen|State::SynReceived)).count()
    }
    /// Takes flow `flow` out of its listening group, as an accepted connection.
    pub fn open_take(&mut self,flow:usize){if let Some(g)=self.open.as_mut().and_then(|o|o.groups.get_mut(flow)){*g=None;}}
    /// A listening flow a peer has connected to.
    pub fn open_connected(&self,flow:usize)->bool{
        self.tcp_handle(flow).is_ok_and(|h|!matches!(self.flows.get::<tcp::Socket>(h).state(),State::Listen|State::SynReceived|State::SynSent|State::Closed))
    }
    /// The far end of flow `flow`.
    pub fn open_peer(&self,flow:usize)->Option<(Address,u16)>{
        let e=self.flows.get::<tcp::Socket>(self.tcp_handle(flow).ok()?).remote_endpoint()?;
        Some((Address::of(e.addr),e.port))
    }
    /// A flow nothing is using in the stack, so a new caller can take it without cutting
    /// off a connection still finishing.
    pub fn open_idle(&self,flow:usize)->bool{self.tcp_handle(flow).is_ok_and(|h|self.flows.get::<tcp::Socket>(h).state()==State::Closed)}
    pub fn open_state(&self,flow:usize)->OpenState{
        let Ok(handle)=self.tcp_handle(flow) else{return OpenState::Closed};
        let flow=self.flows.get::<tcp::Socket>(handle);
        match flow.state(){
            State::SynSent|State::SynReceived|State::Listen=>OpenState::Opening,
            // After our own FIN the peer may still send.
            State::Established|State::FinWait1|State::FinWait2=>OpenState::Open,
            _ if flow.can_recv()=>OpenState::PeerClosed,
            State::CloseWait|State::Closing|State::LastAck|State::TimeWait=>OpenState::PeerClosed,
            _=>OpenState::Closed,
        }
    }
    pub fn open_send(&mut self,flow:usize,bytes:&[u8])->Result<usize,Error>{
        let flow=self.flows.get_mut::<tcp::Socket>(self.tcp_handle(flow)?);
        // A flow the peer or we closed takes nothing more: say so, rather than 0 forever.
        if !flow.may_send(){return Err(Error::Protocol);}
        if !flow.can_send(){return Ok(0);}
        flow.send_slice(bytes).map_err(|_|Error::Protocol)
    }
    pub fn open_receive(&mut self,flow:usize,out:&mut[u8])->Result<usize,Error>{
        let flow=self.flows.get_mut::<tcp::Socket>(self.tcp_handle(flow)?);
        if !flow.can_recv(){return Ok(0);}
        flow.recv_slice(out).map_err(|_|Error::Protocol)
    }
    /// Sends the flow's FIN after what is queued.
    pub fn open_shutdown(&mut self,flow:usize){
        if let Ok(h)=self.tcp_handle(flow){self.flows.get_mut::<tcp::Socket>(h).close();}
    }
    /// Lets the flow go: reset, unless it was shut down and is finishing on its own.
    pub fn open_close(&mut self,flow:usize){
        self.open_take(flow);
        if let Ok(h)=self.tcp_handle(flow){
            let s=self.flows.get_mut::<tcp::Socket>(h);
            if !matches!(s.state(),State::FinWait1|State::FinWait2|State::Closing|State::TimeWait|State::LastAck){s.abort();}
        }
    }
    /// Makes one more program UDP socket and gives its number.
    pub fn open_add_udp(&mut self)->Result<usize,Error>{
        let open=self.open.as_mut().ok_or(Error::Config)?;
        let zeroed=|n:usize|->Result<alloc::vec::Vec<u8>,Error>{let mut v=alloc::vec::Vec::new();v.try_reserve_exact(n).map_err(|_|Error::Length)?;v.resize(n,0);Ok(v)};
        let meta=||->Result<alloc::vec::Vec<udp::PacketMetadata>,Error>{let mut v=alloc::vec::Vec::new();v.try_reserve_exact(UDP_PACKETS).map_err(|_|Error::Length)?;v.resize(UDP_PACKETS,udp::PacketMetadata::EMPTY);Ok(v)};
        let socket=udp::Socket::new(udp::PacketBuffer::new(meta()?,zeroed(UDP_BYTES)?),udp::PacketBuffer::new(meta()?,zeroed(UDP_BYTES)?));
        open.udp.try_reserve(1).map_err(|_|Error::Length)?;
        open.udp.push(self.flows.add(socket));
        Ok(open.udp.len()-1)
    }
    fn udp_handle(&self,i:usize)->Result<SocketHandle,Error>{self.open.as_ref().ok_or(Error::Config)?.udp.get(i).copied().ok_or(Error::Length)}
    /// Binds UDP socket `i` to `port`, or to a free one of the service's when 0, for
    /// either family. Gives the port. A port in use or not a program's is `Error::Busy`.
    pub fn open_udp_bind(&mut self,i:usize,port:u16)->Result<u16,Error>{
        let handle=self.udp_handle(i)?;
        let open=self.open.as_mut().ok_or(Error::Config)?;
        let flows=&self.flows;
        let udp=&open.udp;
        let taken=|p:u16|udp.iter().any(|h|*h!=handle&&{let s=flows.get::<udp::Socket>(*h);s.is_open()&&s.endpoint().port==p});
        let port=if port==0{
            let mut found=None;
            for _ in UDP_PORT_BASE..=UDP_PORT_LAST{
                let p=open.next_udp_port;open.next_udp_port=if p>=UDP_PORT_LAST{UDP_PORT_BASE}else{p+1};
                if !taken(p){found=Some(p);break;}
            }
            found.ok_or(Error::Busy)?
        }else if !program_port(port)||taken(port){return Err(Error::Busy)}else{port};
        let s=self.flows.get_mut::<udp::Socket>(handle);
        s.close();
        s.bind(IpListenEndpoint{addr:None,port}).map_err(|_|Error::Protocol)?;
        Ok(port)
    }
    /// Queues one datagram. False when the queue has no room for it yet.
    pub fn open_udp_send(&mut self,i:usize,to:Address,port:u16,payload:&[u8])->Result<bool,Error>{
        let most=match to{Address::V4(_)=>UDP_PAYLOAD_V4,Address::V6(_)=>UDP_PAYLOAD_V6};
        if payload.len()>most||port==0{return Err(Error::Length);}
        let s=self.flows.get_mut::<udp::Socket>(self.udp_handle(i)?);
        if !s.is_open(){return Err(Error::Config);}
        match s.send_slice(payload,(to.ip(),port)){
            Ok(())=>Ok(true),
            Err(udp::SendError::BufferFull)=>Ok(false),
            Err(_)=>Err(Error::Protocol),
        }
    }
    /// Takes one datagram that came, if any: its sender, port and length copied (cut at
    /// `out.len()`).
    pub fn open_udp_receive(&mut self,i:usize,out:&mut[u8])->Result<Option<(Address,u16,usize)>,Error>{
        let s=self.flows.get_mut::<udp::Socket>(self.udp_handle(i)?);
        let Ok((bytes,meta))=s.recv() else{return Ok(None)};
        let n=bytes.len().min(out.len());out[..n].copy_from_slice(&bytes[..n]);
        Ok(Some((Address::of(meta.endpoint.addr),meta.endpoint.port,n)))
    }
    pub fn open_udp_close(&mut self,i:usize){if let Ok(h)=self.udp_handle(i){self.flows.get_mut::<udp::Socket>(h).close();}}
    pub fn statistics(&self)->Statistics{self.stats}
    pub fn failure(&self)->Option<Error>{self.failed}
    pub fn can_receive(&self)->bool{!self.stopped&&self.adapter.rx.is_none()}
    pub fn submit_frame(&mut self,bytes:&[u8])->Result<(),Error>{
        if self.stopped{return Err(Error::Stopped);}if self.adapter.rx.is_some(){return Err(Error::Busy);}
        if !(14..=MAX_FRAME).contains(&bytes.len()){return Err(Error::Length);}
        if let Some(open)=self.open.as_mut(){if let Some(advert)=v6config::router_advert(bytes){open.six.advert(advert,self.last);}}
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
        let time=Instant::from_millis(now as i64);
        if self.open.is_some(){
            // smoltcp 0.14's SLAAC adds its IPv6 default route only to a table holding no
            // route of another family: the IPv4 one steps aside while SLAAC catches up.
            let v4=self.iface.routes_mut().remove_default_ipv4_route();
            self.iface.poll_maintenance(time);
            if let Some(IpAddress::Ipv4(g))=v4.map(|r|r.via_router){let _=self.iface.routes_mut().add_default_ipv4_route(g);}
        }else{self.iface.poll_maintenance(time);}
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
        self.open_poll_dhcp6();
        // Replies are taken each turn, so a stray datagram can't crowd out the answer.
        self.open_collect();
    }
    /// Stateless DHCPv6: once a router says other configuration comes from DHCPv6, ask
    /// for the resolvers with an Information-Request, retrying with backoff, and again
    /// when the answer's refresh time runs out.
    fn open_poll_dhcp6(&mut self){
        let now=self.last;
        #[allow(irrefutable_let_patterns)]
        let HardwareAddress::Ethernet(mac)=self.iface.hardware_addr() else{return};
        let Some(open)=self.open.as_mut() else{return};
        let six=&mut open.six;
        let socket=self.flows.get_mut::<udp::Socket>(six.dhcp6);
        while let Ok((bytes,_))=socket.recv(){
            if !six.ask{continue;}
            if let Some(info)=v6config::reply(six.xid,mac.0,bytes){
                six.dhcp_resolvers=info.resolvers;six.ask=false;
                six.refresh_at=Some(now.saturating_add(info.refresh_s as u64*1000));
            }
        }
        if !six.ask&&six.refresh_at.is_some_and(|at|now>=at){six.refresh_at=None;six.start(now,self.seed);}
        if six.ask&&now>=six.next_at{
            let message=v6config::information_request(six.xid,mac.0,now-six.started);
            let to=(IpAddress::Ipv6(Ipv6Address::from(v6config::ALL_SERVERS)),v6config::SERVER_PORT);
            if socket.send_slice(&message,to).is_ok(){
                let jitter=((self.seed^six.xid as u64^now)%201)as i64-100;
                six.timeout=v6config::next_timeout(six.timeout,jitter);six.next_at=now.saturating_add(six.timeout);
            }
        }
    }
}

#[cfg(test)] mod tests;
