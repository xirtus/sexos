// SPDX-License-Identifier: GPL-3.0-only
//! The NHI control ring and the firmware connection manager against a model of
//! the controller: it reads what the driver queues on TX ring 0 (checking the
//! CRC), answers like Tiger Lake's ICM on RX ring 0, and injects plug events.

use std::collections::{HashMap, VecDeque};
use sex_thunderbolt::frame::{self, *};
use sex_thunderbolt::icm::{self, *};
use sex_thunderbolt::regs::{self, *};
use sex_thunderbolt::ring::{Layout, SLOTS};
use sex_thunderbolt::Nhi;

const BUS: u64 = 0x1_2340_0000;

#[derive(Default)]
struct Answers { driver_ready: bool, ready_info: u16, root_errors: u8, approve_error: bool, mute: bool }

struct Fake {
    regs: HashMap<u32, u32>,
    mem: Vec<u8>,
    tx_seen: u16,
    rx_next: u16,
    pending: VecDeque<(u8, Vec<u32>)>,
    sent: Vec<(u8, Vec<u32>)>,
    answers: Answers,
}

impl Fake {
    fn new() -> Self {
        let mut f = Fake { regs: HashMap::new(), mem: vec![0; 0x2000], tx_seen: 0, rx_next: 0, pending: VecDeque::new(), sent: vec![], answers: Answers::default() };
        f.regs.insert(regs::CAPS, 0x0040_000c);
        f.answers.driver_ready = true;
        f.answers.ready_info = 1 | 2 << 4 | 4 << 7;
        f
    }
    fn reg(&self, r: u32) -> u32 { *self.regs.get(&r).unwrap_or(&0) }
    fn offset(&self, base: u32) -> u64 { ((self.reg(base + 4) as u64) << 32 | self.reg(base) as u64) - BUS }
    fn word(&self, at: u64) -> u32 { let mut b = [0u8; 4]; self.load(at, &mut b); u32::from_le_bytes(b) }
    fn set_word(&mut self, at: u64, v: u32) { self.store(at, &v.to_le_bytes()); }

    /// The driver moved TX ring 0's producer: take every new frame.
    fn transmit(&mut self) {
        let prod = (self.reg(TX_RING_BASE + 8) >> 16) as u16;
        let base = self.offset(TX_RING_BASE);
        while self.tx_seen != prod {
            let d = base + self.tx_seen as u64 * 16;
            let mut lo = [0u8; 8];
            self.load(d, &mut lo);
            let buffer = u64::from_le_bytes(lo) - BUS;
            let status = self.word(d + 8);
            assert_eq!((status >> 20) & DESC_POSTED, DESC_POSTED);
            let len = (status & 0xfff) as usize;
            let pdf = ((status >> 12) & 0xf) as u8;
            assert_eq!(pdf, ((status >> 16) & 0xf) as u8, "SOF and EOF carry the type");
            let mut bytes = vec![0u8; len];
            self.load(buffer, &mut bytes);
            // Every frame the driver sends carries a good CRC, whatever its type.
            let n = len - 4;
            assert_eq!(crc32c(&bytes[..n]).to_be_bytes(), bytes[n..], "CRC of a sent frame");
            let words = frame::decode(pdf, &bytes).unwrap().words().to_vec();
            self.set_word(d + 8, status | DESC_COMPLETED << 20);
            self.tx_seen = (self.tx_seen + 1) % SLOTS;
            self.sent.push((pdf, words.clone()));
            self.answer(pdf, &words);
        }
    }

    fn answer(&mut self, pdf: u8, w: &[u32]) {
        if self.answers.mute { return; }
        match (pdf, w[0] as u8) {
            (PDF_ICM_COMMAND, icm::DRIVER_READY) if self.answers.driver_ready => {
                let info = self.answers.ready_info as u32;
                self.push(PDF_ICM_RESPONSE, vec![header(DRIVER_READY, 0, 0, 1), info << 16, 0x0025_0000, 0x9a1f]);
            }
            (PDF_ICM_COMMAND, icm::APPROVE_DEVICE) => {
                let flags = if self.answers.approve_error { icm::FLAGS_ERROR } else { 0 };
                let mut r = w.to_vec();
                r[0] = header(APPROVE_DEVICE, flags, 0, 1);
                self.push(PDF_ICM_RESPONSE, r);
            }
            (PDF_READ, _) => {
                if self.answers.root_errors > 0 {
                    self.answers.root_errors -= 1;
                    self.push(PDF_ERROR, vec![w[0] | 1 << 31, w[1], 0]);
                } else {
                    self.push(PDF_READ, vec![w[0] | 1 << 31, w[1], w[2], 0x1513_8086]);
                }
            }
            _ => {}
        }
    }

    fn push(&mut self, pdf: u8, words: Vec<u32>) { self.pending.push_back((pdf, words)); self.deliver(); }

    /// Fills posted RX slots (those before the driver's consumer index).
    fn deliver(&mut self) {
        if self.reg(RX_OPTIONS_BASE) & RING_FLAG_ENABLE == 0 { return; }
        let cons = self.reg(RX_RING_BASE + 8) as u16;
        let base = self.offset(RX_RING_BASE);
        assert_eq!(self.reg(RX_RING_BASE + 12), (FRAME_SIZE as u32) << 16 | SLOTS as u32);
        while self.rx_next != cons {
            let Some((pdf, words)) = self.pending.pop_front() else { return };
            let d = base + self.rx_next as u64 * 16;
            let mut lo = [0u8; 8];
            self.load(d, &mut lo);
            let buffer = u64::from_le_bytes(lo) - BUS;
            let mut bytes = [0u8; FRAME_SIZE];
            let len = frame::encode(&words, &mut bytes).unwrap();
            self.store(buffer, &bytes[..len]);
            self.set_word(d + 8, len as u32 | (pdf as u32) << 12 | (pdf as u32) << 16 | (DESC_COMPLETED | DESC_POSTED) << 20);
            self.rx_next = (self.rx_next + 1) % SLOTS;
        }
    }

    fn connected(&mut self, uuid: u8, route: u64, link_info: u16, slevel: u8) {
        let u = [uuid as u32, 0x1111, 0x2222, 0x3333];
        let flags = slevel << 3 | icm::FLAGS_DUAL_LANE | icm::FLAGS_SPEED_GEN3;
        let mut w = vec![header(EVENT_DEVICE_CONNECTED, flags, 0, 1), u[0], u[1], u[2], u[3], (route >> 32) as u32, route as u32, 7 | (link_info as u32) << 16];
        w.extend(std::iter::repeat_n(0, 55));
        self.push(PDF_ICM_EVENT, w);
    }
    fn disconnected(&mut self, route: u64) {
        self.push(PDF_ICM_EVENT, vec![header(EVENT_DEVICE_DISCONNECTED, 0, 0, 1), (route >> 32) as u32, route as u32]);
    }
}

impl Nhi for Fake {
    fn read(&mut self, r: u32) -> u32 { self.reg(r) }
    fn write(&mut self, r: u32, v: u32) {
        self.regs.insert(r, v);
        // A stopped ring (base cleared) carries nothing.
        if r == TX_RING_BASE + 8 && self.reg(TX_RING_BASE) != 0 { self.transmit(); }
        if r == RX_RING_BASE + 8 && self.reg(RX_RING_BASE) != 0 { self.deliver(); }
    }
    fn load(&self, at: u64, out: &mut [u8]) { out.copy_from_slice(&self.mem[at as usize..at as usize + out.len()]); }
    fn store(&mut self, at: u64, b: &[u8]) { self.mem[at as usize..at as usize + b.len()].copy_from_slice(b); }
}

fn run(m: &mut Manager, f: &mut Fake, now: u64) -> Vec<Event> {
    let mut out = vec![];
    m.poll(f, now, |e| out.push(e));
    out
}

/// A manager brought to Ready: firmware up, driver ready answered, root router read.
fn ready() -> (Manager, Fake) {
    let mut f = Fake::new();
    f.regs.insert(FW_STS, FW_STS_ICM_EN | FW_STS_NVM_AUTH_DONE);
    let mut m = Manager::new(Layout::control(BUS), 0);
    run(&mut m, &mut f, 0);
    run(&mut m, &mut f, 1000);
    assert_eq!(m.phase, Phase::Ready);
    (m, f)
}

#[test]
fn crc_and_frames() {
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    let mut out = [0u8; FRAME_SIZE];
    let n = frame::encode(&[0x0102_0304, 0xa0b0_c0d0], &mut out).unwrap();
    assert_eq!(n, 12);
    assert_eq!(&out[..8], &[1, 2, 3, 4, 0xa0, 0xb0, 0xc0, 0xd0], "words go big-endian");
    let w = frame::decode(PDF_READ, &out[..n]).unwrap();
    assert_eq!(w.words(), &[0x0102_0304, 0xa0b0_c0d0]);
    out[2] ^= 1;
    assert_eq!(frame::decode(PDF_READ, &out[..n]).err(), Some(frame::Error::Crc));
    assert!(frame::decode(PDF_ICM_EVENT, &out[..n]).is_ok(), "the firmware's own events are not CRC-checked");
    assert_eq!(frame::decode(PDF_READ, &out[..7]).err(), Some(frame::Error::Size));
    assert!(frame::encode(&[0; 64], &mut out).is_none(), "63 words at most");
    assert_eq!(frame::config_read(0x0301, SPACE_SWITCH, 2, 0x10, 1, 3), [0, 0x0301, 0x10 | 1 << 13 | 2 << 19 | 2 << 25 | 3 << 27]);
}

#[test]
fn routes_and_ids() {
    assert!(icm::below(0x0301, 0x01));
    assert!(icm::below(0x050301, 0x0301));
    assert!(!icm::below(0x0302, 0x0301));
    assert!(!icm::below(0x01, 0x01));
    assert!(icm::below(0x01, 0), "everything is below the host");
    assert_eq!(sex_thunderbolt::known(0x8086, 0x9a1f), Some("Tiger Lake-H NHI0"));
    assert_eq!(sex_thunderbolt::known(0x8086, 0x15eb), None, "discrete Titan Ridge is not an integrated ICM");
    assert_eq!(regs::config::force_power(0x0000_0001, true), 0x2200_0003);
    assert_eq!(regs::config::force_power(0x2200_0003, false), 0x2200_0001);
}

#[test]
fn waits_for_firmware_then_starts_rings_and_says_driver_ready() {
    let mut f = Fake::new();
    let mut m = Manager::new(Layout::control(BUS), 0);
    assert!(run(&mut m, &mut f, 100_000).is_empty());
    assert_eq!(m.phase, Phase::Firmware);
    f.regs.insert(FW_STS, FW_STS_NVM_AUTH_DONE);
    run(&mut m, &mut f, 200_000);
    assert_eq!(m.phase, Phase::Firmware, "ICM_EN too");
    f.answers.driver_ready = false;
    f.regs.insert(FW_STS, FW_STS_ICM_EN | FW_STS_NVM_AUTH_DONE);
    run(&mut m, &mut f, 300_000);
    assert_eq!(m.phase, Phase::DriverReady);
    assert_eq!(m.hops, 12);
    // Ring 0 both ways: the pool's bus address, sizes, raw + enabled, 9 RX slots posted.
    assert_eq!(f.reg(TX_RING_BASE), (BUS + m.layout.tx_descriptors) as u32);
    assert_eq!(f.reg(TX_RING_BASE + 4), (BUS >> 32) as u32);
    assert_eq!(f.reg(TX_RING_BASE + 12), SLOTS as u32);
    assert_eq!(f.reg(TX_OPTIONS_BASE), RING_FLAG_ENABLE | RING_FLAG_RAW);
    assert_eq!(f.reg(RX_RING_BASE), (BUS + m.layout.rx_descriptors) as u32);
    assert_eq!(f.reg(RX_OPTIONS_BASE + 4), 0xffff_ffff, "every frame type");
    assert_eq!(f.reg(RX_RING_BASE + 8), SLOTS as u32 - 1);
    assert_eq!(f.sent, vec![(PDF_ICM_COMMAND, vec![header(DRIVER_READY, 0, 0, 0)])]);
    assert!(m.layout.end <= 0x2000);
}

#[test]
fn driver_ready_then_root_router_then_ready() {
    let mut f = Fake::new();
    f.answers.root_errors = 2;
    f.regs.insert(FW_STS, FW_STS_ICM_EN | FW_STS_NVM_AUTH_DONE);
    let mut m = Manager::new(Layout::control(BUS), 0);
    // The model answers at once, so the answer is read in the same poll.
    let e = run(&mut m, &mut f, 0);
    let r = Ready { security: 1, proto_version: 2, boot_acl: 4, nvm_version: 0x0025_0000, device_id: 0x9a1f };
    assert_eq!(e, vec![Event::Ready(r)]);
    assert_eq!(r.nvm(), (0x25, 0));
    assert_eq!(m.phase, Phase::RootRouter);
    // Two errors, retried every 50 ms.
    run(&mut m, &mut f, 1000);
    assert_eq!(m.phase, Phase::RootRouter);
    run(&mut m, &mut f, 60_000);
    run(&mut m, &mut f, 60_100);
    assert_eq!(m.phase, Phase::RootRouter);
    run(&mut m, &mut f, 120_000);
    run(&mut m, &mut f, 120_100);
    assert_eq!(m.phase, Phase::Ready);
    let reads: Vec<_> = f.sent.iter().filter(|(p, _)| *p == PDF_READ).collect();
    assert_eq!(reads.len(), 3);
    assert_eq!(reads[0].1[2] & 0x7ffffff, 1 << 13 | 2 << 25, "one dword of the router's own config space");
}

#[test]
fn a_dock_is_approved_and_its_tunnel_comes_up() {
    let (mut m, mut f) = ready();
    // Security level "user": not yet authorized.
    f.connected(0xd0, 0x01, 1 | 1 << 4, 1);
    let e = run(&mut m, &mut f, 10_000);
    let Event::Connected(d) = e[0] else { panic!("{e:?}") };
    assert_eq!((d.route, d.connection_id, d.security, d.authorized, d.boot, d.link, d.depth, d.gbps(), d.dual_lane), (1, 7, Security::User, false, false, 1, 1, 20, true));
    assert!(d.security.needs_approval());
    assert!(!m.approve(0x99), "unknown device");
    assert!(m.approve(1));
    assert!(m.approve(1), "twice is once");
    let e = run(&mut m, &mut f, 20_000);
    assert!(e.is_empty());
    let (pdf, w) = f.sent.last().unwrap().clone();
    assert_eq!(pdf, PDF_ICM_COMMAND);
    assert_eq!(w, vec![header(APPROVE_DEVICE, 0, 0, 0), 0xd0, 0x1111, 0x2222, 0x3333, 0, 1, 7]);
    let e = run(&mut m, &mut f, 30_000);
    assert_eq!(e, vec![Event::Approved { route: 1, ok: true }]);
    assert!(m.device(1).unwrap().authorized);
    assert_eq!(f.sent.iter().filter(|(_, w)| w[0] as u8 == APPROVE_DEVICE).count(), 1);
}

#[test]
fn a_refused_approval_says_so() {
    let (mut m, mut f) = ready();
    f.answers.approve_error = true;
    f.connected(0xd0, 0x01, 1 | 1 << 4, 1);
    run(&mut m, &mut f, 10_000);
    m.approve(1);
    run(&mut m, &mut f, 20_000);
    assert_eq!(run(&mut m, &mut f, 30_000), vec![Event::Approved { route: 1, ok: false }]);
    assert!(!m.device(1).unwrap().authorized);
}

#[test]
fn unplugging_a_dock_drops_its_children_first() {
    let (mut m, mut f) = ready();
    // Already approved by the BIOS's boot list, and a monitor-hub behind it.
    f.connected(0xd0, 0x01, 1 | 1 << 4 | icm::LINK_INFO_APPROVED | icm::LINK_INFO_BOOT, 1);
    f.connected(0xd1, 0x0301, 1 | 2 << 4 | icm::LINK_INFO_APPROVED, 1);
    f.connected(0xd2, 0x03, 3 | 1 << 4, 0);
    let e = run(&mut m, &mut f, 10_000);
    assert_eq!(e.len(), 3);
    let Event::Connected(dock) = e[0] else { panic!() };
    assert!(dock.authorized && dock.boot);
    let Event::Connected(other) = e[2] else { panic!() };
    assert_eq!(other.security, Security::None);
    assert_eq!(m.count(), 3);
    // The same dock again (re-sent after DRIVER_READY): no duplicate.
    f.connected(0xd0, 0x01, 1 | 1 << 4 | icm::LINK_INFO_APPROVED, 1);
    run(&mut m, &mut f, 15_000);
    assert_eq!(m.count(), 3);
    f.disconnected(0x01);
    let e = run(&mut m, &mut f, 20_000);
    let routes: Vec<u64> = e.iter().map(|e| match e { Event::Disconnected(d) => d.route, _ => panic!("{e:?}") }).collect();
    assert_eq!(routes, vec![0x0301, 0x01]);
    assert_eq!(m.count(), 1);
    assert!(m.device(0x03).is_some());
}

#[test]
fn rejected_devices_and_vetoes_are_reported() {
    let (mut m, mut f) = ready();
    f.connected(0xd0, 0x0101_0101, icm::LINK_INFO_REJECTED, 1);
    f.push(PDF_ICM_EVENT, vec![header(EVENT_RTD3_VETO, 0, 0, 1), 2]);
    let e = run(&mut m, &mut f, 10_000);
    assert_eq!(e, vec![Event::Rejected { route: 0x0101_0101 }, Event::Veto(2)]);
    assert_eq!(m.count(), 0);
}

#[test]
fn the_receive_ring_wraps() {
    let (mut m, mut f) = ready();
    let mut seen = 0;
    for i in 0..40u32 {
        f.push(PDF_ICM_EVENT, vec![header(EVENT_RTD3_VETO, 0, 0, 1), i]);
        if i % 3 == 2 {
            for e in run(&mut m, &mut f, 10_000 + i as u64) { assert_eq!(e, Event::Veto(seen)); seen += 1; }
        }
    }
    for e in run(&mut m, &mut f, 20_000) { assert_eq!(e, Event::Veto(seen)); seen += 1; }
    assert_eq!(seen, 40);
    assert_eq!(m.bad_frames, 0);
}

#[test]
fn a_silent_firmware_is_asked_again_then_given_up_on() {
    let mut f = Fake::new();
    f.answers.mute = true;
    f.regs.insert(FW_STS, FW_STS_ICM_EN | FW_STS_NVM_AUTH_DONE);
    let mut m = Manager::new(Layout::control(BUS), 0);
    let mut now = 0;
    let mut events = vec![];
    for _ in 0..100 { events.extend(run(&mut m, &mut f, now)); now += icm::DRIVER_READY_TIMEOUT_US / 20; }
    assert_eq!(f.sent.len(), 1 + icm::RETRIES as usize);
    assert_eq!(events, vec![Event::Failed(Failure::NoAnswer)]);
    assert_eq!(m.phase, Phase::Failed(Failure::NoAnswer));
}

#[test]
fn no_firmware_or_no_controller() {
    let mut f = Fake::new();
    f.regs.insert(FW_STS, 0x4);
    let mut m = Manager::new(Layout::control(BUS), 0);
    assert!(run(&mut m, &mut f, icm::FIRMWARE_WAIT_US - 1).is_empty());
    assert_eq!(run(&mut m, &mut f, icm::FIRMWARE_WAIT_US), vec![Event::Failed(Failure::NoFirmware { status: 4 })]);
    assert!(f.sent.is_empty(), "rings stay off");
    let mut f = Fake::new();
    f.regs.insert(FW_STS, u32::MAX);
    let mut m = Manager::new(Layout::control(BUS), 0);
    assert_eq!(run(&mut m, &mut f, 0), vec![Event::Failed(Failure::Gone)]);
}

#[test]
fn sleep_unloads_then_a_new_manager_says_driver_ready_again() {
    use sex_thunderbolt::mailbox::{self, State};
    let (mut m, mut f) = ready();
    f.connected(1, 0x01, 0x0001, 1);
    run(&mut m, &mut f, 2000);
    assert_eq!(m.count(), 1);
    let sent = f.sent.len();
    m.unload(&mut f);
    assert_eq!(f.sent.len(), sent, "nothing more on the ring");
    assert_eq!(f.reg(TX_OPTIONS_BASE) & RING_FLAG_ENABLE, 0);
    assert_eq!(f.reg(RX_OPTIONS_BASE) & RING_FLAG_ENABLE, 0);
    assert_eq!(f.reg(INMAIL_CMD), INMAIL_OP_REQUEST | mailbox::DRV_UNLOADS as u32);
    assert_eq!(mailbox::state(&mut f), State::Busy);
    assert_eq!(m.count(), 0, "devices forgotten until the firmware reports them again");
    f.regs.insert(INMAIL_CMD, mailbox::DRV_UNLOADS as u32);
    assert_eq!(mailbox::state(&mut f), State::Done);
    f.regs.insert(INMAIL_CMD, INMAIL_ERROR);
    assert_eq!(mailbox::state(&mut f), State::Error);
    // Posting clears a stale error and the old command.
    mailbox::post(&mut f, 0x23, 5);
    assert_eq!((f.reg(INMAIL_DATA), f.reg(INMAIL_CMD)), (5, INMAIL_OP_REQUEST | 0x23));
    // Awake: the rings start from slot 0 and DRIVER_READY goes out again.
    f.tx_seen = 0;
    f.rx_next = 0;
    let mut m = Manager::new(Layout::control(BUS), 10_000);
    run(&mut m, &mut f, 10_000);
    run(&mut m, &mut f, 11_000);
    assert_eq!(m.phase, Phase::Ready);
    assert_eq!(f.sent[sent].1[0] as u8, icm::DRIVER_READY);
}
