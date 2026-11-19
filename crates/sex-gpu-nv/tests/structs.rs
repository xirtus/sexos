// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Byte layouts GSP-RM reads, pinned against the C structs (offsets printed by
//! compiling nouveau's MIT copies of the 535.113.01 headers), and the queues.
mod common;
use common::*;
use sex_gpu_nv::layout::{self, FbLayout, SysmemRefs};
use sex_gpu_nv::msgq::{self, Queues};
use sex_gpu_nv::{libos, radix3, rpc, Dma};

#[test]
fn fb_layout_8gib_ga104() {
    // 8 GiB, display engine says no workspace yet, real 535 sizes.
    let fb = 8u64 << 30;
    let l = FbLayout::compute(fb, Some(0), 0x5000, 0x2448000).unwrap();
    assert_eq!(l.vga_workspace.addr, fb - 0x10_0000);
    assert_eq!(l.frts, layout::Range { addr: fb - 0x20_0000, size: 0x10_0000 });
    assert_eq!(l.boot.addr, l.frts.addr - 0x5000);
    assert_eq!(l.elf.addr % 0x1_0000, 0);
    assert!(l.elf.end() <= l.boot.addr);
    assert_eq!(layout::wpr_heap_size(fb), (20 + 8 + 1 + 96) << 20);
    assert!(l.heap.size >= layout::wpr_heap_size(fb) - 0x10_0000 && l.heap.end() <= l.elf.addr);
    assert_eq!(l.wpr2.addr % 0x10_0000, 0);
    assert_eq!(l.wpr2.end(), l.frts.end());
    assert_eq!(l.non_wpr_heap.end(), l.wpr2.addr);
    assert_eq!(l.wpr_end(), fb - 0x10_0000);
    // Firmware's VGA workspace in the top MiB (bit 3 valid).
    let reg = (((fb - 0x8_0000) >> 8) as u32) | 0x8;
    assert_eq!(layout::vga_workspace_addr(fb, Some(reg)), fb - 0x8_0000);
    // A workspace below the top MiB is ignored in favour of the top 128 KiB.
    let low = (((fb - 0x40_0000) >> 8) as u32) | 0x8;
    assert_eq!(layout::vga_workspace_addr(fb, Some(low)), fb - 0x2_0000);
    assert!(FbLayout::compute(256 << 20, None, 0x5000, 0x2448000).is_err());
}

#[test]
fn wpr_meta_offsets_match_c() {
    let l = FbLayout::compute(8 << 30, None, 0x5000, 0x2448000).unwrap();
    let s = SysmemRefs { radix3_elf: 0x1000, bootloader: 0x2000, bootloader_size: 0x5000, bootloader_code_offset: 0x1000, bootloader_data_offset: 0x1800, bootloader_manifest_offset: 0x2500, signature: 0x3000, signature_size: 0x1000 };
    let mut m = [0xffu8; 256];
    layout::write_wpr_meta(&mut m, &l, &s);
    // offsetof(GspFwWprMeta, ...) from the C compiler:
    let at = |o| r64(&m, o);
    assert_eq!(at(0), 0xdc3aae21371a60b3);
    assert_eq!(at(8), 1);
    assert_eq!((at(16), at(24)), (0x1000, 0x2448000));
    assert_eq!((at(32), at(40), at(48), at(56), at(64)), (0x2000, 0x5000, 0x1000, 0x1800, 0x2500));
    assert_eq!((at(72), at(80)), (0x3000, 0x1000));
    assert_eq!((at(88), at(96), at(104)), (l.non_wpr_heap.addr, l.non_wpr_heap.addr, 0x10_0000));
    assert_eq!((at(112), at(120), at(128), at(136), at(144)), (l.wpr2.addr, l.heap.addr, l.heap.size, l.elf.addr, l.boot.addr));
    assert_eq!((at(152), at(160), at(168), at(176)), (l.frts.addr, l.frts.size, l.wpr_end(), 8 << 30));
    assert_eq!((at(184), at(192), at(200), at(248)), (l.vga_workspace.addr, l.vga_workspace.size, 0, 0));
    assert!(m[200..256].iter().all(|&b| b == 0));
}

#[test]
fn libos_args_and_rmargs() {
    assert_eq!(libos::id8("LOGINIT"), 0x004c_4f47_494e_4954);
    assert_eq!(libos::id8("RMARGS"), u64::from_be_bytes(*b"\0\0RMARGS"));
    let mut args = [0u8; 128];
    libos::write_region(&mut args, 1, "LOGINTR", 0x1234_0000, 0x10000);
    // sizeof(LibosMemoryRegionInitArgument) 32, kind @24, loc @25
    assert_eq!((r64(&args, 32), r64(&args, 40), r64(&args, 48), args[56], args[57]), (libos::id8("LOGINTR"), 0x1234_0000, 0x10000, 1, 1));
    let mut buf = vec![0u8; 0x10000];
    libos::init_log_buffer(&mut Dma::new(&mut buf, 0x8000_0000));
    assert_eq!((r64(&buf, 0), r64(&buf, 8), r64(&buf, 8 + 15 * 8)), (0, 0x8000_0000, 0x8000_f000));
    let mut rm = [0xffu8; 4096];
    libos::write_rmargs(&mut rm, 0x4_0000_0000, 129, 0x1000, 0x41000);
    // GSP_ARGUMENTS_CACHED: sharedMemPhysAddr @0, pageTableEntryCount @8, cmdQueueOffset @16, statQueueOffset @24, srInit @48
    assert_eq!((r64(&rm, 0), r32(&rm, 8), r64(&rm, 16), r64(&rm, 24), r32(&rm, 48), rm[56]), (0x4_0000_0000, 129, 0x1000, 0x41000, 0, 0));
}

#[test]
fn radix3_tables() {
    let (mut a, mut b, mut c) = (vec![0u8; 4096], vec![0u8; 4096], vec![0u8; radix3::lvl2_len(0x2448000)]);
    assert_eq!(c.len(), 0x2448000 / 4096 * 8 / 4096 * 4096 + 4096 * ((0x2448000 / 4096 * 8) % 4096 != 0) as usize);
    let (l0, l1, l2) = (&mut Dma::new(&mut a, 0x10_0000), &mut Dma::new(&mut b, 0x20_0000), &mut Dma::new(&mut c, 0x30_0000));
    radix3::build(l0, l1, l2, 0x2448000, |i| 0x1_0000_0000 + (i as u64) * 4096).unwrap();
    assert_eq!(r64(&a, 0), 0x20_0000);
    assert_eq!((r64(&b, 0), r64(&b, 8)), (0x30_0000, 0x30_1000));
    let pages = 0x2448000 / 4096;
    assert_eq!(r64(&c, (pages - 1) * 8), 0x1_0000_0000 + (pages as u64 - 1) * 4096);
}

#[test]
fn rpc_payloads() {
    let mut s = [0u8; rpc::SYSTEM_INFO_SIZE];
    rpc::system_info(&mut s, &rpc::SystemInfo { bar0: 0xa000_0000, bar1: 0x40_0000_0000, bar_inst: 0x42_0000_0000, bus: 1, devfn: 0, max_user_va: 0x7fff_ffff_f000 });
    // GspSystemInfo offsets: phys @0/8/16, BDF @24, maxUserVa @56, mirror @64/68, size 664
    assert_eq!((r64(&s, 0), r64(&s, 8), r64(&s, 16), r64(&s, 24), r64(&s, 56), r32(&s, 64), r32(&s, 68)), (0xa000_0000, 0x40_0000_0000, 0x42_0000_0000, 0x100, 0x7fff_ffff_f000, 0x88000, 0x1000));
    let mut reg = [0u8; 512];
    let n = rpc::registry(&mut reg, rpc::DEFAULT_REGISTRY).unwrap();
    // PACKED_REGISTRY_TABLE {size, numEntries} + 16-byte entries {nameOffset, type@4, data@8, length@12} + names
    let names = "RMSecBusResetEnable\0RMForcePcieConfigSave\0RMDevidCheckIgnore\0";
    assert_eq!(n, 8 + 3 * 16 + names.len());
    assert_eq!((r32(&reg, 0) as usize, r32(&reg, 4)), (n, 3));
    assert_eq!((r32(&reg, 8), reg[12], r32(&reg, 16), r32(&reg, 20)), (56, 1, 1, 4));
    assert_eq!(&reg[56..n], names.as_bytes());
    assert_eq!(r32(&reg, 8 + 16) as usize, 56 + "RMSecBusResetEnable\0".len());
}

#[test]
fn queue_roundtrip_with_wrap() {
    let l = msgq::shared_layout();
    assert_eq!((l.pte_count, l.ptes_size, l.cmdq_off, l.msgq_off, l.total), (129, 4096, 0x1000, 0x41000, 0x81000));
    assert_eq!(msgq::ENTRIES, 63);
    let mut mem = vec![0u8; l.total];
    let bus = 0x9_0000_0000;
    msgq::init_shared(&mut Dma::new(&mut mem, bus)).unwrap();
    assert_eq!((r64(&mem, 0), r64(&mem, 128 * 8)), (bus, bus + 128 * 4096));
    // msgqTxHeader of the command queue
    let q = l.cmdq_off;
    assert_eq!([r32(&mem, q + 4), r32(&mem, q + 8), r32(&mem, q + 12), r32(&mem, q + 16), r32(&mem, q + 20), r32(&mem, q + 24), r32(&mem, q + 28)], [0x40000, 0x1000, 63, 0, 1, 32, 0x1000]);
    let mut qs = Queues::new(l);
    let mut scratch = vec![0u8; msgq::MAX_MSG];
    let payload: Vec<u8> = (0..5000u32).map(|i| i as u8).collect();
    // Fill until full, with GSP consuming: move GSP's read pointer as it would.
    for i in 0..100u32 {
        let r = r32(&mem, l.msgq_off + 32);
        if qs.cmdq_free(&mem) < 2 { w32(&mut mem, l.msgq_off + 32, (r + 2) % 63); }
        qs.push(&mut mem, &mut scratch, 72, &payload).unwrap();
        // The element in the queue sums (XOR-fold) to zero.
        let w = r32(&mem, q + 16);
        let start = (w + 63 - 2) % 63;
        if start + 2 <= 63 {
            let at = q + 0x1000 + start as usize * 0x1000;
            assert_eq!(msgq::checksum(&mem[at..at + 0x2000]), 0, "element {i}");
            assert_eq!((r32(&mem, at + 36), r32(&mem, at + 40), r32(&mem, at + 48 + 12), r32(&mem, at + 48 + 8)), (i, 2, 72, 32 + 5000));
        }
    }
    // GSP side: write two messages into the message queue, the second wrapping.
    let mq = l.msgq_off;
    w32(&mut mem, mq + 16, 0);
    w32(&mut mem, q + 32, 62); // our read pointer
    let put = |mem: &mut Vec<u8>, slot: u32, function: u32, body: &[u8]| {
        let mut e = vec![0u8; 0x2000];
        w32(&mut e, 40, 2);
        w32(&mut e, 48, 0x0300_0000); e[52..56].copy_from_slice(b"VRPC"); w32(&mut e, 56, (32 + body.len()) as u32); w32(&mut e, 60, function);
        e[80..80 + body.len()].copy_from_slice(body);
        for p in 0..2u32 { let at = mq + 0x1000 + ((slot + p) % 63) as usize * 0x1000; mem[at..at + 0x1000].copy_from_slice(&e[p as usize * 0x1000..][..0x1000]); }
    };
    let body: Vec<u8> = (0..6000u32).map(|i| (i * 3) as u8).collect();
    put(&mut mem, 62, 0x1002, &body);
    w32(&mut mem, mq + 16, 1);
    let mut out = vec![0u8; 8192];
    let m = qs.recv(&mut mem, &mut out).unwrap().unwrap();
    assert_eq!((m.function, m.len), (0x1002, 6000));
    assert_eq!(&out[..6000], &body[..]);
    assert_eq!(r32(&mem, q + 32), 1);
    assert!(qs.recv(&mut mem, &mut out).unwrap().is_none());
    // A half-written message (one of two pages) is left for later.
    put(&mut mem, 1, 0x1001, &[]);
    w32(&mut mem, mq + 16, 2);
    w32(&mut mem, mq + 0x1000 + 0x1000 + 56, 32 + 5000);
    assert!(qs.recv(&mut mem, &mut out).unwrap().is_none());
    w32(&mut mem, mq + 0x1000 + 0x1000 + 56, 32);
    let m = qs.recv(&mut mem, &mut out).unwrap().unwrap();
    assert_eq!((m.function, m.len), (0x1001, 0));
}
