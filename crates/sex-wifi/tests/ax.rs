// SPDX-License-Identifier: GPL-3.0-only
//! Stage X1 (Wi-Fi 4/5/6): elements, agreed mode, command layouts.
use sex_wifi::ax::{self, Ours, Width};
use sex_wifi::ieee80211 as w;
use sex_wifi::join;
use sex_wifi::tlc;

const AP: [u8; 6] = [2, 0, 0, 0, 0, 1];
const STA: [u8; 6] = [2, 0, 0, 0, 0, 2];

fn x17() -> Ours { Ours { tx_antennas: 3, rx_antennas: 3, max_width: Width::W160, vht: true, he_ctxt: 2, he_mbssid: true, tlc: 4 } }

/// HT capabilities: LDPC, 40 MHz, SGI 20/40, RX STBC; 64 KiB A-MPDUs, 8 µs
/// density; MCS 0..15.
fn ht_cap() -> Vec<u8> {
    let mut e = vec![45, 26];
    e.extend_from_slice(&(0x0001u16 | 0x0002 | 0x0020 | 0x0040 | 0x0100).to_le_bytes());
    e.push(3 | 6 << 2);
    let mut mcs = [0u8; 16];
    mcs[0] = 0xff;
    mcs[1] = 0xff;
    e.extend_from_slice(&mcs);
    e.extend_from_slice(&[0; 7]);
    e
}
fn ht_op(primary: u8, sco: u8, protection: u8) -> Vec<u8> {
    let mut e = vec![61, 22, primary, sco | 4, protection];
    e.extend_from_slice(&[0; 19]);
    e
}
fn vht_cap(cap: u32) -> Vec<u8> {
    let mut e = vec![191, 12];
    e.extend_from_slice(&cap.to_le_bytes());
    e.extend_from_slice(&0xfffau16.to_le_bytes()); // 2 streams, MCS 0..9
    e.extend_from_slice(&[0, 0]);
    e.extend_from_slice(&0xfffau16.to_le_bytes());
    e.extend_from_slice(&[0, 0]);
    e
}
fn vht_op(width: u8, seg0: u8, seg1: u8) -> Vec<u8> { vec![192, 5, width, seg0, seg1, 0xfc, 0xff] }
fn he_cap(phy0: u8, phy6: u8, phy9: u8, ppe: &[u8]) -> Vec<u8> {
    let mut b = vec![35];
    b.extend_from_slice(&[0, 0, ax::HE_MAC2_32BIT_BA, 0, 0, 0]);
    let mut phy = [0u8; 11];
    phy[0] = phy0;
    phy[1] = ax::HE_PHY1_LDPC;
    phy[2] = ax::HE_PHY2_STBC_RX_80;
    phy[6] = phy6;
    phy[9] = phy9;
    b.extend_from_slice(&phy);
    b.extend_from_slice(&0xfffau16.to_le_bytes());
    b.extend_from_slice(&0xfffau16.to_le_bytes());
    if phy0 & ax::HE_PHY0_160 != 0 {
        b.extend_from_slice(&0xfffeu16.to_le_bytes()); // one stream at 160 MHz
        b.extend_from_slice(&0xfffeu16.to_le_bytes());
    }
    b.extend_from_slice(ppe);
    let mut e = vec![255, b.len() as u8];
    e.extend_from_slice(&b);
    e
}
/// HE operation: default PE 2, RTS threshold 1023, BSS colour 37.
fn he_op() -> Vec<u8> {
    let params: u32 = 2 | 1023 << 4 | 37 << 24;
    let mut e = vec![255, 7, 36];
    e.extend_from_slice(&params.to_le_bytes());
    e.extend_from_slice(&0xfffcu16.to_le_bytes());
    e
}
fn wmm_param() -> Vec<u8> {
    let mut e = vec![221, 24, 0x00, 0x50, 0xf2, 2, 1, 1, 0x80, 0];
    // ACI 0 BE, 1 BK, 2 VI, 3 VO (sent out of order on purpose).
    e.extend_from_slice(&[0x03 | 3 << 5, 0x43, 0x2f, 0x00]); // VO: AIFSN 3? (here 3), ECW 3/4, TXOP 47
    e.extend_from_slice(&[0x03, 0xa4, 0, 0]); // BE
    e.extend_from_slice(&[0x07 | 1 << 5, 0xa4, 0, 0]); // BK
    e.extend_from_slice(&[0x02 | 2 << 5, 0x43, 0x5e, 0x00]); // VI
    e
}
fn mu_edca() -> Vec<u8> {
    let mut e = vec![255, 14, 38, 0];
    e.extend_from_slice(&[0x08, 0xa4, 0xff]); // BE: AIFSN 8, ECW 4/10, timer 255
    e.extend_from_slice(&[0x28 | 0x0, 0xa4, 0xff]); // BK (ACI 1)
    e.extend_from_slice(&[0x45, 0x43, 0xff]); // VI
    e.extend_from_slice(&[0x62, 0x32, 0xff]); // VO
    e
}

fn beacon(channel: u8, elements: &[Vec<u8>]) -> Vec<u8> {
    let mut f = vec![0x80, 0x00, 0, 0];
    f.extend_from_slice(&w::BROADCAST);
    f.extend_from_slice(&AP);
    f.extend_from_slice(&AP);
    f.extend_from_slice(&[0, 0]);
    f.extend_from_slice(&[0; 8]);
    f.extend_from_slice(&100u16.to_le_bytes());
    f.extend_from_slice(&0x0401u16.to_le_bytes());
    f.extend_from_slice(&[0, 4, b'h', b'o', b'm', b'e']);
    f.extend_from_slice(&[1, 8, 0x8c, 0x12, 0x98, 0x24, 0xb0, 0x48, 0x60, 0x6c]);
    f.extend_from_slice(&[3, 1, channel]);
    for e in elements { f.extend_from_slice(e); }
    f
}

/// A Wi-Fi 6 AP on channel 36, 80 MHz wide around 42.
fn wifi6_80() -> w::Bss {
    w::parse_bss(&beacon(36, &[ht_cap(), ht_op(36, 1, 0), vht_cap(7 << 23 | 0x10 | 0x20 | 0x100), vht_op(1, 42, 0),
        he_cap(ax::HE_PHY0_40_80_5G, 0, 2 << 6, &[]), he_op(), wmm_param(), mu_edca()])).unwrap()
}

#[test]
fn beacon_elements_are_read() {
    let b = wifi6_80();
    let p = b.peer;
    let ht = p.ht.unwrap();
    assert_eq!(ht.rx_mcs[..2], [0xff, 0xff]);
    assert_eq!(ht.ampdu, 3 | 6 << 2);
    assert_eq!(p.ht_op.unwrap(), ax::HtOp { primary: 36, sco: 1, any_width: true, protection: 0 });
    assert_eq!(p.vht_op.unwrap(), ax::VhtOp { width: 1, seg0: 42, seg1: 0 });
    let he = p.he.unwrap();
    assert_eq!((he.rx_mcs_80, he.rx_mcs_160), (0xfffa, 0xffff));
    let op = p.he_op.unwrap();
    assert_eq!((op.bss_color(), op.default_pe(), op.rts_threshold()), (37, 2, 1023));
    let wmm = p.wmm.unwrap();
    assert_eq!(wmm.ac[0], ax::Edca { aifsn: 3, ecw_min: 4, ecw_max: 10, txop: 0 });
    assert_eq!(wmm.ac[1].aifsn, 7);
    assert_eq!(wmm.ac[2], ax::Edca { aifsn: 2, ecw_min: 3, ecw_max: 4, txop: 94 });
    assert_eq!(wmm.ac[3].txop, 47);
    let mu = p.mu_edca.unwrap();
    assert_eq!(mu.ac[0], (8, 0xa4, 0xff));
    assert_eq!(mu.ac[3], (2, 0x32, 0xff));
    // The channel still comes from the DS element; legacy fields are unchanged.
    assert_eq!(b.channel, 36);
    assert_eq!(b.basic_ofdm, 0b10101);
}

#[test]
fn agreed_mode_and_width() {
    let b = wifi6_80();
    // A legacy-only station (the default) keeps everything as before X1.
    let legacy = join::Target::of(&b, 36);
    assert_eq!(legacy.mode(), ax::Mode::LEGACY);
    let t = legacy.with_ours(x17());
    let m = t.mode();
    assert!(m.ht && m.vht && m.he && m.qos);
    assert_eq!(m.generation(), 6);
    assert_eq!((m.width, m.ctrl_pos), (Width::W80, 1)); // 36 is two below the centre 42
    // Primary 48 in the same 80 MHz: one above... no, 48 - 42 = 6: two above.
    let mut p = b.peer;
    p.ht_op.as_mut().unwrap().primary = 48;
    assert_eq!(ax::mode(&p, &x17(), 48, false).ctrl_pos, 5);
    // 160 MHz (new style): segment 1 is the centre of the whole channel.
    let mut p = b.peer;
    p.vht_op = Some(ax::VhtOp { width: 1, seg0: 42, seg1: 50 });
    p.vht.as_mut().unwrap().cap |= ax::VHT_WIDTH_160;
    let m = ax::mode(&p, &x17(), 36, false);
    assert_eq!((m.width, m.ctrl_pos), (Width::W160, 3));
    // Without 160 MHz on our side we stay on the primary's 80 MHz.
    let ours = Ours { max_width: Width::W80, ..x17() };
    let m = ax::mode(&p, &ours, 36, false);
    assert_eq!((m.width, m.ctrl_pos), (Width::W80, 1));
    // Old-style 160 (width 2, centre 50): the primary's 80 MHz is around 42.
    p.vht_op = Some(ax::VhtOp { width: 2, seg0: 50, seg1: 0 });
    assert_eq!(ax::mode(&p, &ours, 36, false).width, Width::W80);
    assert_eq!(ax::mode(&p, &ours, 36, false).ctrl_pos, 1);
    assert_eq!(ax::mode(&p, &x17(), 36, false).ctrl_pos, 3);
    // HT only, secondary below: 40 MHz with the control channel above.
    let ht = w::parse_bss(&beacon(40, &[ht_cap(), ht_op(40, 3, 0), wmm_param()])).unwrap();
    let m = ax::mode(&ht.peer, &x17(), 40, false);
    assert_eq!((m.ht, m.vht, m.he, m.width, m.ctrl_pos), (true, false, false, Width::W40, 4));
    // 2.4 GHz stays at 20 MHz, without VHT.
    let g = w::parse_bss(&beacon(6, &[ht_cap(), ht_op(6, 1, 0), he_cap(0, 0, 0, &[]), he_op(), wmm_param()])).unwrap();
    let m = ax::mode(&g.peer, &x17(), 6, true);
    assert_eq!((m.ht, m.vht, m.he, m.width), (true, false, true, Width::W20));
    // No WMM, no HT: an HT AP must do QoS.
    let n = w::parse_bss(&beacon(36, &[ht_cap(), ht_op(36, 1, 0)])).unwrap();
    assert_eq!(ax::mode(&n.peer, &x17(), 36, false), ax::Mode::LEGACY);
    // Firmware without STA_HE_CTXT_CMD: Wi-Fi 5.
    let m = ax::mode(&b.peer, &Ours { he_ctxt: 0, ..x17() }, 36, false);
    assert_eq!((m.vht, m.he), (true, false));    // An 11n-only SKU (no 11ac, no 11ax): HT at 40 MHz on a Wi-Fi 6 AP, no VHT
    // element offered and no VHT rates chosen.
    let n = Ours { vht: false, he_ctxt: 0, max_width: Width::W40, ..x17() };
    let m = ax::mode(&b.peer, &n, 36, false);
    assert_eq!((m.ht, m.vht, m.he, m.width), (true, false, false, Width::W40));
    let mut caps = [0u8; 128];
    let (_, all) = ax::own_elements(&b.peer, &n, false, &mut caps);
    assert!(w::elements(&caps[..all]).all(|(id, _)| id != 191), "no VHT capabilities element");
}

#[test]
fn association_request_offers_what_both_have() {
    let b = wifi6_80();
    let mut caps = [0u8; 128];
    let (c, all) = ax::own_elements(&b.peer, &x17(), false, &mut caps);
    let ids: Vec<(u8, usize)> = w::elements(&caps[..all]).map(|(id, body)| (id, body.len())).collect();
    assert_eq!(ids, [(45, 26), (191, 12), (255, 26), (221, 7)]);
    assert_eq!(c, 28 + 14 + 28);
    // HT: 40 MHz + SGI 40 on 5 GHz, two streams of MCS.
    let ht = u16::from_le_bytes([caps[2], caps[3]]);
    assert_ne!(ht & ax::HT_40, 0);
    assert_eq!(caps[5..7], [0xff, 0xff]);
    // HE: extension 35, 160 MHz offered, MCS 0..11 on two streams.
    let he = &caps[28 + 14..c];
    assert_eq!(he[2], ax::EXT_HE_CAP);
    assert_eq!(he[3 + 6] & (ax::HE_PHY0_40_80_5G | ax::HE_PHY0_160), ax::HE_PHY0_40_80_5G | ax::HE_PHY0_160);
    assert_eq!(u16::from_le_bytes([he[3 + 17], he[3 + 18]]), 0xfffa);
    // The frame: rates, RSN, capabilities, then WMM last.
    let mut frame = [0u8; 512];
    let n = w::assoc_request(&mut frame, &b, &STA, 1, &[], &caps[..c], &caps[c..all]);
    let ids: Vec<u8> = w::elements(&frame[28..n]).map(|(id, _)| id).collect();
    assert_eq!(ids, [0, 1, 45, 191, 255, 221]);
    // 2.4 GHz: no VHT, no 40 MHz, one stream when we have one antenna.
    let one = Ours { tx_antennas: 1, rx_antennas: 1, ..x17() };
    let g = w::parse_bss(&beacon(6, &[ht_cap(), ht_op(6, 0, 0), wmm_param()])).unwrap();
    let (_, all) = ax::own_elements(&g.peer, &one, true, &mut caps);
    let ids: Vec<u8> = w::elements(&caps[..all]).map(|(id, _)| id).collect();
    assert_eq!(ids, [45, 221]);
    assert_eq!(u16::from_le_bytes([caps[2], caps[3]]) & ax::HT_40, 0);
    assert_eq!(caps[5..7], [0xff, 0x00]);
    // Nothing at all for a legacy AP or a legacy station.
    let legacy = w::parse_bss(&beacon(36, &[])).unwrap();
    assert_eq!(ax::own_elements(&legacy.peer, &x17(), false, &mut caps), (0, 0));
    assert_eq!(ax::own_elements(&b.peer, &Ours::default(), false, &mut caps), (0, 0));
}

#[test]
fn station_mac_and_phy_commands() {
    let t = join::Target::of(&wifi6_80(), 36).with_ours(x17());
    let m = t.mode();
    let (flags, mask) = ax::station_flags(&m, &t.peer, &t.ours);
    assert_eq!(flags, 7 << 19 | 6 << 23 | 2 << 26 | 1 << 28); // 1 MiB A-MPDU, 8 µs, 80 MHz, MIMO2
    assert_eq!(mask, 0xf << 19 | 7 << 23);
    let mut c = [0u8; join::ADD_STA_BYTES];
    join::add_station_flags(&mut c, &AP, true, (flags, mask));
    assert_eq!(u32::from_le_bytes(c[20..24].try_into().unwrap()), flags);
    assert_eq!(u32::from_le_bytes(c[24..28].try_into().unwrap()), 3 << 26 | 3 << 28 | mask);
    // The pre-association entry carries no HT flags.
    join::add_station(&mut c, &AP, false);
    assert_eq!(u32::from_le_bytes(c[20..24].try_into().unwrap()), 0);

    // MAC context after association: QoS + TGN, 11ax filter, the AP's EDCA.
    let mut mac = [0u8; join::MAC_CONTEXT_BYTES];
    join::mac_context(&mut mac, join::ACTION_MODIFY, &STA, &t, Some(5));
    let word = |at: usize| u32::from_le_bytes(mac[at..at + 4].try_into().unwrap());
    assert_eq!(word(56), ax::MAC_QOS_FLG_UPDATE_EDCA | ax::MAC_QOS_FLG_TGN);
    assert_ne!(word(52) & ax::MAC_FILTER_IN_11AX, 0);
    assert_eq!(word(40), 0);
    // VI (FIFO 3): ECW 3/4 -> CW 7/15, AIFSN 2, TXOP 94*32.
    let vi = 60 + 3 * 8;
    assert_eq!(mac[vi..vi + 8], [7, 0, 15, 0, 2, 1 << 3, (94u16 * 32) as u8, ((94u16 * 32) >> 8) as u8]);
    // Before association the MAC context is the legacy one.
    join::mac_context(&mut mac, join::ACTION_ADD, &STA, &t, None);
    assert_eq!(u32::from_le_bytes(mac[56..60].try_into().unwrap()), 0);
    // Mixed-mode protection.
    let mut p = t.peer;
    p.ht_op.as_mut().unwrap().protection = 3;
    assert_eq!(ax::mac_flags(&m, &p).0, ax::MAC_PROT_FLG_HT_PROT | ax::MAC_PROT_FLG_FAT_PROT);

    // PHY context widened in place: v1 (byte channel) and the 6 GHz-sized layout.
    let mut phy = [0u8; join::PHY_CONTEXT_BYTES];
    let n = join::phy_context_wide(&mut phy, join::ACTION_MODIFY, 36, Width::W80, 1, false, false, None);
    assert_eq!(n, join::PHY_CONTEXT_V1_BYTES);
    assert_eq!(phy[8..12], [0, 36, 2, 1]);
    join::phy_context_wide(&mut phy, join::ACTION_MODIFY, 36, Width::W160, 3, true, true, None);
    assert_eq!(phy[8..16], [36, 0, 0, 0, 0, 3, 3, 0]);
    assert_eq!(u32::from_le_bytes(phy[16..20].try_into().unwrap()), 1); // 5 GHz LMAC
}

fn h(c: &[u8], at: usize) -> u16 { u16::from_le_bytes([c[at], c[at + 1]]) }

#[test]
fn rate_control_names_mcs_per_stream() {
    let t = join::Target::of(&wifi6_80(), 36).with_ours(x17());
    let mut c = [0u8; tlc::CONFIG_BYTES];
    tlc::config(&mut c, 4, &t);
    assert_eq!((c[4], c[5], c[6], c[7]), (2, ax::TLC_MODE_HE, 3, 0)); // 80 MHz, HE, chains A+B, no SGI in HE
    assert_eq!(h(&c, 8), ax::TLC_FLAGS_STBC | ax::TLC_FLAGS_LDPC);
    assert_eq!(h(&c, 10), 0x0ff0); // OFDM legacy rates
    assert_eq!((h(&c, 12), h(&c, 14), h(&c, 18), h(&c, 20)), (0x0fff, 0, 0x0fff, 0));
    assert_eq!(h(&c, 24), 3895);
    // 160 MHz on HE: the AP gives one stream there.
    let b = w::parse_bss(&beacon(36, &[ht_cap(), ht_op(36, 1, 0), vht_cap(7 << 23 | ax::VHT_WIDTH_160), vht_op(1, 42, 50),
        he_cap(ax::HE_PHY0_40_80_5G | ax::HE_PHY0_160, 0, 0, &[]), he_op(), wmm_param()])).unwrap();
    let t = join::Target::of(&b, 36).with_ours(x17());
    tlc::config(&mut c, 4, &t);
    assert_eq!(c[4], 3);
    assert_eq!((h(&c, 12), h(&c, 14), h(&c, 18), h(&c, 20)), (0x0fff, 0x0fff, 0x0fff, 0));
    // Wi-Fi 5 (VHT) at 80 MHz, firmware without HE, v3 layout: two widths per stream.
    let t = join::Target::of(&wifi6_80(), 36).with_ours(Ours { he_ctxt: 0, tlc: 3, ..x17() });
    tlc::config(&mut c, 3, &t);
    assert_eq!((c[5], c[7]), (ax::TLC_MODE_VHT, 0)); // v3 has no SGI byte
    assert_eq!((h(&c, 12), h(&c, 14), h(&c, 16), h(&c, 18)), (0x03ff, 0, 0x03ff, 0));
    assert_eq!(h(&c, 20), 3895);
    // Legacy: exactly what W5g sent.
    let t = join::Target::of(&wifi6_80(), 36);
    tlc::config(&mut c, 4, &t);
    assert_eq!((c[4], c[5], c[6], h(&c, 12), h(&c, 24)), (0, 0, 1, 0, 2312));
}

#[test]
fn he_context_command() {
    let b = wifi6_80();
    let mut c = [0u8; ax::HE_CTXT_V3_BYTES];
    let n = ax::he_context(&mut c, 0, &b.peer, &x17());
    assert_eq!(n, ax::HE_CTXT_V2_BYTES);
    let flags = u32::from_le_bytes(c[4..8].try_into().unwrap());
    assert_eq!(flags, 1 << 13 | 1 << 7 | 1 << 8 | 1 << 12); // not ACK-enabled, 32-bit BA, packet ext, MU EDCA
    assert_eq!(c[1], 8);
    // Nominal padding 16 µs: no extension below the high threshold, BPSK above.
    assert!(c[24..40].chunks(2).all(|th| th == [7, 0]));
    assert_eq!((c[40], c[41], u16::from_le_bytes([c[42], c[43]])), (37, 2, 1023));
    // MU EDCA: BE goes to firmware AC 1, VO to 3 (cwmin, cwmax, aifsn, timer).
    assert_eq!(c[56..64], [4, 0, 10, 0, 8, 0, 255, 0]);
    assert_eq!(c[72..80], [2, 0, 3, 0, 2, 0, 255, 0]);
    // Without the MBSSID API: version 1's size.
    assert_eq!(ax::he_context(&mut c, 0, &b.peer, &Ours { he_mbssid: false, ..x17() }), ax::HE_CTXT_V1_BYTES);
    // Version 3: thresholds for five widths, the rest four bytes later.
    let n = ax::he_context(&mut c, 0, &b.peer, &Ours { he_ctxt: 3, ..x17() });
    assert_eq!(n, ax::HE_CTXT_V3_BYTES);
    assert_eq!(c[44], 37);
    // No HE operation element: no command.
    let mut p = b.peer;
    p.he_op = None;
    assert_eq!(ax::he_context(&mut c, 0, &p, &x17()), 0);
}

/// Bits written least significant first, as the PPE thresholds field holds them.
fn bits(fields: &[(u32, u32)]) -> Vec<u8> {
    let mut out = vec![0u8; 8];
    let mut pos = 0;
    for &(v, n) in fields {
        for i in 0..n { if v >> i & 1 != 0 { out[pos / 8] |= 1 << (pos % 8); } pos += 1; }
    }
    out.truncate(pos.div_ceil(8));
    out
}

#[test]
fn ppe_thresholds_are_read() {
    // One stream (nss-1 = 0), RU bitmap 0b0101: widths 0 and 2 have thresholds.
    let ppe = bits(&[(0, 3), (0b0101, 4), (2, 3), (5, 3), (7, 3), (1, 3)]);
    let he = ax::He { mac: [0; 6], phy: { let mut p = [0u8; 11]; p[6] = ax::HE_PHY6_PPE_PRESENT; p }, rx_mcs_80: 0xfffa, tx_mcs_80: 0xfffa,
        rx_mcs_160: 0xffff, tx_mcs_160: 0xffff, ppe: { let mut a = [0u8; ax::PPE_MAX]; a[..ppe.len()].copy_from_slice(&ppe); a }, ppe_len: ppe.len() as u8 };
    let t = ax::packet_extension(&he).unwrap();
    assert_eq!(t[0][0], [5, 2]); // [PPET8 = low, PPET16 = high]
    assert_eq!(t[0][1], [7, 7]);
    assert_eq!(t[0][2], [1, 7]);
    assert_eq!(t[1][0], [7, 7]);
    // And through the element parser, after the MCS maps.
    let b = w::parse_bss(&beacon(36, &[ht_cap(), ht_op(36, 1, 0), he_cap(ax::HE_PHY0_40_80_5G, ax::HE_PHY6_PPE_PRESENT, 0, &ppe), he_op(), wmm_param()])).unwrap();
    assert_eq!(ax::packet_extension(&b.peer.he.unwrap()).unwrap()[0][0], [5, 2]);
}

#[test]
fn qos_data_frames() {
    // IPv4 with DSCP 46 (EF): voice, TID 6.
    let mut e = vec![0u8; 14 + 20];
    e[..6].copy_from_slice(&AP);
    e[6..12].copy_from_slice(&STA);
    e[12..14].copy_from_slice(&[0x08, 0x00]);
    e[14] = 0x45;
    e[15] = 46 << 2;
    assert_eq!(w::user_priority(&e), 6);
    let mut out = [0u8; 128];
    let (hdr, total) = w::from_ethernet_qos(&e, &AP, 9, 6, &mut out).unwrap();
    assert_eq!((hdr, total), (26, 26 + 6 + 22));
    assert_eq!(u16::from_le_bytes([out[0], out[1]]), w::FC_QOS_DATA | w::FC_TO_DS);
    assert_eq!(out[24], 6);
    assert_eq!(w::data_header_len(&out[..total]), Some(26));
    // IPv6 traffic class CS1: background; ARP: best effort.
    let mut v6 = vec![0u8; 14 + 40];
    v6[12..14].copy_from_slice(&[0x86, 0xdd]);
    v6[14] = 0x60 | (8 >> 2);
    v6[15] = (8 & 3) << 6;
    assert_eq!(w::user_priority(&v6), 1);
    e[12..14].copy_from_slice(&[0x08, 0x06]);
    assert_eq!(w::user_priority(&e), 0);
}

/// The x17's firmware has the command versions X1 builds for.
#[test]
fn x17_firmware_versions() {
    let Some(dir) = std::env::var_os("SEXOS_WIFI_FIRMWARE_DIR") else { return };
    let file = std::fs::read(std::path::Path::new(&dir).join("iwlwifi-ty-a0-gf-a0-77.ucode")).unwrap();
    let fw = sex_wifi::ucode::Firmware::parse(&file).unwrap();
    let v = |c| fw.command_version(sex_wifi::hcmd::DATA_PATH_GROUP, c).map(|v| v.version);
    assert_eq!(v(ax::STA_HE_CTXT_CMD), Some(2));
    assert_eq!(v(tlc::TLC_MNG_CONFIG_CMD), Some(4));
    let f = sex_wifi::init::Features::of(&fw);
    assert!(f.api(sex_wifi::ucode::API_MBSSID_HE));
    assert_eq!((f.tx_antennas(), f.rx_antennas()), (3, 3));
}
