//! Driver-level behaviour: chip table, firmware files, DMA layout, the power-up
//! sequence against a register model, the 4-way handshake against an access
//! point played by the authenticator helpers, and frames end to end.
use sex_wifi::{assoc, bringup, ccmp, cmdq, context, csr, devices, eapol, hcmd, ieee80211 as w, init, join, pack, rx, scan, tx, ucode};
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
fn synthetic_firmware() -> Vec<u8> { firmware_with(&[]) }

fn firmware_with(extra: &[(u32, Vec<u8>)]) -> Vec<u8> {
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
    for (kind, data) in extra { tlv(*kind, data); }
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
    // W5: the x17's firmware takes the join commands this driver sends.
    let v = join::Versions::of(&fw, &init::Features::of(&fw), 0x420);
    assert_eq!(v, join::Versions { phy: 4, rlc: 2, scd_queue: 3, uhb: true, cdb: false, mld: false });
    // The AX211-type MAC (0x44) would need the MLD API instead.
    assert!(join::Versions::of(&fw, &init::Features::of(&fw), 0x440).mld);
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

impl context::DmaRead for Memory {
    fn read(&self, offset: u64, out: &mut [u8]) { out.copy_from_slice(&self.0[offset as usize..offset as usize + out.len()]); }
}

/// What the firmware does with the receive ring: take the next posted page, write
/// one notification into it, and report it on the used ring and the status word.
fn firmware_delivers(m: &mut Memory, layout: &context::Layout, family: devices::Family, base: u64, slot: usize, packet: &[u8]) -> u16 {
    let (addr, id) = if family == devices::Family::Ax210 {
        let d = layout.rx_free.offset as usize + slot * 16;
        (u64_at(&m.0, d as u64 + 8), u16::from_le_bytes([m.0[d], m.0[d + 1]]))
    } else {
        let v = u64_at(&m.0, layout.rx_free.offset + slot as u64 * 8);
        (v & !0xfff, (v & 0xfff) as u16)
    };
    let page = (addr - base) as usize;
    m.0[page..page + packet.len()].copy_from_slice(packet);
    if family == devices::Family::Ax210 {
        let u = layout.rx_used.offset as usize + slot * 32;
        m.0[u + 4..u + 6].copy_from_slice(&id.to_le_bytes());
    } else {
        let u = layout.rx_used.offset as usize + slot * 4;
        m.0[u..u + 4].copy_from_slice(&(id as u32).to_le_bytes());
    }
    let s = layout.rx_status.offset as usize;
    m.0[s..s + 2].copy_from_slice(&((slot + 1) as u16).to_le_bytes());
    id
}

fn alive_packet(ok: bool) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    let len = 4 + 4 + 2 * 48 + 16;
    b[0..4].copy_from_slice(&(len as u32).to_le_bytes());
    b[4] = hcmd::ALIVE;
    b[7] = 0x80;
    b[8..10].copy_from_slice(&(if ok { hcmd::ALIVE_STATUS_OK } else { 0xdead }).to_le_bytes());
    b[12..16].copy_from_slice(&83u32.to_le_bytes());
    b
}

#[test]
fn receive_ring_posts_pages_and_recycles_them_on_both_families() {
    let file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&file).unwrap();
    let image = fw.image();
    const BASE: u64 = 0x2_0000_0000;
    for family in [devices::Family::Gen22000, devices::Family::Ax210] {
        let layout = context::Layout::plan(family, &image, 1234, 1 << 20).unwrap();
        assert_eq!(layout.rx_buffers.len, context::RX_BUFFERS as u64 * 4096);
        assert!(layout.rx_buffers.offset >= layout.paging[0].offset + 4096 && layout.end <= 1 << 20);
        let mut m = Memory(vec![0; 1 << 20]);
        layout.write(&mut m, BASE, &image, &[0xee; 1234], 0x420, false);
        let mut ring = rx::Rx::new(family);
        ring.fill(&mut m, &layout, BASE);
        assert_eq!(ring.write_index(), 64);
        assert!(ring.next(&m, &layout).is_none(), "nothing closed yet");
        let mut hw = Chip::new();
        ring.kick(&mut hw);
        assert!(hw.writes.contains(&(rx::RFH_Q0_FRBDCB_WIDX_TRG, 64)));
        // Two notifications: pages 0 and 1, in order.
        firmware_delivers(&mut m, &layout, family, BASE, 0, &alive_packet(true));
        firmware_delivers(&mut m, &layout, family, BASE, 1, &alive_packet(true));
        assert_eq!(ring.next(&m, &layout), Some(Ok(0)));
        assert_eq!(ring.next(&m, &layout), Some(Ok(1)));
        assert_eq!(ring.next(&m, &layout), None);
        // Page 0 goes back at entry 64, pointing at the same memory.
        ring.post(&mut m, &layout, BASE, 0);
        let page0 = BASE + rx::Rx::buffer(&layout, 0).offset;
        if family == devices::Family::Ax210 {
            assert_eq!(u64_at(&m.0, layout.rx_free.offset + 64 * 16 + 8), page0);
        } else {
            assert_eq!(u64_at(&m.0, layout.rx_free.offset + 64 * 8), page0);
        }
        // A used entry naming a page that was never posted is refused.
        let u = if family == devices::Family::Ax210 { layout.rx_used.offset as usize + 2 * 32 + 4 } else { layout.rx_used.offset as usize + 2 * 4 };
        m.0[u..u + 2].copy_from_slice(&900u16.to_le_bytes());
        let s = layout.rx_status.offset as usize;
        m.0[s..s + 2].copy_from_slice(&3u16.to_le_bytes());
        assert_eq!(ring.next(&m, &layout), Some(Err(rx::Error::BadBuffer(900))));
    }
}

#[test]
fn bringup_reaches_alive_through_the_receive_ring() {
    let file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&file).unwrap();
    for (ok, want) in [(true, csr::Step::Alive), (false, csr::Step::Failed(csr::Failure::AliveTimeout))] {
        let chip = devices::identify(0x2725, 0x1674, 0x420, 0).unwrap();
        let base = 0x8000_0000u64;
        let mut m = Memory(vec![0; 1 << 20]);
        let mut b = bringup::Bringup::prepare(chip, &fw, &mut m, base, 1 << 20, 0x420, 0).unwrap();
        // The image loader went into the region, and the free ring is full.
        assert_eq!(m.0[b.layout.iml.offset as usize], 0xee);
        let mut hw = Chip::new();
        let mut started_at = None;
        let mut delivered = false;
        for _ in 0..2000 {
            let t = hw.now;
            if !b.advance(&mut hw, &mut m, t) { break; }
            if started_at.is_none() && hw.prph_writes.contains(&(csr::UREG_CPU_INIT_RUN + 0x30_0000, 1)) {
                // The free ring index was written before the CPU was started.
                let kick = hw.writes.iter().position(|w| *w == (rx::RFH_Q0_FRBDCB_WIDX_TRG, 64)).expect("free ring kicked");
                let start = hw.writes.iter().position(|w| w.0 == csr::PRPH_WDAT && w.1 == 1).unwrap();
                assert!(kick < start, "free ring visible before CPU start");
                started_at = Some(hw.now);
            }
            if let Some(s) = started_at { if !delivered && hw.now >= s + 3000 {
                firmware_delivers(&mut m, &b.layout, chip.family, base, 0, &alive_packet(ok));
                hw.alive_at = Some(hw.now);
                delivered = true;
            } }
            hw.now += 1000;
        }
        assert_eq!(b.boot.step, want);
        let alive = b.alive.expect("ALIVE parsed");
        assert_eq!((alive.ok, alive.lmac_major), (ok, 83));
        assert_eq!(b.notifications, 1);
        assert_eq!(b.rx.write, 65, "page recycled");
        // The ALIVE interrupt published the free ring again after the CPU start (iwx_intr).
        assert_eq!(b.boot.alive_interrupts, 1);
        let start = hw.writes.iter().position(|w| w.0 == csr::PRPH_WDAT && w.1 == 1).unwrap();
        assert!(hw.writes[start..].iter().any(|w| *w == (rx::RFH_Q0_FRBDCB_WIDX_TRG, 64)), "free ring republished on ALIVE");
    }
}

#[test]
fn firmware_pack_finds_checked_files() {
    use sha2::Digest;
    let files: [(&str, Vec<u8>); 2] = [("iwlwifi-ty-a0-gf-a0-77.ucode", vec![7u8; 5000]), ("iwlwifi-ty-a0-gf-a0.pnvm", vec![9u8; 100])];
    let mut pack = Vec::new();
    pack.extend_from_slice(pack::MAGIC);
    pack.extend_from_slice(&(files.len() as u32).to_le_bytes());
    pack.extend_from_slice(&0u32.to_le_bytes());
    let mut at = 4096usize;
    let mut data = vec![0u8; 0];
    for (name, bytes) in &files {
        let mut e = [0u8; pack::ENTRY];
        e[..name.len()].copy_from_slice(name.as_bytes());
        e[48..52].copy_from_slice(&(at as u32).to_le_bytes());
        e[52..56].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        e[56..88].copy_from_slice(&sha2::Sha256::digest(bytes));
        pack.extend_from_slice(&e);
        data.resize(at - 4096, 0);
        data.extend_from_slice(bytes);
        at = (4096 + data.len()).next_multiple_of(4096);
    }
    pack.resize(4096, 0);
    pack.extend_from_slice(&data);
    let p = pack::Pack::parse(&pack).unwrap();
    assert_eq!(p.len(), 2);
    assert_eq!(p.file("iwlwifi-ty-a0-gf-a0-77.ucode").unwrap(), &[7u8; 5000][..]);
    assert_eq!(p.file("iwlwifi-ty-a0-gf-a0.pnvm").unwrap().len(), 100);
    assert_eq!(p.file("iwlwifi-cc-a0-77.ucode").err(), Some(pack::Error::Missing));
    let mut bad = pack.clone();
    bad[4096] ^= 1;
    assert_eq!(pack::Pack::parse(&bad).unwrap().file("iwlwifi-ty-a0-gf-a0-77.ucode").err(), Some(pack::Error::Damaged));
    assert_eq!(pack::Pack::parse(&pack[..20]).err(), Some(pack::Error::Truncated));
    assert_eq!(pack::Pack::parse(b"NOTAPACK0000000000").err(), Some(pack::Error::Magic));
    // An empty pack (a build without firmware) parses and finds nothing.
    let mut empty = pack::MAGIC.to_vec(); empty.extend_from_slice(&[0u8; 8]);
    assert!(pack::Pack::parse(&empty).unwrap().is_empty());
}

#[test]
fn real_firmware_pack_starts_the_x17_chip_in_a_4_mib_pool() {
    // Set by scripts/wifi_host_gate.sh after packing the fetched linux-firmware files.
    let Some(path) = std::env::var_os("SEXOS_WIFI_PACK") else { return };
    let bytes = std::fs::read(&path).unwrap();
    let p = pack::Pack::parse(&bytes).unwrap();
    for name in devices::FIRMWARE_FILES {
        let file = p.file(name).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        if name.ends_with(".ucode") { assert!(ucode::Firmware::parse(file).is_ok(), "{name}"); }
    }
    let chip = devices::identify(0x2725, 0x1674, 0x420, 0x0010d000).unwrap();
    let fw = ucode::Firmware::parse(p.file(chip.firmware).unwrap()).unwrap();
    let mut m = Memory(vec![0; 4 << 20]);
    let b = bringup::Bringup::prepare(chip, &fw, &mut m, 0x1_0000_0000, 4 << 20, 0x420, 0).unwrap();
    assert!(b.layout.end <= 4 << 20 && b.iml_len > 0);
    assert_eq!(b.rx.write, context::RX_BUFFERS as u16);
    // Every SKU in the x17's .pnvm has a section set for its radio that fits.
    let pnvm = p.file(chip.pnvm.unwrap()).unwrap();
    let mut skus = 0;
    let mut rest = pnvm;
    while rest.len() >= 8 {
        let kind = u32::from_le_bytes(rest[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(rest[4..8].try_into().unwrap()) as usize;
        if kind == 64 {
            let sku = [0, 4, 8].map(|o| u32::from_le_bytes(rest[8 + o..12 + o].try_into().unwrap()));
            let mut alive = hcmd::parse_alive(&hcmd::packets(&alive_v6(sku)).next().unwrap()).unwrap();
            alive.sku = sku;
            let i = init::Init::new(chip, &fw, &alive, Some(pnvm), &b.layout, 0x1_0000_0000, &mut m, 0x420, 0x0010d000, 0);
            assert_eq!(i.pnvm.source, init::PnvmSource::File, "sku {sku:?}");
            assert!(i.pnvm.sections == 2 && i.pnvm.bytes > 10_000, "{:?}", i.pnvm);
            skus += 1;
        }
        rest = &rest[(8 + ((len + 3) & !3)).min(rest.len())..];
    }
    assert!(skus >= 1);
    // What the init sequence will see in the real firmware.
    let f = init::Features::of(&fw);
    assert!(f.api(ucode::API_REDUCED_SCAN_CONFIG) && f.api(ucode::API_REGULATORY_NVM_INFO), "iwx(4) needs both");
    assert!(f.mcc_answer_version.is_some_and(|v| v < 8));
    assert_eq!((f.tx_antennas(), f.rx_antennas()), (3, 3));
}

// ---- Stage W3: commands and the init sequence ----

/// The x17's firmware as far as the init sequence cares: antennas A+B, the API and
/// capability bits iwx(4) tests, and the command versions of iwlwifi-ty-a0-gf-a0-77.
fn init_firmware(dqa: bool, embedded_pnvm: Option<Vec<u8>>) -> Vec<u8> {
    let bits = |index: u32, bits: u32| { let mut v = index.to_le_bytes().to_vec(); v.extend_from_slice(&bits.to_le_bytes()); v };
    let mut extra = vec![
        (23, 0x0033_0018u32.to_le_bytes().to_vec()),
        (29, bits(0, 1 << ucode::API_WIFI_MCC_UPDATE)),
        (29, bits(1, 1 << (ucode::API_REGULATORY_NVM_INFO - 32) | 1 << (ucode::API_REDUCED_SCAN_CONFIG - 32))),
        (30, bits(0, 1 << ucode::CAPA_LAR_SUPPORT | if dqa { 1 << ucode::CAPA_DQA_SUPPORT } else { 0 })),
        (30, bits(1, 1 << (ucode::CAPA_ULTRA_HB_CHANNELS - 32))),
        (30, bits(2, 1 << (ucode::CAPA_CT_KILL_BY_FW - 64) | 1 << (ucode::CAPA_MCC_UPDATE_11AX_SUPPORT - 64))),
        // Command versions as the x17's firmware (-77) lists them, W5's included:
        // PHY_CONTEXT v4, RLC_CONFIG v2, SCD_QUEUE_CONFIG v3.
        (48, vec![0x01, 0x00, 99, 6, 0x0c, 0x01, 5, 0, 0x0d, 0x01, 15, 0, 0xc8, 0x01, 1, 6, 0x02, 0x0c, 1, 4,
            0x08, 0x01, 4, 0, 0x08, 0x05, 2, 0, 0x17, 0x05, 3, 2]),
    ];
    if let Some(p) = embedded_pnvm { extra.push((74, p)); }
    firmware_with(&extra)
}

fn tlv(out: &mut Vec<u8>, kind: u32, data: &[u8]) {
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    while out.len() % 4 != 0 { out.push(0); }
}

/// A .pnvm file shaped like iwlwifi-ty-a0-gf-a0.pnvm: per SKU, the hardware type,
/// a version, and two sections (each with its 4-byte load offset first).
fn pnvm_file(skus: &[([u32; 3], u16, u8)]) -> Vec<u8> {
    let mut f = vec![];
    for (sku, rf, fill) in skus {
        let s: Vec<u8> = sku.iter().flat_map(|w| w.to_le_bytes()).collect();
        tlv(&mut f, 64, &s);
        tlv(&mut f, 58, &[0x42, 0, (*rf & 0xff) as u8, (*rf >> 8) as u8, 0, 0, 0, 0]);
        tlv(&mut f, 62, &0x1234_5678u32.to_le_bytes());
        let mut a = 0xcafe_deadu32.to_le_bytes().to_vec(); a.extend(std::iter::repeat(*fill).take(1660));
        tlv(&mut f, 19, &a);
        tlv(&mut f, 19, &0xdddd_eeeeu32.to_le_bytes());
        let mut b = 0xdead_cafeu32.to_le_bytes().to_vec(); b.extend(std::iter::repeat(fill + 1).take(12000));
        tlv(&mut f, 19, &b);
    }
    f
}

const X17_SKU: [u32; 3] = [397520, 0, 0];
const X17_RF_ID: u32 = 0x0010_d000;

/// ALIVE v6 (144 bytes) with a SKU id.
fn alive_v6(sku: [u32; 3]) -> Vec<u8> {
    let mut b = vec![0u8; 160];
    b[0..4].copy_from_slice(&(4 + 144u32).to_le_bytes());
    b[4] = hcmd::ALIVE;
    b[7] = 0x80;
    b[8..10].copy_from_slice(&hcmd::ALIVE_STATUS_OK.to_le_bytes());
    b[12..16].copy_from_slice(&77u32.to_le_bytes());
    for (i, w) in sku.iter().enumerate() { b[8 + 116 + i * 4..8 + 120 + i * 4].copy_from_slice(&w.to_le_bytes()); }
    b
}

fn packet(code: u8, group: u8, index: u8, qid: u8, payload: &[u8]) -> Vec<u8> {
    let mut b = vec![0u8; 8];
    b[0..4].copy_from_slice(&(4 + payload.len() as u32).to_le_bytes());
    b[4] = code; b[5] = group; b[6] = index; b[7] = qid;
    b.extend_from_slice(payload);
    b
}

/// The firmware's side of the command queue: reads each command the driver
/// queued (TFD -> buffer), and answers on the receive ring the way the x17's
/// firmware does.
struct Firmware {
    family: devices::Family,
    base: u64,
    /// Receive-ring slot of the next delivery.
    slot: usize,
    /// Commands consumed from the ring.
    read: u32,
    commands: Vec<(u8, u8, Vec<u8>)>,
    lar: bool,
    refuse: Option<(u8, u8)>,
    error_on: Option<(u8, u8)>,
    silent_on: Option<(u8, u8)>,
    pnvm_rung: bool,
    /// Frames the radio "hears" during a scan: (frame, energy, channel, padded).
    air: Vec<(Vec<u8>, u8, u8, bool)>,
    scan_status: u8,
    /// ADD_STA's status word, and the queue number SCD_QUEUE_CONFIG answers with.
    station_status: u32,
    queue_answer: u16,
    /// The station TX queue (W5b): entries read, frames sent (TX command, 802.11
    /// frame), and how the AP answers.
    tx_read: u32,
    frames: Vec<(Vec<u8>, Vec<u8>)>,
    ap: Ap,
}

/// The AP's side of authentication and association.
#[derive(Clone)]
struct Ap { auth_status: u16, assoc_status: u16, aid: u16, ignore: u8, tx_status: u16, deauth_after_assoc: Option<u16> }
impl Default for Ap { fn default() -> Self { Ap { auth_status: 0, assoc_status: 0, aid: 5, ignore: 0, tx_status: 1, deauth_after_assoc: None } } }

/// A management frame from the AP (`bssid`) to `da`.
fn from_ap(fc: u16, da: &[u8], bssid: &[u8], body: &[u8]) -> Vec<u8> {
    let mut f = vec![0u8; 24];
    f[0..2].copy_from_slice(&fc.to_le_bytes());
    f[4..10].copy_from_slice(da); f[10..16].copy_from_slice(bssid); f[16..22].copy_from_slice(bssid);
    f.extend_from_slice(body);
    f
}

/// An RX MPDU notification as the firmware writes it (descriptor, then frame).
fn mpdu(family: devices::Family, frame: &[u8], energy: u8, channel: u8, pad: bool) -> Vec<u8> {
    let desc = if family == devices::Family::Ax210 { 64 } else { 48 };
    let mut d = vec![0u8; desc];
    d[0..2].copy_from_slice(&(frame.len() as u16 + if pad { 2 } else { 0 }).to_le_bytes());
    if pad { d[3] = 0x20; }
    d[12..16].copy_from_slice(&3u32.to_le_bytes());
    let e = if family == devices::Family::Ax210 { 40 } else { 32 };
    d[e] = energy; d[e + 1] = 0; d[e + 2] = channel;
    if pad { d.extend_from_slice(&frame[..24]); d.extend_from_slice(&[0, 0]); d.extend_from_slice(&frame[24..]); }
    else { d.extend_from_slice(frame); }
    packet(0xc1, 0, 0, 0x80, &d)
}

impl Firmware {
    /// Frames the driver queued on TX queue 1: checks the TFD, TX command and
    /// byte count, answers with a TX response, then lets the AP answer.
    fn serve_tx(&mut self, m: &mut Memory, layout: &context::Layout, hw: &mut Chip) {
        let Some(wp) = hw.writes.iter().rev().find(|w| w.0 == csr::TARG_WRPTR && w.1 >> 16 == 1).map(|w| w.1 & 0xffff) else { return };
        while self.tx_read != wp {
            let entry = self.tx_read as u64 % context::TX_RING_COUNT as u64;
            let tfd = m.0[(layout.tx_ring.offset + entry * 256) as usize..][..256].to_vec();
            let n = u16::from_le_bytes([tfd[0], tfd[1]]) as usize;
            assert!(n == 2 || n == 3, "TB0, TB1 and the body");
            let mut tbs = vec![];
            for i in 0..n {
                let at = 2 + i * 10;
                let len = u16::from_le_bytes([tfd[at], tfd[at + 1]]) as usize;
                let off = (u64::from_le_bytes(tfd[at + 2..at + 10].try_into().unwrap()) - self.base) as usize;
                tbs.push(m.0[off..off + len].to_vec());
            }
            assert_eq!(tbs[0].len(), hcmd::FIRST_TB_SIZE);
            let cmd: Vec<u8> = tbs[0].iter().chain(tbs[1].iter()).copied().collect();
            assert_eq!((cmd[0], cmd[2] as u32, cmd[3]), (hcmd::TX_CMD, self.tx_read % 256, 1));
            let tx = &cmd[4..];
            let size = if self.family == devices::Family::Ax210 { tx::GEN3_BYTES } else { tx::GEN2_BYTES };
            let len = u16::from_le_bytes([tx[0], tx[1]]) as usize;
            let offload = if self.family == devices::Family::Ax210 { u32::from_le_bytes(tx[4..8].try_into().unwrap()) } else { u16::from_le_bytes([tx[2], tx[3]]) as u32 };
            let hdr = ((offload >> 8) & 0x1f) as usize * 2;
            let pad = if offload & 1 << 13 != 0 { 4 - hdr % 4 } else { 0 };
            assert_eq!(tx.len(), size + hdr + pad, "TB1 ends after the header and pad");
            let mut frame = tx[size..size + hdr].to_vec();
            if n == 3 { frame.extend_from_slice(&tbs[2]); }
            assert_eq!(frame.len(), len, "TX command length is the whole frame");
            let bc = u16::from_le_bytes(m.0[(layout.tx_byte_counts.offset + entry * 2) as usize..][..2].try_into().unwrap());
            assert_eq!(bc, if self.family == devices::Family::Ax210 { len as u16 } else { len.div_ceil(4) as u16 });
            self.frames.push((tx[..size].to_vec(), frame.clone()));
            self.tx_read = (self.tx_read + 1) % if self.family == devices::Family::Ax210 { 65536 } else { 256 };
            let mut resp = vec![0u8; 48];
            resp[0] = 1;
            resp[40..42].copy_from_slice(&self.ap.tx_status.to_le_bytes());
            resp[44..48].copy_from_slice(&self.tx_read.to_le_bytes());
            self.deliver(m, layout, &packet(hcmd::TX_CMD, 0, cmd[2], 1, &resp));
            if self.ap.tx_status != 1 { continue; }
            if self.ap.ignore > 0 { self.ap.ignore -= 1; continue; }
            let (me, bssid) = (frame[10..16].to_vec(), frame[4..10].to_vec());
            let reply = match u16::from_le_bytes([frame[0], frame[1]]) {
                w::FC_MGMT_AUTH => {
                    let mut b = vec![0, 0, 2, 0]; b.extend_from_slice(&self.ap.auth_status.to_le_bytes());
                    from_ap(w::FC_MGMT_AUTH, &me, &bssid, &b)
                }
                w::FC_MGMT_ASSOC_REQ => {
                    let mut b = vec![0x01, 0x04]; b.extend_from_slice(&self.ap.assoc_status.to_le_bytes());
                    b.extend_from_slice(&(self.ap.aid | 0xc000).to_le_bytes());
                    from_ap(w::FC_MGMT_ASSOC_RESP, &me, &bssid, &b)
                }
                _ => continue,
            };
            // Another station's answer first: it must be ignored.
            let other = from_ap(w::FC_MGMT_AUTH, &[2, 2, 2, 2, 2, 2], &bssid, &[0, 0, 2, 0, 1, 0]);
            self.deliver(m, layout, &mpdu(self.family, &other, 40, 6, true));
            self.deliver(m, layout, &mpdu(self.family, &reply, 40, 6, true));
            if let (Some(r), w::FC_MGMT_ASSOC_REQ) = (self.ap.deauth_after_assoc, u16::from_le_bytes([frame[0], frame[1]])) {
                self.deliver(m, layout, &mpdu(self.family, &from_ap(w::FC_MGMT_DEAUTH, &me, &bssid, &r.to_le_bytes()), 40, 6, true));
            }
        }
    }

    fn new(family: devices::Family, base: u64, slot: usize) -> Self {
        Self { family, base, slot, read: 0, commands: vec![], lar: true, refuse: None, error_on: None, silent_on: None, pnvm_rung: false, air: vec![], scan_status: 1, station_status: 1, queue_answer: 1, tx_read: 0, frames: vec![], ap: Ap::default() }
    }
    fn deliver(&mut self, m: &mut Memory, layout: &context::Layout, p: &[u8]) {
        firmware_delivers(m, layout, self.family, self.base, self.slot, p);
        self.slot += 1;
    }
    fn serve(&mut self, m: &mut Memory, layout: &context::Layout, hw: &mut Chip) {
        let doorbell = csr::umac_prph(self.family, csr::UREG_DOORBELL_TO_ISR6) & 0xff_ffff;
        if !self.pnvm_rung && hw.prph_writes.contains(&(doorbell, csr::DOORBELL_PNVM)) {
            self.pnvm_rung = true;
            self.deliver(m, layout, &packet(hcmd::PNVM_INIT_COMPLETE, hcmd::REGULATORY_AND_NVM_GROUP, 0, 0x80, &[0; 4]));
        }
        self.serve_tx(m, layout, hw);
        let Some(wp) = hw.writes.iter().rev().find(|w| w.0 == csr::TARG_WRPTR && w.1 >> 16 == 0).map(|w| w.1 & 0xffff) else { return };
        while self.read != wp {
            let entry = self.read as u64 % context::TX_RING_COUNT as u64;
            let tfd = &m.0[(layout.cmd_ring.offset + entry * 256) as usize..][..256];
            let n = u16::from_le_bytes([tfd[0], tfd[1]]) as usize;
            let mut bytes = vec![];
            for i in 0..n {
                let at = 2 + i * 10;
                let len = u16::from_le_bytes([tfd[at], tfd[at + 1]]) as usize;
                let addr = u64::from_le_bytes(tfd[at + 2..at + 10].try_into().unwrap());
                let off = (addr - self.base) as usize;
                bytes.extend_from_slice(&m.0[off..off + len]);
            }
            let (opcode, group, index, qid) = (bytes[0], bytes[1], bytes[2], bytes[3]);
            let len = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
            assert_eq!(bytes.len(), 8 + len, "TFD covers the whole command");
            // Payload sizes the firmware ABI fixes (iwx(4) structs); a wrong size is refused.
            let want = match (group, opcode) {
                (hcmd::LONG_GROUP, hcmd::MCC_UPDATE_CMD) => Some(28),
                (hcmd::LONG_GROUP, hcmd::SCAN_CFG_CMD) => Some(12),
                (hcmd::LONG_GROUP, hcmd::REPLY_BEACON_FILTERING_CMD) => Some(60),
                (hcmd::PHY_OPS_GROUP, hcmd::TEMP_REPORTING_THRESHOLDS_CMD) => Some(20),
                (hcmd::LONG_GROUP, hcmd::BT_CONFIG) | (hcmd::SYSTEM_GROUP, hcmd::SOC_CONFIGURATION_CMD) => Some(8),
                (hcmd::LONG_GROUP, hcmd::SCAN_REQ_UMAC) => Some(scan::REQUEST_V14_BYTES),
                (hcmd::LONG_GROUP, hcmd::PHY_CONTEXT_CMD) => Some(join::PHY_CONTEXT_BYTES),
                (hcmd::DATA_PATH_GROUP, hcmd::RLC_CONFIG_CMD) => Some(32),
                (hcmd::LONG_GROUP, hcmd::MAC_CONTEXT_CMD) => Some(148),
                (hcmd::LONG_GROUP, hcmd::BINDING_CONTEXT_CMD) => Some(28),
                (hcmd::LONG_GROUP, hcmd::ADD_STA) => Some(48),
                (hcmd::DATA_PATH_GROUP, hcmd::SCD_QUEUE_CONFIG_CMD) => Some(36),
                (hcmd::MAC_CONF_GROUP, hcmd::SESSION_PROTECTION_CMD) => Some(24),
                _ => Some(4),
            };
            assert_eq!(Some(len), want, "payload size of {group:#x}/{opcode:#x}");
            assert_eq!((qid, index as u32), (0, self.read % 256));
            let payload = bytes[8..].to_vec();
            self.commands.push((group, opcode, payload));
            self.read = (self.read + 1) % if self.family == devices::Family::Ax210 { 65536 } else { 256 };
            if self.silent_on == Some((group, opcode)) { continue; }
            if self.error_on == Some((group, opcode)) {
                let mut e = vec![0u8; 20];
                e[0..4].copy_from_slice(&0x1234u32.to_le_bytes());
                e[4] = opcode;
                self.deliver(m, layout, &packet(hcmd::REPLY_ERROR, 0, 0, 0x80, &e));
                continue;
            }
            let failed = if self.refuse == Some((group, opcode)) { hcmd::CMD_FAILED } else { 0 };
            let answer: Vec<u8> = match (group, opcode) {
                (hcmd::REGULATORY_AND_NVM_GROUP, hcmd::NVM_GET_INFO) => {
                    let mut a = vec![0u8; 468];
                    a[4..6].copy_from_slice(&0x0a1du16.to_le_bytes());
                    a[7] = 1;
                    a[8..12].copy_from_slice(&(init::SKU_2GHZ | init::SKU_5GHZ | init::SKU_11N | init::SKU_11AC | init::SKU_11AX).to_le_bytes());
                    a[12..16].copy_from_slice(&3u32.to_le_bytes());
                    a[16..20].copy_from_slice(&3u32.to_le_bytes());
                    a[20..24].copy_from_slice(&(self.lar as u32).to_le_bytes());
                    a[24..28].copy_from_slice(&110u32.to_le_bytes());
                    for c in 0..59 { a[28 + c * 4] = 1; }
                    a
                }
                (hcmd::LONG_GROUP, hcmd::MCC_UPDATE_CMD) => {
                    let mut a = vec![0u8; 20 + 4 * 37];
                    a[4..6].copy_from_slice(&((b'D' as u16) << 8 | b'E' as u16).to_le_bytes());
                    a[12] = 0x10;
                    a[16..20].copy_from_slice(&37u32.to_le_bytes());
                    // Channels 1-13 and the first 23 at 5 GHz allowed; 14 not.
                    for c in (0..13).chain(14..37) { a[20 + c * 4] = 1; }
                    a
                }
                (hcmd::LONG_GROUP, hcmd::ADD_STA) => self.station_status.to_le_bytes().to_vec(),
                (hcmd::DATA_PATH_GROUP, hcmd::SCD_QUEUE_CONFIG_CMD) => {
                    let mut a = vec![0u8; 8];
                    a[0..2].copy_from_slice(&self.queue_answer.to_le_bytes());
                    a
                }
                _ => vec![0u8; 4],
            };
            self.deliver(m, layout, &packet(opcode, group | failed, index, qid, &answer));
            if (group, opcode) == (hcmd::LONG_GROUP, hcmd::SCAN_REQ_UMAC) && failed == 0 {
                for (frame, energy, channel, pad) in self.air.clone() { let p = mpdu(self.family, &frame, energy, channel, pad); self.deliver(m, layout, &p); }
                let mut it = vec![0u8; 16]; it[4] = 40;
                self.deliver(m, layout, &packet(scan::SCAN_ITERATION_COMPLETE_UMAC, 1, 0, 0x80, &it));
                let mut done = vec![0u8; 16]; done[6] = self.scan_status;
                self.deliver(m, layout, &packet(scan::SCAN_COMPLETE_UMAC, 1, 0, 0x80, &done));
            }
            if (group, opcode) == (hcmd::REGULATORY_AND_NVM_GROUP, hcmd::NVM_ACCESS_COMPLETE) {
                self.deliver(m, layout, &packet(hcmd::INIT_COMPLETE_NOTIF, 0, 0, 0x80, &[0; 4]));
            }
        }
    }
}

/// Powers the x17's chip up to ALIVE (v6, with `sku`), as the W2 test does.
fn alive_x17(fw: &ucode::Firmware<'_>, sku: [u32; 3]) -> (bringup::Bringup, Memory, Chip, u64) {
    let chip = devices::identify(0x2725, 0x1674, 0x420, X17_RF_ID).unwrap();
    let base = 0x8000_0000u64;
    let mut m = Memory(vec![0; 2 << 20]);
    let mut b = bringup::Bringup::prepare(chip, fw, &mut m, base, 2 << 20, 0x420, 0).unwrap();
    let mut hw = Chip::new();
    hw.regs.insert(csr::MAC_ADDR_STRAP, 0x0211_2233);
    hw.regs.insert(csr::MAC_ADDR_STRAP + 4, 0x0000_4455);
    let mut started = None;
    for _ in 0..2000 {
        let t = hw.now;
        if !b.advance(&mut hw, &mut m, t) { break; }
        if started.is_none() && hw.prph_writes.contains(&(csr::UREG_CPU_INIT_RUN + 0x30_0000, 1)) { started = Some(hw.now); }
        if started.is_some_and(|s| hw.now == s + 3000) {
            firmware_delivers(&mut m, &b.layout, chip.family, base, 0, &alive_v6(sku));
            hw.alive_at = Some(hw.now);
        }
        hw.now += 1000;
    }
    assert_eq!(b.boot.step, csr::Step::Alive);
    hw.alive_at = None;
    (b, m, hw, base)
}

fn run_init(i: &mut init::Init, b: &mut bringup::Bringup, m: &mut Memory, hw: &mut Chip, fw: &mut Firmware) {
    for _ in 0..10_000 {
        let t = hw.now;
        if !i.advance(b, hw, m, t) { break; }
        fw.serve(m, &b.layout, hw);
        hw.now += 1000;
    }
}

#[test]
fn alive_v6_carries_the_sku_and_older_layouts_do_not() {
    let p = alive_v6(X17_SKU);
    let pk = hcmd::packets(&p).next().unwrap();
    let a = hcmd::parse_alive(&pk).unwrap();
    assert!(a.ok);
    assert_eq!((a.lmac_major, a.sku), (77, X17_SKU));
    let old = alive_packet(true);
    assert_eq!(hcmd::parse_alive(&hcmd::packets(&old).next().unwrap()).unwrap().sku, [0; 3]);
}

#[test]
fn pnvm_section_for_the_sku_and_this_radio_only() {
    let file = pnvm_file(&[([1, 0, 0], 0x10d, 0x10), (X17_SKU, 0x10a, 0x20), (X17_SKU, 0x10d, 0x30)]);
    // The first chunk for the SKU is for another radio: no match there, so the
    // driver, like iwx(4), does not take it; pnvm_for_sku returns the first chunk.
    let chunk = ucode::pnvm_for_sku(&file, X17_SKU).unwrap();
    assert!(ucode::pnvm_image(chunk, 0x42, 0x10d).is_none());
    let img = ucode::pnvm_image(chunk, 0x42, 0x10a).unwrap();
    assert_eq!((img.count, img.bytes, img.version), (2, 1660 + 12000, 0x1234_5678));
    assert_eq!(img.sections()[0][0], 0x20);
    assert_eq!(img.sections()[1][0], 0x21);
    assert!(ucode::pnvm_for_sku(&file, [9, 9, 9]).is_none());
    // Selection tries every chunk for the SKU, as iwx_pnvm_parse does.
    let best = ucode::pnvm_select(&file, X17_SKU, 0x42, 0x10d).unwrap();
    assert_eq!(best.sections()[0][0], 0x30);
    assert!(ucode::pnvm_select(&file, X17_SKU, 0x41, 0x10d).is_none());

    let fw_file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&fw_file).unwrap();
    let image = fw.image();
    let layout = context::Layout::plan(devices::Family::Ax210, &image, 1234, 2 << 20).unwrap();
    let mut m = Memory(vec![0; 2 << 20]);
    const BASE: u64 = 0x1_0000_0000;
    // Joined: one block, its address and byte size in the peripheral scratch.
    let (addr, size) = layout.write_pnvm(&mut m, BASE, img.sections(), false).unwrap();
    assert_eq!((addr, size), (BASE + layout.pnvm.offset + 4096, 13660));
    assert_eq!(u64_at(&m.0, layout.prph_scratch.offset + 16), addr);
    assert_eq!(u32::from_le_bytes(m.0[(layout.prph_scratch.offset + 24) as usize..][..4].try_into().unwrap()), 13660);
    let at = (addr - BASE) as usize;
    assert_eq!((m.0[at], m.0[at + 1659], m.0[at + 1660]), (0x20, 0x20, 0x21));
    // Fragmented: a table of segment addresses, each segment page aligned.
    let (addr, _) = layout.write_pnvm(&mut m, BASE, img.sections(), true).unwrap();
    assert_eq!(addr, BASE + layout.pnvm.offset);
    let s0 = u64_at(&m.0, layout.pnvm.offset);
    let s1 = u64_at(&m.0, layout.pnvm.offset + 8);
    assert_eq!((s0, s1), (BASE + layout.pnvm.offset + 4096, BASE + layout.pnvm.offset + 8192));
    assert_eq!(m.0[(s1 - BASE) as usize], 0x21);
    // The 22000 family has no PNVM.
    let l22 = context::Layout::plan(devices::Family::Gen22000, &image, 0, 2 << 20).unwrap();
    assert!(l22.write_pnvm(&mut m, BASE, img.sections(), false).is_none());
}

#[test]
fn command_queue_frames_commands_and_matches_answers() {
    let fw_file = synthetic_firmware();
    let fw = ucode::Firmware::parse(&fw_file).unwrap();
    for family in [devices::Family::Gen22000, devices::Family::Ax210] {
        let layout = context::Layout::plan(family, &fw.image(), 0, 2 << 20).unwrap();
        let mut m = Memory(vec![0; 2 << 20]);
        let mut hw = Chip::new();
        let mut q = cmdq::CommandQueue::new(family);
        let base = 0x4000_0000;
        // 300 commands: the ring index wraps at 256; on AX210 the write pointer does not.
        for n in 0..300u32 {
            let payload = n.to_le_bytes();
            let s = q.send(&mut m, &mut hw, &layout, base, hcmd::SYSTEM_GROUP, hcmd::SOC_CONFIGURATION_CMD, 0, &payload).unwrap();
            assert_eq!(q.send(&mut m, &mut hw, &layout, base, 0, 1, 0, &[]), Err(cmdq::Error::Busy));
            let wp = hw.writes.iter().rev().find(|w| w.0 == csr::TARG_WRPTR).unwrap().1;
            let want = if family == devices::Family::Ax210 { n + 1 } else { (n + 1) % 256 };
            assert_eq!(wp, want);
            // An unsolicited packet or another index does not complete it.
            let note = packet(hcmd::SOC_CONFIGURATION_CMD, hcmd::SYSTEM_GROUP, s.index, 0x80, &[0; 4]);
            assert!(!q.on_packet(&hcmd::packets(&note).next().unwrap()));
            let other = packet(hcmd::SOC_CONFIGURATION_CMD, hcmd::SYSTEM_GROUP, s.index.wrapping_add(1), 0, &[0; 4]);
            assert!(!q.on_packet(&hcmd::packets(&other).next().unwrap()));
            let ans = packet(hcmd::SOC_CONFIGURATION_CMD, hcmd::SYSTEM_GROUP, s.index, 0, &[7, 0, 0, 0]);
            assert!(q.on_packet(&hcmd::packets(&ans).next().unwrap()));
            assert_eq!(q.response(), &[7, 0, 0, 0]);
            assert!(!q.answer.unwrap().failed);
        }
        // A legacy (group 0) command goes out as LONG_GROUP and its answer matches.
        let s = q.send(&mut m, &mut hw, &layout, base, 0, hcmd::BT_CONFIG, 0, &[0; 8]).unwrap();
        let buf = cmdq::CommandQueue::buffer(&layout, (q.cur + 255) % 256);
        assert_eq!(&m.0[buf.offset as usize..][..4], &[hcmd::BT_CONFIG, hcmd::LONG_GROUP, s.index, 0]);
        let ans = packet(hcmd::BT_CONFIG, hcmd::LONG_GROUP | hcmd::CMD_FAILED, s.index, 0, &[]);
        assert!(q.on_packet(&hcmd::packets(&ans).next().unwrap()));
        assert!(q.answer.unwrap().failed);
        assert_eq!(q.send(&mut m, &mut hw, &layout, base, 0, 1, 0, &[0; cmdq::MAX_PAYLOAD + 1]), Err(cmdq::Error::TooLong));
    }
}

#[test]
fn x17_init_sequence_reaches_ready() {
    let pnvm = pnvm_file(&[([1, 0, 0], 0x10d, 0x10), (X17_SKU, 0x10d, 0x30)]);
    let file = init_firmware(false, None);
    let fw = ucode::Firmware::parse(&file).unwrap();
    let (mut b, mut m, mut hw, base) = alive_x17(&fw, X17_SKU);
    let alive = b.alive.unwrap();
    let mut i = init::Init::new(b.boot.chip, &fw, &alive, Some(&pnvm), &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
    assert_eq!(i.pnvm, init::Pnvm { source: init::PnvmSource::File, sections: 2, bytes: 13660, version: 0x1234_5678 });
    assert_eq!(i.step, init::Step::Pnvm);
    let mut sim = Firmware::new(b.boot.chip.family, base, 1);
    run_init(&mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert_eq!(i.step, init::Step::Ready, "{:?}", i.step);
    assert!(sim.pnvm_rung && i.pnvm_loaded && i.init_complete);
    // The PNVM the chip was pointed at is the x17 SKU's.
    let addr = u64_at(&m.0, b.layout.prph_scratch.offset + 16);
    assert_eq!(m.0[(addr - base) as usize], 0x30);
    let order: Vec<(u8, u8)> = sim.commands.iter().map(|c| (c.0, c.1)).collect();
    use hcmd::*;
    assert_eq!(order, vec![
        (SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD), (REGULATORY_AND_NVM_GROUP, NVM_ACCESS_COMPLETE),
        (REGULATORY_AND_NVM_GROUP, NVM_GET_INFO), (LONG_GROUP, TX_ANT_CONFIGURATION_CMD), (LONG_GROUP, BT_CONFIG),
        (SYSTEM_GROUP, SOC_CONFIGURATION_CMD), (PHY_OPS_GROUP, TEMP_REPORTING_THRESHOLDS_CMD), (LONG_GROUP, POWER_TABLE_CMD),
        (LONG_GROUP, MCC_UPDATE_CMD), (LONG_GROUP, SCAN_CFG_CMD), (LONG_GROUP, REPLY_BEACON_FILTERING_CMD),
    ], "DQA is skipped: this firmware does not claim it");
    let payload = |g: u8, c: u8| sim.commands.iter().find(|x| (x.0, x.1) == (g, c)).unwrap().2.clone();
    assert_eq!(payload(SYSTEM_GROUP, INIT_EXTENDED_CFG_CMD), 2u32.to_le_bytes());
    assert_eq!(payload(LONG_GROUP, TX_ANT_CONFIGURATION_CMD), 3u32.to_le_bytes());
    assert_eq!(payload(LONG_GROUP, BT_CONFIG), [3, 0, 0, 0, 0, 0, 0, 0]);
    // The x17's AX210 is a discrete card: flag 1, no crystal latency.
    assert_eq!(payload(SYSTEM_GROUP, SOC_CONFIGURATION_CMD), [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(payload(PHY_OPS_GROUP, TEMP_REPORTING_THRESHOLDS_CMD), [0; 20]);
    let mcc = payload(LONG_GROUP, MCC_UPDATE_CMD);
    assert_eq!((mcc.len(), &mcc[..3]), (28, &[b'Z', b'Z', 0x10][..]));
    // SCAN_CFG v5: no broadcast station id; antennas A+B both ways.
    assert_eq!(payload(LONG_GROUP, SCAN_CFG_CMD), [0, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(payload(LONG_GROUP, REPLY_BEACON_FILTERING_CMD), vec![0u8; 60]);
    let nvm = i.nvm.unwrap();
    assert_eq!((nvm.version, nvm.tx_chains, nvm.rx_chains, nvm.lar, nvm.channels), (0x0a1d, 3, 3, true, 59));
    assert_eq!(i.mac, [0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
    assert_eq!(i.regulatory.unwrap().mcc, *b"DE");
    assert_eq!(i.regulatory.unwrap().channels, 37);
    // The NIC lock taken for the MAC address read was held through the commands
    // (as iwx_init_hw does) and given back at the end.
    assert_eq!(hw.regs[&csr::GP_CNTRL] & csr::GP_MAC_ACCESS_REQ, 0);
}

#[test]
fn init_without_lar_skips_mcc_and_dqa_firmware_gets_dqa() {
    let file = init_firmware(true, None);
    let fw = ucode::Firmware::parse(&file).unwrap();
    let (mut b, mut m, mut hw, base) = alive_x17(&fw, [0; 3]);
    let alive = b.alive.unwrap();
    let mut i = init::Init::new(b.boot.chip, &fw, &alive, None, &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
    // SKU 0: nothing to load, no doorbell.
    assert_eq!((i.step, i.pnvm.source), (init::Step::ExtendedConfig, init::PnvmSource::NotNeeded));
    let mut sim = Firmware::new(b.boot.chip.family, base, 1);
    sim.lar = false;
    run_init(&mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert_eq!(i.step, init::Step::Ready);
    assert!(!sim.pnvm_rung);
    assert!(sim.commands.iter().any(|c| (c.0, c.1, c.2.as_slice()) == (hcmd::DATA_PATH_GROUP, hcmd::DQA_ENABLE_CMD, &[0u8, 0, 0, 0][..])));
    assert!(!sim.commands.iter().any(|c| c.1 == hcmd::MCC_UPDATE_CMD));
    assert!(i.regulatory.is_none());
}

#[test]
fn embedded_pnvm_wins_and_unknown_sku_still_rings_the_doorbell() {
    let embedded = pnvm_file(&[(X17_SKU, 0x10d, 0x50)]);
    let file = init_firmware(false, Some(embedded));
    let fw = ucode::Firmware::parse(&file).unwrap();
    let (b, mut m, hw, base) = alive_x17(&fw, X17_SKU);
    let i = init::Init::new(b.boot.chip, &fw, &b.alive.unwrap(), Some(&pnvm_file(&[(X17_SKU, 0x10d, 0x30)])), &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
    assert_eq!(i.pnvm.source, init::PnvmSource::Embedded);
    assert_eq!(m.0[(u64_at(&m.0, b.layout.prph_scratch.offset + 16) - base) as usize], 0x50);

    let file = init_firmware(false, None);
    let fw = ucode::Firmware::parse(&file).unwrap();
    let (mut b, mut m, mut hw, base) = alive_x17(&fw, [7, 7, 7]);
    let alive = b.alive.unwrap();
    let mut i = init::Init::new(b.boot.chip, &fw, &alive, Some(&pnvm_file(&[(X17_SKU, 0x10d, 0x30)])), &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
    assert_eq!((i.step, i.pnvm.source), (init::Step::Pnvm, init::PnvmSource::NoMatch));
    let mut sim = Firmware::new(b.boot.chip.family, base, 1);
    run_init(&mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert!(sim.pnvm_rung, "the firmware is told to go on without it");
    assert_eq!(i.step, init::Step::Ready);
    // Missing file: same.
    let (b, mut m, hw, base) = alive_x17(&fw, X17_SKU);
    let i = init::Init::new(b.boot.chip, &fw, &b.alive.unwrap(), None, &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
    assert_eq!((i.step, i.pnvm.source), (init::Step::Pnvm, init::PnvmSource::Missing));
}

#[test]
fn init_stops_on_refusal_error_silence_and_crash() {
    let file = init_firmware(false, None);
    let fw = ucode::Firmware::parse(&file).unwrap();
    let run = |setup: &dyn Fn(&mut Firmware, &mut Chip)| {
        let (mut b, mut m, mut hw, base) = alive_x17(&fw, [0; 3]);
        let alive = b.alive.unwrap();
        let mut i = init::Init::new(b.boot.chip, &fw, &alive, None, &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
        let mut sim = Firmware::new(b.boot.chip.family, base, 1);
        setup(&mut sim, &mut hw);
        run_init(&mut i, &mut b, &mut m, &mut hw, &mut sim);
        (i.step, hw.now)
    };
    use hcmd::*;
    assert_eq!(run(&|f, _| f.refuse = Some((SYSTEM_GROUP, SOC_CONFIGURATION_CMD))).0,
        init::Step::Failed(init::Failure::Refused { group: SYSTEM_GROUP, opcode: SOC_CONFIGURATION_CMD }));
    match run(&|f, _| f.error_on = Some((LONG_GROUP, BT_CONFIG))).0 {
        init::Step::Failed(init::Failure::Firmware(e)) => assert_eq!((e.kind, e.command), (0x1234, BT_CONFIG)),
        other => panic!("{other:?}"),
    }
    let (step, at) = run(&|f, _| f.silent_on = Some((REGULATORY_AND_NVM_GROUP, NVM_GET_INFO)));
    assert_eq!(step, init::Step::Failed(init::Failure::Timeout { group: REGULATORY_AND_NVM_GROUP, opcode: NVM_GET_INFO }));
    assert!(at < 3_000_000, "gave up after about a second, not never");
    // NVM_ACCESS_COMPLETE answered but INIT_COMPLETE never sent.
    let (step, _) = run(&|f, _| f.silent_on = Some((REGULATORY_AND_NVM_GROUP, NVM_ACCESS_COMPLETE)));
    assert!(matches!(step, init::Step::Failed(init::Failure::Timeout { .. })));
    let (step, _) = run(&|_, hw| { hw.regs.insert(csr::MAC_ADDR_STRAP, 0); hw.regs.insert(csr::MAC_ADDR_STRAP + 4, 0); });
    assert_eq!(step, init::Step::Failed(init::Failure::NoMacAddress));
    let (step, _) = run(&|_, hw| { hw.regs.insert(csr::INT, csr::INT_SW_ERR); });
    assert_eq!(step, init::Step::Failed(init::Failure::Crashed { cause: csr::INT_SW_ERR }));
}

#[test]
fn mac_address_comes_from_the_fuses() {
    assert_eq!(init::mac_from_words(0x0211_2233, 0x0000_4455), [0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
    assert!(init::valid_mac(&[0x02, 0x11, 0x22, 0x33, 0x44, 0x55]));
    for bad in [[0; 6], [0xff; 6], [0x02, 0xcc, 0xaa, 0xff, 0xee, 0x00], [0x01, 0, 0, 0, 0, 1]] { assert!(!init::valid_mac(&bad)); }
}

// ---- Stage W4: scanning ----

fn beacon_from(bssid: [u8; 6], ssid: &[u8], channel: u8, rsn: Option<&[u8]>) -> Vec<u8> {
    let mut f = beacon(ssid, channel, rsn, false);
    f[10..16].copy_from_slice(&bssid);
    f[16..22].copy_from_slice(&bssid);
    f
}

#[test]
fn scan_request_lists_allowed_channels_passively() {
    // 2.4 GHz 1-13 valid (14 not), 5 GHz 36-48 valid; channel 1 active.
    let mut flags = [0u32; 51];
    for f in flags.iter_mut().take(13) { *f = scan::CHANNEL_VALID; }
    flags[0] |= scan::CHANNEL_ACTIVE;
    for f in flags.iter_mut().skip(14).take(4) { *f = scan::CHANNEL_VALID; }
    let mut list = [scan::Channel::default(); 51];
    assert_eq!(scan::channels(&flags, true, &mut list), 17);
    assert_eq!((list[0], list[13]), (scan::Channel { number: 1, five_ghz: false, active: true }, scan::Channel { number: 36, five_ghz: true, active: false }));
    assert_eq!(scan::channels(&flags, false, &mut list), 13, "no 5 GHz without the band in the SKU");
    let n = scan::channels(&flags, true, &mut list);
    let mut req = [0u8; scan::REQUEST_V14_BYTES];
    let o = scan::Options { mac: STA, ext_channel: true, ds_param: true, has_5ghz: true, max_channels: 67 };
    assert_eq!(scan::request_v14(&mut req, &list[..n], &o), 1940);
    let flags16 = u16::from_le_bytes([req[8], req[9]]);
    assert_eq!(flags16, (1 << 11) | (1 << 1) | (1 << 2) | (1 << 7), "passive, pass all, iteration notice, adaptive dwell");
    assert_eq!((req[44], req[45]), (1 << 5, 17));
    // Channel entries: flags u32, number, band (1 = 2.4 GHz, 0 = 5 GHz), iterations.
    assert_eq!(&req[48 + 4..48 + 7], &[1, 1, 1]);
    assert_eq!(&req[48 + 13 * 8 + 4..48 + 13 * 8 + 7], &[36, 0, 1]);
    // Probe template: probe request from our address, broadcast, rates per band.
    let buf = 596 + 20;
    assert_eq!(req[buf], 0x40);
    assert_eq!(&req[buf + 10..buf + 16], &STA);
    assert_eq!(&req[buf + 26..buf + 28], &[1, 8]);
    assert_eq!(u16::from_le_bytes([req[596 + 6], req[596 + 7]]), 19, "2.4 GHz: rates, extended rates, DS parameter");
    // The firmware's channel limit is honoured.
    let o = scan::Options { max_channels: 5, ..o };
    scan::request_v14(&mut req, &list[..n], &o);
    assert_eq!(req[45], 5);
}

#[test]
fn mpdu_frames_with_and_without_header_padding() {
    let b = beacon_from(AP, b"Cafe", 6, None);
    for family in [devices::Family::Gen22000, devices::Family::Ax210] {
        for pad in [false, true] {
            let p = mpdu(family, &b, 45, 6, pad);
            let pk = hcmd::packets(&p).next().unwrap();
            let mut scratch = [0u8; 4096];
            let r = scan::parse_mpdu(&pk, family, &mut scratch).unwrap();
            assert_eq!((r.frame, r.rssi, r.channel), (&b[..], -45, 6));
        }
    }
    // A frame with a bad CRC is dropped.
    let mut p = mpdu(devices::Family::Ax210, &b, 45, 6, false);
    p[8 + 12] = 0;
    assert!(scan::parse_mpdu(&hcmd::packets(&p).next().unwrap(), devices::Family::Ax210, &mut [0; 4096]).is_none());
}

#[test]
fn x17_scan_finds_networks_after_ready() {
    let pnvm = pnvm_file(&[(X17_SKU, 0x10d, 0x30)]);
    let file = init_firmware(false, None);
    let fw = ucode::Firmware::parse(&file).unwrap();
    let (mut b, mut m, mut hw, base) = alive_x17(&fw, X17_SKU);
    let alive = b.alive.unwrap();
    let mut i = init::Init::new(b.boot.chip, &fw, &alive, Some(&pnvm), &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
    let mut sim = Firmware::new(b.boot.chip.family, base, 1);
    run_init(&mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert_eq!(i.step, init::Step::Ready);
    let wpa2 = [48u8, 20, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 2, 0, 0];
    let home = [0x02, 0x10, 0, 0, 0, 1];
    sim.air = vec![
        (beacon_from(home, b"Home", 6, Some(&wpa2)), 60, 6, true),
        (beacon_from(home, b"Home", 6, Some(&wpa2)), 48, 6, true),
        (beacon_from([0x02, 0x10, 0, 0, 0, 2], b"Cafe", 36, None), 70, 36, false),
    ];
    // The regulatory answer's channel list replaced the NVM's: 13 + 23 channels.
    let mut s = i.scan(hw.now);
    assert_eq!(s.channels, 36);
    for _ in 0..1000 {
        let t = hw.now;
        if !s.advance(&mut b, &mut i.cmdq, &mut hw, &mut m, t) { break; }
        sim.serve(&mut m, &b.layout, &mut hw);
        hw.now += 1000;
    }
    assert_eq!(s.step, scan::Step::Done);
    assert_eq!(s.count, 2);
    let home_net = s.networks().find(|n| n.bss.ssid() == b"Home").unwrap();
    assert_eq!((home_net.rssi, home_net.frames, home_net.bss.security), (-48, 2, w::Security::Wpa2Personal), "strongest kept");
    let cafe = s.networks().find(|n| n.bss.ssid() == b"Cafe").unwrap();
    assert_eq!((cafe.bss.channel, cafe.bss.security), (36, w::Security::Open));
    assert_eq!(s.channels, 40, "from the iteration notice");
    // An aborted scan is reported as such.
    sim.scan_status = 2;
    let mut s = i.scan(hw.now);
    for _ in 0..1000 {
        let t = hw.now;
        if !s.advance(&mut b, &mut i.cmdq, &mut hw, &mut m, t) { break; }
        sim.serve(&mut m, &b.layout, &mut hw);
        hw.now += 1000;
    }
    assert_eq!(s.step, scan::Step::Failed(scan::Failure::Status(2)));
}

#[test]
fn scan_keeps_the_strongest_networks_when_full() {
    let mut s = scan::Scan::new(devices::Family::Ax210, &[], &scan::Options { mac: STA, ext_channel: true, ds_param: false, has_5ghz: true, max_channels: 67 }, 0);
    for i in 0..(scan::MAX_NETWORKS as u8 + 4) {
        s.heard(&beacon_from([2, 0, 0, 0, 0, i], b"n", 1, None), -90 + i as i16, 1);
    }
    assert_eq!((s.count, s.dropped), (scan::MAX_NETWORKS, 4));
    assert!(s.networks().all(|n| n.rssi >= -90 + 4), "the four weakest gave way");
}

/// The x17 up to `[wifi.ready]` with the scan's Home network heard, ready to join.
fn x17_ready() -> (init::Init, bringup::Bringup, Memory, Chip, Firmware, w::Bss, join::Versions) {
    let pnvm = pnvm_file(&[(X17_SKU, 0x10d, 0x30)]);
    let file = init_firmware(false, None);
    let fw = ucode::Firmware::parse(&file).unwrap();
    let (mut b, mut m, mut hw, base) = alive_x17(&fw, X17_SKU);
    let alive = b.alive.unwrap();
    let mut i = init::Init::new(b.boot.chip, &fw, &alive, Some(&pnvm), &b.layout, base, &mut m, 0x420, X17_RF_ID, hw.now);
    let mut sim = Firmware::new(b.boot.chip.family, base, 1);
    run_init(&mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert_eq!(i.step, init::Step::Ready);
    let wpa2 = [48u8, 20, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 2, 0, 0];
    let bss = w::parse_bss(&beacon_from([0x02, 0x10, 0, 0, 0, 1], b"Home", 6, Some(&wpa2))).unwrap();
    let v = join::Versions::of(&fw, &i.features, 0x420);
    (i, b, m, hw, sim, bss, v)
}

fn run_join(j: &mut join::Join, i: &mut init::Init, b: &mut bringup::Bringup, m: &mut Memory, hw: &mut Chip, sim: &mut Firmware) {
    for _ in 0..1000 {
        let t = hw.now;
        if !j.advance(b, &mut i.cmdq, hw, m, t) { break; }
        sim.serve(m, &b.layout, hw);
        hw.now += 1000;
    }
}

#[test]
fn x17_joins_the_channel_like_iwx_auth() {
    let (mut i, mut b, mut m, mut hw, mut sim, bss, v) = x17_ready();
    assert_eq!(v, join::Versions { phy: 4, rlc: 2, scd_queue: 3, uhb: true, cdb: false, mld: false });
    let target = join::Target::of(&bss, 6);
    let mut j = join::Join::new(target, v, i.mac, i.valid_rx_antennas(), hw.now);
    let before = sim.commands.len();
    run_join(&mut j, &mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert_eq!(j.step, join::Step::Joined);
    let sent: Vec<(u8, u8)> = sim.commands[before..].iter().map(|c| (c.0, c.1)).collect();
    assert_eq!(sent, vec![
        (hcmd::LONG_GROUP, hcmd::PHY_CONTEXT_CMD), (hcmd::DATA_PATH_GROUP, hcmd::RLC_CONFIG_CMD),
        (hcmd::LONG_GROUP, hcmd::MAC_CONTEXT_CMD), (hcmd::LONG_GROUP, hcmd::BINDING_CONTEXT_CMD),
        (hcmd::LONG_GROUP, hcmd::ADD_STA), (hcmd::DATA_PATH_GROUP, hcmd::SCD_QUEUE_CONFIG_CMD),
        (hcmd::MAC_CONF_GROUP, hcmd::SESSION_PROTECTION_CMD),
    ], "iwx_auth's order");
    let c = |k: usize| &sim.commands[before + k].2;
    let u32at = |p: &Vec<u8>, at: usize| u32::from_le_bytes(p[at..at + 4].try_into().unwrap());
    // PHY: add, channel 6 in the wide field, 2.4 GHz band, no RX chain (RLC v2 carries it).
    assert_eq!((u32at(c(0), 4), u32at(c(0), 8), c(0)[12], u32at(c(0), 20)), (join::ACTION_ADD, 6, 1, 0));
    // RLC: antennas A+B valid (phy_config 0x330018 → 3), one idle and one active chain.
    assert_eq!(u32at(c(1), 4), 3 << 1 | 1 << 10 | 1 << 12);
    // MAC: our address and the AP's, beacons let through until associated.
    assert_eq!(&c(2)[16..22], &i.mac);
    assert_eq!(&c(2)[24..30], &bss.bssid);
    assert_eq!(u32at(c(2), 52), 1 << 2 | 1 << 6);
    assert_eq!(u32at(c(2), 116), 100, "beacon interval");
    // ADD_STA: the AP's address, station 0, MAC 0.
    assert_eq!((&c(4)[8..14], c(4)[16]), (&bss.bssid[..], 0));
    // Queue: our TFD ring and byte-count table, 256 entries (log2 - 3 = 5), TID 15.
    assert_eq!((u32at(c(5), 4), c(5)[8], u32at(c(5), 16)), (1, 15, 5));
    assert_eq!(u64::from_le_bytes(c(5)[20..28].try_into().unwrap()), b.base + b.layout.tx_byte_counts.offset);
    assert_eq!(u64::from_le_bytes(c(5)[28..36].try_into().unwrap()), b.base + b.layout.tx_ring.offset);
    // Session protection: nine beacon intervals.
    assert_eq!(u32at(c(6), 12), 900);
}

#[test]
fn join_fails_on_a_refused_station_or_a_wrong_queue() {
    let (mut i, mut b, mut m, mut hw, mut sim, bss, v) = x17_ready();
    sim.station_status = 2;
    let mut j = join::Join::new(join::Target::of(&bss, 6), v, i.mac, 3, hw.now);
    run_join(&mut j, &mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert!(matches!(j.step, join::Step::Failed(init::Failure::BadAnswer { opcode: hcmd::ADD_STA, .. })), "{:?}", j.step);
    let (mut i, mut b, mut m, mut hw, mut sim, bss, v) = x17_ready();
    sim.queue_answer = 2;
    let mut j = join::Join::new(join::Target::of(&bss, 6), v, i.mac, 3, hw.now);
    run_join(&mut j, &mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert!(matches!(j.step, join::Step::Failed(init::Failure::BadAnswer { opcode: hcmd::SCD_QUEUE_CONFIG_CMD, .. })), "{:?}", j.step);
    // A chip that only speaks the MLD API is refused up front, as are old PHY commands.
    let mld = join::Versions { mld: true, ..v };
    assert_eq!(join::Join::new(join::Target::of(&bss, 6), mld, i.mac, 3, 0).step, join::Step::Failed(init::Failure::Unsupported("mld_api")));
    let old = join::Versions { phy: 1, ..v };
    assert!(join::Join::new(join::Target::of(&bss, 6), old, i.mac, 3, 0).done());
}

#[test]
fn join_uses_only_the_antennas_the_nvm_allows() {
    let (mut i, mut b, mut m, mut hw, mut sim, bss, v) = x17_ready();
    assert_eq!(i.valid_rx_antennas(), 3);
    i.nvm.as_mut().unwrap().rx_chains = 0b10;
    assert_eq!(i.valid_rx_antennas(), 0b10, "firmware A+B narrowed to the NVM's B");
    let mut j = join::Join::new(join::Target::of(&bss, 6), v, i.mac, i.valid_rx_antennas(), hw.now);
    let before = sim.commands.len();
    run_join(&mut j, &mut i, &mut b, &mut m, &mut hw, &mut sim);
    let rlc = &sim.commands[before + 1];
    assert_eq!((rlc.0, rlc.1), (hcmd::DATA_PATH_GROUP, hcmd::RLC_CONFIG_CMD));
    assert_eq!(u32::from_le_bytes(rlc.2[4..8].try_into().unwrap()), 0b10 << 1 | 1 << 10 | 1 << 12);
}

#[test]
fn join_command_layouts() {
    // ACK rates (iwx_ack_rates): basic 1/2 Mb/s and 6 Mb/s on 2.4 GHz.
    let mut t = join::Target { bssid: [2, 0, 0, 0, 0, 1], channel: 1, beacon_interval: 100, dtim_period: 1,
        basic_cck: 0b0011, basic_ofdm: 0b1, short_slot: true, short_preamble: true };
    assert_eq!(join::ack_rates(&t), (0b0011, 0b1));
    // Only 11 Mb/s basic: every lower CCK rate joins; only 24 Mb/s OFDM: 6 and 12 join.
    t.basic_cck = 0b1000; t.basic_ofdm = 1 << 4;
    assert_eq!(join::ack_rates(&t), (0b1111, 1 << 4 | 1 << 2 | 1));
    // 5 GHz: no CCK basic rates, 1 Mb/s stays (the firmware ignores it there).
    t.channel = 36; t.basic_cck = 0b1111;
    assert_eq!(join::ack_rates(&t).0, 1);
    // PHY context without the 6 GHz numbering: 28 bytes, band then channel in one word.
    let mut p = [0u8; join::PHY_CONTEXT_BYTES];
    assert_eq!(join::phy_context(&mut p, join::ACTION_ADD, 36, false, false, Some(0x1406)), join::PHY_CONTEXT_V1_BYTES);
    assert_eq!((p[8], p[9], u32::from_le_bytes(p[12..16].try_into().unwrap()), u32::from_le_bytes(p[16..20].try_into().unwrap())), (0, 36, 0, 0x1406));
    // Dual-band (CDB) firmware runs a 5 GHz channel on LMAC 1, in the PHY context and the binding alike.
    join::phy_context(&mut p, join::ACTION_ADD, 36, true, true, None);
    assert_eq!(u32::from_le_bytes(p[16..20].try_into().unwrap()), 1);
    let mut bind = [0u8; join::BINDING_BYTES];
    join::binding(&mut bind, join::ACTION_ADD, 36, true);
    assert_eq!(u32::from_le_bytes(bind[24..28].try_into().unwrap()), 1);
    join::binding(&mut bind, join::ACTION_ADD, 6, true);
    assert_eq!(u32::from_le_bytes(bind[24..28].try_into().unwrap()), 0, "2.4 GHz stays on LMAC 0");
    // MAC context after association: the AID, no beacons, EDCA per FIFO.
    let mut mac = [0u8; join::MAC_CONTEXT_BYTES];
    join::mac_context(&mut mac, join::ACTION_MODIFY, &STA, &t, Some(0xc005));
    assert_eq!(u32::from_le_bytes(mac[52..56].try_into().unwrap()), 1 << 2);
    assert_eq!((u32::from_le_bytes(mac[100..104].try_into().unwrap()), u32::from_le_bytes(mac[136..140].try_into().unwrap())), (1, 5));
    // BE goes to FIFO 2: CWmin 15, CWmax 1023, AIFSN 3; VO to FIFO 4 with TXOP 1504 µs.
    assert_eq!(&mac[60 + 16..60 + 24], &[15, 0, 0xff, 3, 3, 1 << 2, 0, 0]);
    assert_eq!(&mac[60 + 32..60 + 40], &[3, 0, 7, 0, 2, 1 << 4, 0xe0, 5]);
    // Removing a context sends only its id and the action.
    join::mac_context(&mut mac, join::ACTION_REMOVE, &STA, &t, None);
    assert!(mac[8..].iter().all(|&b| b == 0));
}

#[test]
fn beacons_give_basic_rates_and_dtim() {
    let mut f = beacon(b"rates", 6, None, false);
    f.extend_from_slice(&[5, 4, 0, 3, 0, 0]); // TIM: DTIM count 0, period 3
    f.extend_from_slice(&[50, 4, 0x8c, 0x12, 0x98, 0x24]); // extended rates: 6 and 12 basic
    let b = w::parse_bss(&f).unwrap();
    assert_eq!((b.basic_cck, b.basic_ofdm, b.dtim_period), (0b1111, 0b101, 3));
    let t = join::Target::of(&b, 6);
    assert_eq!(join::ack_rates(&t), (0b1111, 0b101), "nothing below 6 Mb/s to add");
}

/// Joins the x17 to "Home" (WPA2, channel 6), then runs association against `ap`.
fn x17_associate(ap: Ap) -> (assoc::Assoc, Firmware, init::Init, w::Bss, usize) {
    let (mut i, mut b, mut m, mut hw, mut sim, bss, v) = x17_ready();
    let target = join::Target::of(&bss, 6);
    let mut j = join::Join::new(target, v, i.mac, i.valid_rx_antennas(), hw.now);
    run_join(&mut j, &mut i, &mut b, &mut m, &mut hw, &mut sim);
    assert_eq!(j.step, join::Step::Joined);
    sim.ap = ap;
    let before = sim.commands.len();
    let mut a = assoc::Assoc::new(bss, target, i.mac, b.boot.chip.family, hw.now);
    for _ in 0..5000 {
        let t = hw.now;
        if !a.advance(&mut b, &mut i.cmdq, &mut hw, &mut m, t) { break; }
        sim.serve(&mut m, &b.layout, &mut hw);
        hw.now += 1000;
    }
    (a, sim, i, bss, before)
}

#[test]
fn x17_authenticates_and_associates_like_net80211() {
    let (a, sim, i, bss, before) = x17_associate(Ap::default());
    assert_eq!(a.step, assoc::Step::Associated);
    assert_eq!(a.aid, 5);
    assert_eq!(sim.frames.len(), 2, "one auth, one assoc request");
    let (cmd, auth) = &sim.frames[0];
    assert_eq!(&auth[..2], &[0xb0, 0]);
    assert_eq!((&auth[4..10], &auth[10..16], &auth[16..22]), (&bss.bssid[..], &i.mac[..], &bss.bssid[..]));
    assert_eq!(&auth[24..30], &[0, 0, 1, 0, 0, 0], "open system, transaction 1");
    // TX command v10 (gen3): fixed rate, no encryption, a 24-byte header (12 words), no pad.
    assert_eq!(u16::from_le_bytes([cmd[2], cmd[3]]), tx::FLAG_CMD_RATE | tx::FLAG_ENCRYPT_DIS);
    assert_eq!(u32::from_le_bytes(cmd[4..8].try_into().unwrap()), 12 << 8);
    assert_eq!(u32::from_le_bytes(cmd[16..20].try_into().unwrap()), tx::management_rate(&a.target));
    let (_, req) = &sim.frames[1];
    assert_eq!(&req[..2], &[0, 0]);
    let ies: Vec<(u8, Vec<u8>)> = w::elements(&req[28..]).map(|(id, b)| (id, b.to_vec())).collect();
    assert_eq!(ies[0], (0, b"Home".to_vec()));
    let rsn = ies.iter().find(|e| e.0 == 48).expect("our RSN element on a WPA2 network");
    assert_eq!(&rsn.1[8..12], &[0, 0x0f, 0xac, 4], "CCMP pairwise");
    assert_eq!(&rsn.1[14..18], &[0, 0x0f, 0xac, 2], "PSK");
    // Then iwx_run: the station entry updated, the MAC context told the AID.
    let sent: Vec<(u8, u8)> = sim.commands[before..].iter().map(|c| (c.0, c.1)).collect();
    assert_eq!(sent, vec![(hcmd::LONG_GROUP, hcmd::ADD_STA), (hcmd::LONG_GROUP, hcmd::MAC_CONTEXT_CMD)]);
    let (sta, mac) = (&sim.commands[before].2, &sim.commands[before + 1].2);
    assert_eq!(sta[0], 1, "modify");
    let u32at = |p: &Vec<u8>, at: usize| u32::from_le_bytes(p[at..at + 4].try_into().unwrap());
    // The test beacon has no TIM, so beacons keep coming through (as in iwx) until a DTIM period is known.
    assert_eq!((u32at(mac, 4), u32at(mac, 52), u32at(mac, 100), u32at(mac, 136)), (join::ACTION_MODIFY, 1 << 2 | 1 << 6, 1, 5));
}

#[test]
fn association_retries_and_reports_refusals() {
    // The AP misses the first auth frame: the driver sends it again.
    let (a, sim, ..) = x17_associate(Ap { ignore: 1, ..Ap::default() });
    assert_eq!(a.step, assoc::Step::Associated);
    assert_eq!(sim.frames.iter().map(|f| f.1[0]).collect::<Vec<_>>(), vec![0xb0, 0xb0, 0x00]);
    let (a, sim, ..) = x17_associate(Ap { ignore: 9, ..Ap::default() });
    assert_eq!(a.step, assoc::Step::Refused(assoc::Refusal::NoAnswer("auth")));
    assert_eq!(sim.frames.len(), assoc::ATTEMPTS as usize);
    let (a, ..) = x17_associate(Ap { auth_status: 17, ..Ap::default() });
    assert_eq!(a.step, assoc::Step::Refused(assoc::Refusal::Status { frame: "auth", status: 17 }));
    let (a, ..) = x17_associate(Ap { assoc_status: 18, ..Ap::default() });
    assert_eq!(a.step, assoc::Step::Refused(assoc::Refusal::Status { frame: "assoc", status: 18 }));
    let (a, ..) = x17_associate(Ap { tx_status: 0x83, ..Ap::default() });
    assert_eq!(a.step, assoc::Step::Refused(assoc::Refusal::NotSent { frame: "auth", status: 0x83 }));
    let (a, ..) = x17_associate(Ap { deauth_after_assoc: Some(3), ..Ap::default() });
    assert_eq!(a.step, assoc::Step::Refused(assoc::Refusal::Disconnected(3)));
}

#[test]
fn tx_command_layouts_and_rates() {
    let hdr = [0x88u8; 26]; // a QoS data header: 13 words, padded by 2
    let mut out = [0u8; 64];
    let n = tx::command(&mut out, devices::Family::Ax210, 100, &hdr, 3, 7);
    assert_eq!(n, tx::GEN3_BYTES + 28);
    assert_eq!(u32::from_le_bytes(out[4..8].try_into().unwrap()), 13 << 8 | 1 << 13);
    let n = tx::command(&mut out, devices::Family::Gen22000, 100, &hdr, 3, 7);
    assert_eq!(n, tx::GEN2_BYTES + 28);
    assert_eq!((u16::from_le_bytes([out[2], out[3]]), u32::from_le_bytes(out[4..8].try_into().unwrap())), (13 << 8 | 1 << 13, 3));
    assert_eq!(u32::from_le_bytes(out[16..20].try_into().unwrap()), 7);
    assert_eq!(tx::byte_count(devices::Family::Ax210, 30, 3), 30);
    assert_eq!(tx::byte_count(devices::Family::Gen22000, 30, 3), 8);
    assert_eq!(tx::byte_count(devices::Family::Ax210, 30, 7), 30 | 1 << 14);
    let mut t = join::Target { bssid: AP, channel: 36, beacon_interval: 100, dtim_period: 1, basic_cck: 0, basic_ofdm: 0b101, short_slot: true, short_preamble: false };
    assert_eq!(tx::management_rate(&t), 1 << 14 | 1 << 8, "OFDM 6 Mb/s");
    t.channel = 1; t.basic_cck = 0b1100;
    assert_eq!(tx::management_rate(&t), 1 << 14 | 2, "CCK 5.5 Mb/s");
    t.basic_cck = 0; t.basic_ofdm = 0b100;
    assert_eq!(tx::management_rate(&t), 1 << 14 | 1 << 8 | 2, "OFDM-only 2.4 GHz network: 12 Mb/s");
}

#[test]
fn wpa3_only_networks_wait_for_sae() {
    let (_, _, _, _, _, mut bss, _) = x17_ready();
    bss.security = w::Security::Wpa3Personal;
    let a = assoc::Assoc::new(bss, join::Target::of(&bss, 6), STA, devices::Family::Ax210, 0);
    assert_eq!(a.step, assoc::Step::Failed(init::Failure::Unsupported("wpa3_sae")));
}
