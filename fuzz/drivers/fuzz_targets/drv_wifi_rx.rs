// SPDX-License-Identifier: GPL-3.0-only
//! Intel Wi-Fi (AX201/AX210/Killer 1675 on the x17): everything the firmware and
//! the air hand the driver. The input is an RX buffer as the firmware writes it:
//! packets (notifications, command answers, received MPDUs) that go through every
//! parser the driver uses, and each received frame through the 802.11, RSN,
//! EAPOL, CCMP and SAE code a hostile access point reaches.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_wifi::{ccmp, devices::Family, eapol, hcmd, ieee80211 as w, link, rsna, sae, scan, tlc, tx};
use std::sync::OnceLock;

fn frame(f: &[u8]) {
    let _ = w::parse_bss(f);
    let _ = w::parse_auth(f);
    let _ = w::parse_assoc_response(f);
    let _ = w::parse_disconnect(f);
    for (id, body) in w::elements(f) { if id == 48 { let _ = w::parse_rsn(body); } }
    let _ = w::parse_rsn(f);
    let _ = eapol::parse(f);
    let me = [0x02, 0xaa, 0xbb, 0xcc, 0xdd, 0xee];
    if let Some(e) = rsna::eapol_of(f, &me, &[0x02, 0x11, 0x22, 0x33, 0x44, 0x55]) { let _ = eapol::parse(e); }
    let _ = ccmp::parse_ccmp_header(f);
    let mut aad = [0u8; 30];
    let mut nonce = [0u8; 13];
    let _ = ccmp::aad_and_nonce(f, 7, &mut aad, &mut nonce);
    for header_len in [24usize, 26, 30, 32] {
        let mut copy = f.to_vec();
        let _ = ccmp::decrypt_frame(&[0x42; 16], &mut copy, header_len);
    }
    // An access point's SAE commit and confirm (WPA3).
    if f.len() >= 98 && f[0] == 19 {
        static PT: OnceLock<p256::AffinePoint> = OnceLock::new();
        let pt = *PT.get_or_init(|| sae::derive_pt(b"sexos", b"password", None));
        let mut s = sae::Sae::new(pt, &[3; 32], &[5; 32]).expect("sae");
        if s.process_commit(f).is_ok() { let _ = s.check_confirm(&f[f.len().min(98)..]); }
    }
}

fn run(d: &[u8]) {
    for p in hcmd::packets(d) {
        let _ = (p.is_response(), p.group_id());
        let _ = hcmd::parse_alive(&p);
        let _ = hcmd::parse_error(&p);
        let _ = tlc::parse_update(&p);
        let _ = tx::parse_response(&p);
        let _ = link::parse_missed_beacons(&p);
        for family in [Family::Gen22000, Family::Ax210] {
            let mut scratch = [0u8; 4096];
            if let Some(r) = scan::parse_mpdu(&p, family, &mut scratch) { frame(r.frame); }
        }
    }
    frame(d);
}

/// RX buffers with one packet of each kind the firmware sends, plus a beacon.
fn seeds() -> Vec<Vec<u8>> {
    let packet = |group: u8, code: u8, body: &[u8]| {
        // iwl_rx_packet: len_n_flags (u32), then the command header (cmd, group, seq).
        let mut v = ((body.len() as u32 + 4) & 0x3fff).to_le_bytes().to_vec();
        v.extend_from_slice(&[code, group, 0, 0x80]);
        v.extend_from_slice(body);
        while v.len() % 64 != 0 { v.push(0); }
        v
    };
    let mut beacon = vec![0x80, 0, 0, 0];
    beacon.extend_from_slice(&[0xff; 6]);
    beacon.extend_from_slice(&[0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
    beacon.extend_from_slice(&[0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
    beacon.extend_from_slice(&[0; 2]);
    beacon.extend_from_slice(&[0; 8]);
    beacon.extend_from_slice(&[100, 0, 0x11, 0x04]);
    beacon.extend_from_slice(&[0, 5]);
    beacon.extend_from_slice(b"sexos");
    beacon.extend_from_slice(&[3, 1, 6]);
    beacon.extend_from_slice(&[48, 20, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 4, 1, 0, 0, 0x0f, 0xac, 2, 0, 0]);
    let mut mpdu = vec![0u8; 48];
    mpdu[0..2].copy_from_slice(&(beacon.len() as u16).to_le_bytes());
    mpdu.extend_from_slice(&beacon);
    let mut all = Vec::new();
    for (g, c) in [(0u8, 1u8), (0, 0xc1), (0, 0x2a), (4, 0xf5), (0, 0x1c), (0, 0xa2)] { all.extend(packet(g, c, &[0x11; 96])); }
    all.extend(packet(0, 0xc1, &mpdu));
    vec![all, beacon, packet(0, 0xc1, &mpdu)]
}

sexos_driver_fuzz::driver_target!(run, seeds);
