// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Decoding a hand-built FW_READY mailbox: the fixed part, then the extended
//! records (a compiler-info record to skip, then the window list) and the
//! mailbox layout derived from it.

use sex_sof::ipc3::*;

fn put(b: &mut [u8], o: usize, v: u32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

/// Bytes the firmware leaves in the DSP-to-host mailbox after boot.
fn mailbox() -> [u8; 1024] {
    let mut m = [0u8; 1024];
    // sof_ipc_fw_ready
    put(&mut m, 0, FW_READY_SIZE as u32);
    put(&mut m, 4, FW_READY);
    let v = 24; // sof_ipc_fw_version
    put(&mut m, v, FW_VERSION_SIZE as u32);
    m[v + 4..v + 6].copy_from_slice(&2u16.to_le_bytes());
    m[v + 6..v + 8].copy_from_slice(&14u16.to_le_bytes());
    m[v + 8..v + 10].copy_from_slice(&1u16.to_le_bytes());
    m[v + 10..v + 12].copy_from_slice(&7u16.to_le_bytes());
    m[v + 12..v + 23].copy_from_slice(b"Sep 30 2026");
    m[v + 24..v + 32].copy_from_slice(b"12:34:56");
    m[v + 34..v + 39].copy_from_slice(b"v2.14");
    put(&mut m, v + 40, abi_version(3, 29, 1));
    put(&mut m, v + 44, 0xdead_beef);
    m[84..92].copy_from_slice(&0x0000_0001_0000_0003u64.to_le_bytes());

    let mut o = FW_READY_SIZE;
    // A compiler-info record the walk must step over (24 bytes).
    put(&mut m, o, 24);
    put(&mut m, o + 4, FW_READY);
    put(&mut m, o + 8, EXT_CC_INFO);
    o += 24;
    // The window list with three regions.
    let n = 3;
    let size = 16 + n * WINDOW_ELEM_SIZE;
    put(&mut m, o, size as u32);
    put(&mut m, o + 4, FW_READY);
    put(&mut m, o + 8, EXT_WINDOW);
    put(&mut m, o + 12, n as u32);
    let elems = [
        // region, id, flags, size, offset
        (REGION_UPBOX, 0, 0, 0x1000, 0x6000),
        (REGION_DOWNBOX, 1, 0, 0x1000, 0),
        (REGION_STREAM, 3, 0, 0x1000, 0),
    ];
    for (i, e) in elems.iter().enumerate() {
        let q = o + 16 + i * WINDOW_ELEM_SIZE;
        put(&mut m, q, WINDOW_ELEM_SIZE as u32);
        put(&mut m, q + 4, e.0);
        put(&mut m, q + 8, e.1);
        put(&mut m, q + 12, e.2);
        put(&mut m, q + 16, e.3);
        put(&mut m, q + 20, e.4);
    }
    // Whatever follows has no FW_READY header and ends the walk.
    m
}

#[test]
fn fw_ready_fixed_part() {
    let m = mailbox();
    let r = FwReady::parse(&m).unwrap();
    assert_eq!(r.version.major, 2);
    assert_eq!(r.version.minor, 14);
    assert_eq!(r.version.micro, 1);
    assert_eq!(r.version.build, 7);
    assert_eq!(r.version.date_str(), "Sep 30 2026");
    assert_eq!(r.version.time_str(), "12:34:56");
    assert_eq!(r.version.tag_str(), "v2.14");
    assert_eq!(r.version.abi_version, abi_version(3, 29, 1));
    assert!(abi_compatible(r.version.abi_version));
    assert_eq!(r.version.src_hash, 0xdead_beef);
    assert_eq!(r.flags, 0x0000_0001_0000_0003);
    assert_eq!(r.dspbox_size, 0);
}

#[test]
fn fw_ready_rejects_other_messages() {
    let mut m = mailbox();
    put(&mut m, 4, GLB_REPLY);
    assert_eq!(FwReady::parse(&m), Err(Error::UnexpectedCmd(GLB_REPLY)));
    assert!(matches!(FwReady::parse(&mailbox()[..100]), Err(Error::Truncated { .. })));
}

#[test]
fn ext_data_walk() {
    let m = mailbox();
    let kinds: Vec<u32> = ExtData::new(&m[FW_READY_SIZE..]).map(|i| i.unwrap().ext_type).collect();
    assert_eq!(kinds, [EXT_CC_INFO, EXT_WINDOW]);
}

#[test]
fn window_list_and_layout() {
    let m = mailbox();
    let w = Windows::from_ext_data(&m[FW_READY_SIZE..]).unwrap();
    assert_eq!(w.count, 3);
    let up = w.find(REGION_UPBOX).unwrap();
    assert_eq!((up.id, up.size, up.offset), (0, 0x1000, 0x6000));
    assert!(w.find(REGION_DEBUG).is_none());

    // cAVS: SRAM window N sits at 0x80000 + N * 0x20000 in the DSP BAR.
    let base = |id: u32| if id < 4 { Some(0x80000 + id * 0x20000) } else { None };
    let l = w.layout(base).unwrap();
    assert_eq!(l.dsp_box, (0x86000, 0x1000));
    assert_eq!(l.host_box, (0xa0000, 0x1000));
    assert_eq!(l.stream_box, (0xe0000, 0x1000));
    assert_eq!(l.debug_box, (0, 0));

    // Without a host-to-DSP mailbox there is nothing to send requests through.
    let only_up = |id: u32| if id == 0 { Some(0x80000) } else { None };
    assert_eq!(w.layout(only_up), Err(Error::Invalid("mailbox window missing")));
}

#[test]
fn bad_ext_record_size_stops_the_walk() {
    let mut m = mailbox();
    put(&mut m, FW_READY_SIZE, 4000); // larger than the mailbox
    let mut it = ExtData::new(&m[FW_READY_SIZE..]);
    assert_eq!(it.next(), Some(Err(Error::BadSize(4000))));
    assert_eq!(it.next(), None);
    assert!(Windows::from_ext_data(&m[FW_READY_SIZE..]).is_err());
}
