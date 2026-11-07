//! Serves the open-internet stream to one owner at a time. A client's bytes pass
//! through its own BOOT region, and each call is served in one turn.
use crate::jobs::Identity;
use sex_net::{Engine, OpenBuffers, OpenState};
use sex_pdx::memory_region::{QUERY_VIEW, READ, WRITE};
use sex_pdx::web as w;

static mut RX: [u8; 64 * 1024] = [0; 64 * 1024];
static mut TX: [u8; 16 * 1024] = [0; 16 * 1024];
static mut DNS_RX: [u8; 512] = [0; 512];
static mut DNS_TX: [u8; 512] = [0; 512];
static mut DNS_RX_META: [sex_net::DnsMetadata; 2] = [sex_net::DnsMetadata::EMPTY; 2];
static mut DNS_TX_META: [sex_net::DnsMetadata; 2] = [sex_net::DnsMetadata::EMPTY; 2];

pub struct Service { owner: Option<Identity>, attached: bool, announced: bool }
impl Service {
    pub const fn new() -> Self { Self { owner: None, attached: false, announced: false } }
    /// A program that ended holds nothing here.
    pub fn retire(&mut self, who: Identity, engine: Option<&mut Engine<'static>>) {
        if self.owner == Some(who) {
            self.owner = None;
            if let Some(e) = engine { e.open_close(); }
        }
    }
    pub fn handle(&mut self, who: Identity, a: u64, value: u64, engine: Option<&mut Engine<'static>>) -> u64 {
        let seq = (a & 0xFF_FFFF) as u32;
        let kind = (a >> 32) as u8;
        let Some(engine) = engine else { return w::reply(seq, w::OFFLINE, 0) };
        let Some(index) = sex_pdx::app_catalog::SPINDLE_TARGETS.iter().position(|t| *t == who.pd) else { return w::reply(seq, w::DENIED, 0) };
        let Ok(view) = sex_pdx::memory_region_view(w::PROVIDER_SLOT_BASE + index as u64, 0, w::REGION_BYTES as u64, QUERY_VIEW | READ | WRITE) else { return w::reply(seq, w::FAILED, 0) };
        let region = unsafe { core::slice::from_raw_parts_mut(view.virtual_address as *mut u8, w::REGION_BYTES) };
        if self.owner.is_some_and(|o| o != who) { return w::reply(seq, w::BUSY, 0); }
        self.owner = Some(who);
        if !self.attached {
            let b = unsafe { OpenBuffers {
                rx: &mut *core::ptr::addr_of_mut!(RX), tx: &mut *core::ptr::addr_of_mut!(TX),
                dns_rx_meta: &mut *core::ptr::addr_of_mut!(DNS_RX_META), dns_rx: &mut *core::ptr::addr_of_mut!(DNS_RX),
                dns_tx_meta: &mut *core::ptr::addr_of_mut!(DNS_TX_META), dns_tx: &mut *core::ptr::addr_of_mut!(DNS_TX) } };
            if engine.open_attach(b).is_err() { return w::reply(seq, w::FAILED, 0); }
            self.attached = true;
        }
        match kind {
            w::UP => match engine.open_lease() {
                Some(l) => {
                    if !self.announced {
                        self.announced = true;
                        let (g, d) = (l.gateway.unwrap_or([0; 4]), l.dns.unwrap_or([0; 4]));
                        sex_pdx::serial_record!("[net.dhcp.lease] address={}.{}.{}.{}/{} gateway={}.{}.{}.{} dns={}.{}.{}.{}", l.address[0], l.address[1], l.address[2], l.address[3], l.prefix, g[0], g[1], g[2], g[3], d[0], d[1], d[2], d[3]);
                    }
                    w::reply(seq, w::OK, 0)
                }
                None => w::reply(seq, w::WAIT, 0),
            },
            w::RESOLVE => {
                // value 0 asks for the next answer to the question already sent.
                if value != 0 {
                    let len = value as usize;
                    if len > 253 { return w::reply(seq, w::INVALID, 0); }
                    let mut name = [0u8; 253];
                    name[..len].copy_from_slice(&region[..len]);
                    return match engine.open_resolve(&name[..len]) { Ok(()) => w::reply(seq, w::WAIT, 0), Err(_) => w::reply(seq, w::NO_NAME, 0) };
                }
                match engine.open_resolved() {
                    Ok(Some(address)) => { region[..4].copy_from_slice(&address); w::reply(seq, w::OK, 0) }
                    Ok(None) => w::reply(seq, w::WAIT, 0),
                    Err(_) => w::reply(seq, w::NO_NAME, 0),
                }
            }
            w::CONNECT => {
                let address = ((value >> 16) as u32).to_be_bytes();
                match engine.open_connect(address, value as u16) { Ok(()) => w::reply(seq, w::OK, 0), Err(_) => w::reply(seq, w::FAILED, 0) }
            }
            w::STATE => {
                let state = match engine.open_state() {
                    OpenState::Opening => w::STATE_OPENING, OpenState::Open => w::STATE_OPEN,
                    OpenState::PeerClosed => w::STATE_PEER_CLOSED, OpenState::Closed => w::STATE_CLOSED,
                };
                w::reply(seq, w::OK, state)
            }
            w::SEND => {
                let len = (value as usize).min(w::REGION_BYTES);
                let mut own = [0u8; 1024];
                let take = len.min(own.len());
                own[..take].copy_from_slice(&region[..take]);
                match engine.open_send(&own[..take]) { Ok(n) => w::reply(seq, w::OK, n as u32), Err(_) => w::reply(seq, w::FAILED, 0) }
            }
            w::RECEIVE => {
                let len = (value as usize).min(w::REGION_BYTES);
                match engine.open_receive(&mut region[..len]) { Ok(n) => w::reply(seq, w::OK, n as u32), Err(_) => w::reply(seq, w::FAILED, 0) }
            }
            w::CLOSE => { engine.open_close(); self.owner = None; w::reply(seq, w::OK, 0) }
            _ => w::reply(seq, w::INVALID, 0),
        }
    }
}
