//! Driver-level behaviour: chip table, firmware files, DMA layout, the power-up
//! sequence against a register model, the 4-way handshake against an access
//! point played by the authenticator helpers, and frames end to end.
use sex_wifi::{ccmp, context, csr, devices, eapol, hcmd, ieee80211 as w, ucode};
use std::collections::HashMap;

const AP: [u8; 6] = [0x02, 0x11, 0x22, 0x33, 0x44, 0x55];
const STA: [u8; 6] = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];

#[test]
fn alienware_x17_killer_1675_gets_the_ty_firmware() {
    let chip = devices::identify(0x2725, 0x1674, 0x420, 0x0010d000).unwrap();
    assert_eq!(chip.firmware, "iwlwifi-ty-a0-gf-a0-77.ucode");
    assert_eq!(chip.pnvm, Some("iwlwifi-ty-a0-gf-a0.pnvm"));
    assert_eq!(chip.family, devices::Family::Ax210);
    assert!(chip.name.contains("1675x"));
    assert!(chip.six_ghz);
}

#[test]
fn chip_table_rules() {
    assert!(devices::claims(0x8086, 0x2723));
    assert!(!devices::claims(0x8086, 0x1234));
    assert!(!devices::claims(0x10ec, 0x2723));
    // AX200.
    assert_eq!(devices::identify(0x2723, 0, 0x340, 0).unwrap().firmware, "iwlwifi-cc-a0-77.ucode");
    // AX201 CNVi on Tiger Lake-H: QuZ MAC with an Hr radio.
    let ax201 = devices::identify(0x43f0, 0x0074, 0x354, 0x0010a000).unwrap();
    assert_eq!(ax201.firmware, "iwlwifi-QuZ-a0-hr-b0-77.ucode");
    assert!(ax201.integrated && ax201.low_latency_xtal);
    assert_eq!(devices::identify(0xa0f0, 0, 0x338, 0).unwrap().firmware, "iwlwifi-Qu-c0-hr-b0-77.ucode");
    assert_eq!(devices::identify(0xa0f0, 0, 0x334, 0).unwrap().firmware, "iwlwifi-Qu-b0-hr-b0-77.ucode");
    // 0x02f0 is QuZ only.
    assert!(devices::identify(0x02f0, 0, 0x338, 0).is_none());
    // Killer 1690i on an AX211 CNVi.
    assert_eq!(devices::identify(0x51f0, 0x1692, 0x370, 0x0010d000).unwrap().firmware, "iwlwifi-so-a0-gf4-a0-77.ucode");
    // The step bits move from 0-1 to 2-3.
    assert_eq!(devices::normalize_hw_rev(0x0000_0351), 0x0000_0354);
    for f in devices::FIRMWARE_FILES { assert!(f.starts_with("iwlwifi-")); }
}

/// A small firmware file with the same TLV structure as Intel's.
fn synthetic_firmware() -> Vec<u8> {
    let mut f = vec![0u8; 4];
    f.extend_from_slice(&ucode::MAGIC.to_le_bytes());
    let mut text = [0u8; 64];
    text[..12].copy_from_slice(b"77.test.sexs");
    f.extend_from_slice(&text);
    f.extend_from_slice(&(77u32 << 8 | 1 << 24).to_le_bytes());
    f.extend_from_slice(&[0u8; 12]);
    let mut tlv = |kind: u32, data: &[u8]| {
        f.extend_from_slice(&kind.to_le_bytes());
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(data);
        while f.len() % 4 != 0 { f.push(0); }
    };
    let section = |off: u32, len: usize, fill: u8| { let mut v = off.to_le_bytes().to_vec(); v.extend(std::iter::repeat(fill).take(len)); v };
    tlv(19, &section(0x0040_0000, 5000, 1));
    tlv(19, &section(0x0048_0000, 300, 2));
    tlv(19, &section(ucode::CPU1_CPU2_SEPARATOR, 0, 0));
    tlv(19, &section(0x0080_0000, 7000, 3));
    tlv(19, &section(ucode::PAGING_SEPARATOR, 0, 0));
    tlv(19, &section(0, 4096, 4));
    tlv(30, &[1, 0, 0, 0, 0x08, 0, 0, 0]); // capability bit 35
    tlv(31, &56u32.to_le_bytes());
    tlv(36, &[83, 0, 0, 0, 0x34, 0x12, 0, 0, 77, 0, 0, 0]);
    tlv(52, &[0xee; 1234]);
    tlv(48, &[0x0c, 0x01, 0x02, 0x00, 0x09, 0x06, 0x03, 0x00, 0xff]);
    f
}

#[test]
fn firmware_tlv_file_parses_into_lmac_umac_paging() {
    let file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&file).unwrap();
    assert_eq!(fw.human_readable, b"77.test.sexs");
    assert_eq!((fw.major, fw.minor, fw.api), (83, 0x1234, 77));
    assert_eq!(fw.scan_channels, 56);
    assert!(fw.has_capa(35) && !fw.has_capa(34));
    assert_eq!(fw.iml.unwrap().len(), 1234);
    assert_eq!(fw.command_version(1, 0x0c).unwrap().version, 2);
    assert_eq!(fw.command_version(6, 0x09).unwrap().version, 3);
    let image = fw.image();
    assert_eq!((image.lmac.len(), image.umac.len(), image.paging.len()), (2, 1, 1));
    assert_eq!(image.umac[0].data.len(), 7000);
    assert_eq!(image.total_bytes(), 5000 + 300 + 7000 + 4096);
    // Damaged files are refused, not misread.
    assert_eq!(ucode::Firmware::parse(&file[..60]).err(), Some(ucode::Error::Short));
    let mut bad = file.clone();
    bad[4] ^= 1;
    assert_eq!(ucode::Firmware::parse(&bad).err(), Some(ucode::Error::Magic));
    let truncated = &file[..file.len() - 10];
    assert_eq!(ucode::Firmware::parse(truncated).err(), Some(ucode::Error::Short));
}

/// The real linux-firmware file for the x17's Killer AX1675x, when fetched by
/// scripts/fetch_wifi_firmware.sh (CI fetches it; offline runs skip).
#[test]
fn real_ax210_firmware_when_present() {
    let Some(dir) = std::env::var_os("SEXOS_WIFI_FIRMWARE_DIR") else { return };
    let path = std::path::Path::new(&dir).join("iwlwifi-ty-a0-gf-a0-77.ucode");
    let Ok(file) = std::fs::read(&path) else { panic!("missing {}", path.display()) };
    let fw = ucode::Firmware::parse(&file).unwrap();
    assert_eq!(fw.api, 77);
    assert!(fw.iml.is_some(), "AX210 images carry an image loader");
    let image = fw.image();
    assert!(!image.lmac.is_empty() && !image.umac.is_empty() && !image.paging.is_empty());
    assert_eq!(image.lmac.len() + image.umac.len() + image.paging.len() + 2, fw.sections().len());
    let layout = context::Layout::plan(devices::Family::Ax210, &image, fw.iml.unwrap().len(), 8 << 20).unwrap();
    assert!(layout.end < 4 << 20, "firmware plus rings fit in 4 MiB: {}", layout.end);
}

struct Memory(Vec<u8>);
impl context::Dma for Memory {
    fn write(&mut self, offset: u64, bytes: &[u8]) { self.0[offset as usize..offset as usize + bytes.len()].copy_from_slice(bytes); }
    fn zero(&mut self, offset: u64, len: u64) { self.0[offset as usize..(offset + len) as usize].fill(0); }
}
fn u64_at(m: &[u8], at: u64) -> u64 { u64::from_le_bytes(m[at as usize..at as usize + 8].try_into().unwrap()) }

#[test]
fn context_info_tables_point_at_the_sections() {
    let file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&file).unwrap();
    let image = fw.image();
    const BASE: u64 = 0x1_2340_0000;
    for family in [devices::Family::Gen22000, devices::Family::Ax210] {
        let layout = context::Layout::plan(family, &image, 1234, 1 << 20).unwrap();
        assert!(context::Layout::plan(family, &image, 1234, 4096).is_err());
        let mut mem = Memory(vec![0xcc; layout.end as usize]);
        layout.write(&mut mem, BASE, &image, fw.iml.unwrap(), 0x420, false);
        let m = &mem.0;
        // Sections are copied to page-aligned places.
        assert_eq!(m[layout.lmac[0].offset as usize], 1);
        assert_eq!(m[layout.umac[0].offset as usize + 6999], 3);
        assert_eq!(m[layout.paging[0].offset as usize], 4);
        let dram = if family == devices::Family::Ax210 {
            let c = layout.context.offset;
            assert_eq!(u64_at(m, c + 8), BASE + layout.prph_info.offset);
            assert_eq!(u64_at(m, c + 52), BASE + layout.cmd_ring.offset);
            assert_eq!(u64_at(m, c + 60), BASE + layout.rx_used.offset);
            assert_eq!(u64_at(m, c + 88), BASE + layout.prph_scratch.offset);
            assert_eq!(m[layout.iml.offset as usize], 0xee);
            let s = layout.prph_scratch.offset;
            assert_eq!(u64_at(m, s + 48), BASE + layout.rx_free.offset);
            s + 124
        } else {
            let c = layout.context.offset;
            assert_eq!(u16::from_le_bytes([m[4], m[5]]), 448, "size in dwords");
            assert_eq!(u64_at(m, c + 24), BASE + layout.rx_free.offset);
            assert_eq!(u64_at(m, c + 48), BASE + layout.cmd_ring.offset);
            assert_eq!(m[c as usize + 56], 5);
            c + 192
        };
        // Table order: UMAC, LMAC, paging; unused entries stay zero.
        assert_eq!(u64_at(m, dram), BASE + layout.umac[0].offset);
        assert_eq!(u64_at(m, dram + 8), 0);
        assert_eq!(u64_at(m, dram + 512), BASE + layout.lmac[0].offset);
        assert_eq!(u64_at(m, dram + 520), BASE + layout.lmac[1].offset);
        assert_eq!(u64_at(m, dram + 1024), BASE + layout.paging[0].offset);
        for r in [layout.lmac[0], layout.umac[0], layout.paging[0], layout.cmd_ring, layout.rx_free] {
            assert_eq!(r.offset % 4096, 0);
        }
    }
}

/// Register model of the chip: bits the hardware sets on its own after a delay.
struct Chip { regs: HashMap<u32, u32>, prph: HashMap<u32, u32>, now: u64, clock_at: Option<u64>, writes: Vec<(u32, u32)>, prph_writes: Vec<(u32, u32)>, pending_prph: u32, alive_at: Option<u64> }
impl Chip {
    fn new() -> Self {
        let mut regs = HashMap::new();
        regs.insert(csr::GP_CNTRL, csr::GP_RFKILL);
        regs.insert(csr::HW_REV, 0x421);
        Chip { regs, prph: HashMap::new(), now: 0, clock_at: None, writes: vec![], prph_writes: vec![], pending_prph: 0, alive_at: None }
    }
}
impl csr::Registers for Chip {
    fn read(&mut self, o: u32) -> u32 {
        if o == csr::GP_CNTRL {
            if let Some(t) = self.clock_at { if self.now >= t { *self.regs.get_mut(&o).unwrap() |= csr::GP_MAC_CLOCK_READY; } }
        }
        if o == csr::INT { if let Some(t) = self.alive_at { if self.now >= t { return csr::INT_ALIVE; } } }
        if o == csr::PRPH_RDAT { return *self.prph.get(&(self.pending_prph & 0xffffff)).unwrap_or(&0); }
        *self.regs.get(&o).unwrap_or(&0)
    }
    fn write(&mut self, o: u32, v: u32) {
        self.writes.push((o, v));
        match o {
            csr::HW_IF_CONFIG => { self.regs.insert(o, v | if v & csr::IF_NIC_READY != 0 { csr::IF_NIC_READY } else { 0 }); }
            csr::GP_CNTRL => { if v & csr::GP_INIT_DONE != 0 && self.clock_at.is_none() { self.clock_at = Some(self.now + 3000); } self.regs.insert(o, v); }
            csr::INT | csr::FH_INT_STATUS => { let old = *self.regs.get(&o).unwrap_or(&0); self.regs.insert(o, old & !v); } // write 1 to clear
            csr::PRPH_WADDR | csr::PRPH_RADDR => self.pending_prph = v,
            csr::PRPH_WDAT => { self.prph.insert(self.pending_prph & 0xffffff, v); self.prph_writes.push((self.pending_prph & 0xffffff, v)); }
            _ => { self.regs.insert(o, v); }
        }
    }
}

#[test]
fn power_up_reaches_firmware_start_then_alive() {
    let file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&file).unwrap();
    let image = fw.image();
    let chip_info = devices::identify(0x2725, 0x1674, 0x420, 0).unwrap();
    let layout = context::Layout::plan(chip_info.family, &image, 1234, 1 << 20).unwrap();
    let mut hw = Chip::new();
    let base = 0x8000_0000u64;
    let mut boot = csr::Boot::new(chip_info, 0);
    let mut steps = vec![];
    while boot.step != csr::Step::WaitAlive {
        assert!({ let t = hw.now; boot.advance(&mut hw, t, &layout, base, 1234) }, "stopped at {:?}", boot.step);
        if steps.last() != Some(&boot.step) { steps.push(boot.step); }
        hw.now += 1000;
        assert!(hw.now < 1_000_000, "power-up stalled at {:?}", boot.step);
    }
    assert_eq!(steps, [csr::Step::Reset, csr::Step::Clock, csr::Step::Configure, csr::Step::NicLock, csr::Step::WaitAlive]);
    // Context info and image loader addresses, then the CPU kick.
    assert!(hw.writes.contains(&(csr::CTXT_INFO_ADDR, (base + layout.context.offset) as u32)));
    assert!(hw.writes.contains(&(csr::IML_SIZE_ADDR, 1234)));
    assert!(hw.prph_writes.contains(&(csr::UREG_CPU_INIT_RUN + 0x30_0000, 1)), "AX210 UMAC registers are offset");
    assert!(hw.writes.contains(&(csr::LTR_LONG_VAL_AD, csr::boot_ltr())), "discrete card gets the boot LTR");
    assert_eq!(boot.hw_rev, 0x421);
    // The firmware reports in: interrupt, then the ALIVE packet on the RX ring.
    hw.alive_at = Some(hw.now + 2000);
    for _ in 0..5 { let t = hw.now; boot.advance(&mut hw, t, &layout, base, 1234); hw.now += 1000; }
    assert_eq!(boot.step, csr::Step::WaitAlive);
    let mut buffer = vec![0u8; 4096];
    let len = 4 + 4 + 2 * 48 + 16;
    buffer[0..4].copy_from_slice(&(len as u32).to_le_bytes());
    buffer[4] = hcmd::ALIVE;
    buffer[7] = 0x80;
    buffer[8..10].copy_from_slice(&hcmd::ALIVE_STATUS_OK.to_le_bytes());
    buffer[12..16].copy_from_slice(&83u32.to_le_bytes());
    let packet = hcmd::packets(&buffer).next().unwrap();
    let alive = hcmd::parse_alive(&packet).unwrap();
    assert!(alive.ok && alive.lmac_major == 83);
    boot.alive_received(alive.ok);
    assert_eq!(boot.step, csr::Step::Alive);
}

#[test]
fn power_up_stops_on_radio_switch_and_on_silence() {
    let file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&file).unwrap();
    let image = fw.image();
    let info = devices::identify(0x2723, 0, 0x340, 0).unwrap();
    let layout = context::Layout::plan(info.family, &image, 0, 1 << 20).unwrap();
    // Radio killed by the hardware switch.
    let mut hw = Chip::new();
    hw.regs.insert(csr::GP_CNTRL, 0);
    let mut boot = csr::Boot::new(info, 0);
    while { let t = hw.now; boot.advance(&mut hw, t, &layout, 0, 0) } { hw.now += 1000; assert!(hw.now < 2_000_000); }
    assert_eq!(boot.step, csr::Step::Failed(csr::Failure::RfKill));
    // Firmware never says ALIVE: give up after a second.
    let mut hw = Chip::new();
    let mut boot = csr::Boot::new(info, 0);
    while { let t = hw.now; boot.advance(&mut hw, t, &layout, 0, 0) } { hw.now += 10_000; assert!(hw.now < 5_000_000); }
    assert_eq!(boot.step, csr::Step::Failed(csr::Failure::AliveTimeout));
    assert!(hw.writes.contains(&(csr::CTXT_INFO_BA, 0)), "22000 series uses the old context register");
    assert!(hw.prph_writes.is_empty() || !hw.prph_writes.iter().any(|w| w.0 == csr::UREG_CPU_INIT_RUN + 0x30_0000));
    // A device that fell off the bus reads all ones.
    let mut hw = Chip::new();
    hw.regs.insert(csr::HW_IF_CONFIG, u32::MAX);
    let mut boot = csr::Boot::new(info, 0);
    assert!(!boot.advance(&mut hw, 0, &layout, 0, 0));
    assert_eq!(boot.step, csr::Step::Failed(csr::Failure::Gone));
}

#[test]
fn host_command_framing() {
    let mut buf = [0u8; 64];
    let n = hcmd::encode(&mut buf, hcmd::LEGACY_GROUP, 0x88, 0, 0, 7, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]).unwrap();
    assert_eq!(n, 24);
    assert_eq!(&buf[..8], &[0x88, 1, 7, 0, 16, 0, 0, 0], "group 0 goes out as LONG_GROUP");
    let tfd = hcmd::tfd_for_command(0x1_0000_1000, n);
    assert_eq!(u16::from_le_bytes([tfd[0], tfd[1]]), 2);
    assert_eq!(u16::from_le_bytes([tfd[2], tfd[3]]), 20);
    assert_eq!(u64::from_le_bytes(tfd[4..12].try_into().unwrap()), 0x1_0000_1000);
    assert_eq!(u16::from_le_bytes([tfd[12], tfd[13]]), 4);
    assert_eq!(u64::from_le_bytes(tfd[14..22].try_into().unwrap()), 0x1_0000_1014);
    assert_eq!(hcmd::write_pointer(0, 5), 5);
    // Two packets in one RX buffer, 64-byte aligned, then the end marker.
    let mut rx = vec![0u8; 4096];
    rx[0..4].copy_from_slice(&12u32.to_le_bytes());
    rx[4] = 0x88; rx[5] = 1; rx[6] = 7; rx[7] = 0;
    rx[64..68].copy_from_slice(&8u32.to_le_bytes());
    rx[68] = 0xc5; rx[71] = 0x80;
    rx[128..132].copy_from_slice(&0x5555_0000u32.to_le_bytes());
    let p: Vec<_> = hcmd::packets(&rx).collect();
    assert_eq!(p.len(), 2);
    assert!(p[0].is_response() && p[0].index == 7 && p[0].data.len() == 8);
    assert!(!p[1].is_response());
}

fn beacon(ssid: &[u8], channel: u8, rsn: Option<&[u8]>, h2e: bool) -> Vec<u8> {
    let mut f = vec![0x80, 0x00, 0, 0];
    f.extend_from_slice(&w::BROADCAST);
    f.extend_from_slice(&AP);
    f.extend_from_slice(&AP);
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(&[0; 8]);
    f.extend_from_slice(&100u16.to_le_bytes());
    f.extend_from_slice(&(0x0001u16 | if rsn.is_some() { 0x10 } else { 0 }).to_le_bytes());
    f.push(0); f.push(ssid.len() as u8); f.extend_from_slice(ssid);
    f.extend_from_slice(&[1, 4, 0x82, 0x84, 0x8b, 0x96]);
    f.extend_from_slice(&[3, 1, channel]);
    if let Some(r) = rsn { f.extend_from_slice(r); }
    if h2e { f.extend_from_slice(&[244, 1, 0x20]); }
    f
}

#[test]
fn scan_results_name_security() {
    let mut rsn = [0u8; 32];
    let n = w::own_rsn(false, &mut rsn);
    let b = w::parse_bss(&beacon(b"home", 6, Some(&rsn[..n]), false)).unwrap();
    assert_eq!(b.ssid(), b"home");
    assert_eq!(b.channel, 6);
    assert_eq!(b.security, w::Security::Wpa2Personal);
    assert_eq!(b.rsn_element(), &rsn[..n]);
    let n3 = w::own_rsn(true, &mut rsn);
    let b = w::parse_bss(&beacon(b"wpa3", 36, Some(&rsn[..n3]), true)).unwrap();
    assert_eq!(b.security, w::Security::Wpa3Personal);
    assert!(b.rsn.mfpr && b.h2e);
    let transition = [48, 24, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 2, 0, 0, 0x0f, 0xac, 2, 0, 0x0f, 0xac, 8, 0x80, 0];
    assert_eq!(w::parse_bss(&beacon(b"mixed", 11, Some(&transition), false)).unwrap().security, w::Security::Wpa3Transition);
    assert_eq!(w::parse_bss(&beacon(b"cafe", 1, None, false)).unwrap().security, w::Security::Open);
    let tkip_only = [48, 20, 1, 0, 0, 0x0f, 0xac, 2, 1, 0, 0, 0x0f, 0xac, 2, 1, 0, 0, 0x0f, 0xac, 2, 0, 0];
    assert_eq!(w::parse_bss(&beacon(b"old", 1, Some(&tkip_only), false)).unwrap().security, w::Security::Unsupported);
    assert!(w::parse_bss(&beacon(b"x", 1, None, false)[..30]).is_none());
    assert_eq!(w::channel_mhz(6, false), 2437);
    assert_eq!(w::channel_mhz(36, false), 5180);
    assert_eq!(w::channel_mhz(1, true), 5955);
}

fn run_handshake(akm: eapol::Akm, pmk: [u8; 32], with_igtk: bool) -> eapol::Install {
    let mut rsn = [0u8; 32];
    let n = w::own_rsn(akm == eapol::Akm::Sae, &mut rsn);
    let rsn = &rsn[..n];
    let anonce = [0x5a; 32];
    let snonce = [0xa5; 32];
    let mut sta = eapol::Supplicant::new(akm, pmk, AP, STA, snonce, rsn, rsn);
    let mut frame = [0u8; eapol::MAX_FRAME];
    let mut reply = [0u8; eapol::MAX_FRAME];
    let len = eapol::authenticator::message1(&mut frame, akm, 1, &anonce);
    let (r, keys) = sta.receive(&frame[..len], &mut reply).unwrap();
    assert!(keys.pairwise.is_none());
    let (ptk, got_snonce) = eapol::authenticator::check_message2(&reply[..r], akm, &pmk, &AP, &STA, &anonce).unwrap();
    assert_eq!(got_snonce, snonce);
    let gtk = eapol::GroupKey { id: 1, key: [0x77; 16], rsc: 42 };
    let igtk = eapol::IntegrityGroupKey { id: 4, key: [0x99; 16], ipn: 7 };
    let len = eapol::authenticator::message3(&mut frame, akm, &ptk, 2, &anonce, rsn, &gtk, with_igtk.then_some(&igtk));
    let (r, keys) = sta.receive(&frame[..len], &mut reply).unwrap();
    eapol::authenticator::check_message4(&reply[..r], akm, &ptk).unwrap();
    assert_eq!(sta.state, eapol::State::Done);
    assert_eq!(keys.pairwise, Some(ptk.tk));
    assert_eq!(keys.group, Some(gtk));
    assert_eq!(keys.igtk, with_igtk.then_some(igtk));
    // A replayed message 3 is dropped.
    assert_eq!(sta.receive(&frame[..len], &mut reply).err(), Some(eapol::Error::Replay));
    // The AP lost message 4 and retransmits message 3 with a new counter: we
    // answer again but install nothing twice (KRACK).
    let len = eapol::authenticator::message3(&mut frame, akm, &ptk, 3, &anonce, rsn, &gtk, with_igtk.then_some(&igtk));
    let (r, again) = sta.receive(&frame[..len], &mut reply).unwrap();
    eapol::authenticator::check_message4(&reply[..r], akm, &ptk).unwrap();
    assert!(again.pairwise.is_none() && again.group.is_none() && again.igtk.is_none());
    keys
}

#[test]
fn wpa2_personal_four_way_handshake() {
    let pmk = sex_wifi::crypto::psk_from_passphrase(b"correct horse battery", b"home");
    run_handshake(eapol::Akm::Psk, pmk, false);
}

#[test]
fn wpa3_personal_four_way_handshake_after_sae() {
    let pwe = sex_wifi::sae::pwe_hunt_and_peck(&STA, &AP, b"correct horse").unwrap();
    let mut a = sex_wifi::sae::Sae::new(pwe, &[0x11; 32], &[0x22; 32]).unwrap();
    let mut b = sex_wifi::sae::Sae::new(pwe, &[0x33; 32], &[0x44; 32]).unwrap();
    a.process_commit(&b.commit()).unwrap();
    b.process_commit(&a.commit()).unwrap();
    run_handshake(eapol::Akm::Sae, a.pmk, true);
}

#[test]
fn handshake_refuses_wrong_password_and_downgrade() {
    let akm = eapol::Akm::Psk;
    let mut rsn = [0u8; 32];
    let n = w::own_rsn(false, &mut rsn);
    let rsn = &rsn[..n];
    let anonce = [1; 32];
    let ap_pmk = sex_wifi::crypto::psk_from_passphrase(b"right", b"home");
    let mut frame = [0u8; eapol::MAX_FRAME];
    let mut reply = [0u8; eapol::MAX_FRAME];
    // Wrong password: the AP rejects message 2.
    let mut sta = eapol::Supplicant::new(akm, sex_wifi::crypto::psk_from_passphrase(b"wrong", b"home"), AP, STA, [2; 32], rsn, rsn);
    let len = eapol::authenticator::message1(&mut frame, akm, 1, &anonce);
    let (r, _) = sta.receive(&frame[..len], &mut reply).unwrap();
    assert_eq!(eapol::authenticator::check_message2(&reply[..r], akm, &ap_pmk, &AP, &STA, &anonce).err(), Some(eapol::Error::Mic));
    // Downgrade: message 3 carries a different RSN element than the beacon had.
    let beacon_rsn = [48, 20, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 8, 0xc0, 0];
    let mut sta = eapol::Supplicant::new(akm, ap_pmk, AP, STA, [2; 32], rsn, &beacon_rsn);
    let len = eapol::authenticator::message1(&mut frame, akm, 1, &anonce);
    let (r, _) = sta.receive(&frame[..len], &mut reply).unwrap();
    let (ptk, _) = eapol::authenticator::check_message2(&reply[..r], akm, &ap_pmk, &AP, &STA, &anonce).unwrap();
    let gtk = eapol::GroupKey { id: 1, key: [3; 16], rsc: 0 };
    let len = eapol::authenticator::message3(&mut frame, akm, &ptk, 2, &anonce, rsn, &gtk, None);
    assert_eq!(sta.receive(&frame[..len], &mut reply).err(), Some(eapol::Error::RsnMismatch));
    // A tampered message 3 fails the MIC.
    let mut sta = eapol::Supplicant::new(akm, ap_pmk, AP, STA, [2; 32], rsn, rsn);
    let len = eapol::authenticator::message1(&mut frame, akm, 1, &anonce);
    sta.receive(&frame[..len], &mut reply).unwrap();
    let len = eapol::authenticator::message3(&mut frame, akm, &ptk, 2, &anonce, rsn, &gtk, None);
    frame[len - 1] ^= 1;
    assert_eq!(sta.receive(&frame[..len], &mut reply).err(), Some(eapol::Error::Mic));
}

#[test]
fn ethernet_over_wifi_round_trip_with_ccmp() {
    let tk = [0x42; 16];
    // IP stack -> 802.11 -> CCMP -> air.
    let mut eth = Vec::new();
    eth.extend_from_slice(&[0x02, 0x99, 0x99, 0x99, 0x99, 0x99]);
    eth.extend_from_slice(&STA);
    eth.extend_from_slice(&0x0800u16.to_be_bytes());
    eth.extend_from_slice(b"an IPv4 packet would be here");
    let mut plain = [0u8; 2048];
    let (hdr, total) = w::from_ethernet(&eth, &AP, 9, &mut plain).unwrap();
    let mut air = vec![0u8; total + 16];
    ccmp::encrypt_frame(&tk, &plain[..total], hdr, 1, 0, &mut air).unwrap();
    assert_eq!(u16::from_le_bytes([air[0], air[1]]), w::FC_DATA | w::FC_TO_DS | w::FC_PROTECTED);
    // The AP relays it back down to us (from-DS): swap the addresses as an AP would.
    let mut down = air.clone();
    down[0..2].copy_from_slice(&(w::FC_DATA | w::FC_FROM_DS | w::FC_PROTECTED).to_le_bytes());
    down[4..10].copy_from_slice(&STA);
    down[10..16].copy_from_slice(&AP);
    down[16..22].copy_from_slice(&[0x02, 0x99, 0x99, 0x99, 0x99, 0x99]);
    let mut resealed = vec![0u8; down.len()];
    let mut clear = down.clone();
    clear[1] &= !0x40;
    let body: Vec<u8> = { let mut v = clear[..24].to_vec(); v.extend_from_slice(&plain[24..total]); v };
    ccmp::encrypt_frame(&tk, &body, 24, 2, 0, &mut resealed).unwrap();
    let h = w::data_header_len(&resealed).unwrap();
    let (pn, range) = ccmp::decrypt_frame(&tk, &mut resealed, h).unwrap();
    assert_eq!(pn, 2);
    let mut out = [0u8; 2048];
    let n = w::to_ethernet(&resealed[..h], &resealed[range], &mut out).unwrap();
    assert_eq!(&out[..6], &STA);
    assert_eq!(&out[6..12], &[0x02, 0x99, 0x99, 0x99, 0x99, 0x99]);
    assert_eq!(&out[12..n], &eth[12..]);
}

#[test]
fn authentication_and_association_frames() {
    let mut buf = [0u8; 256];
    let n = w::auth(&mut buf, &AP, &STA, 1, w::AUTH_OPEN, 1, 0, &[]);
    let a = w::parse_auth(&buf[..n]).unwrap();
    assert_eq!((a.algorithm, a.transaction, a.status), (0, 1, 0));
    assert_eq!(&buf[4..10], &AP);
    let pwe = sex_wifi::sae::pwe_hunt_and_peck(&STA, &AP, b"pw").unwrap();
    let s = sex_wifi::sae::Sae::new(pwe, &[5; 32], &[6; 32]).unwrap();
    let n = w::auth(&mut buf, &AP, &STA, 2, w::AUTH_SAE, 1, 0, &s.commit());
    assert_eq!(w::parse_auth(&buf[..n]).unwrap().body.len(), sex_wifi::sae::COMMIT_LEN);
    let mut rsn = [0u8; 32];
    let rn = w::own_rsn(true, &mut rsn);
    let bss = w::parse_bss(&beacon(b"wpa3", 36, Some(&rsn[..rn]), true)).unwrap();
    let n = w::assoc_request(&mut buf, &bss, &STA, 3, &rsn[..rn]);
    let ies: Vec<u8> = w::elements(&buf[28..n]).map(|(id, _)| id).collect();
    assert_eq!(ies, [0, 1, 48, 244]);
    let mut resp = [0u8; 30];
    resp[0] = 0x10;
    resp[28..30].copy_from_slice(&(0xc000u16 | 5).to_le_bytes());
    assert_eq!(w::parse_assoc_response(&resp), Some(w::AssocReply { status: 0, aid: 5 }));
    let mut deauth = [0u8; 26];
    deauth[0] = 0xc0;
    deauth[24] = 3;
    assert_eq!(w::parse_disconnect(&deauth), Some(3));
}
