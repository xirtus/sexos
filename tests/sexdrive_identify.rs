// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
// Host tests for SexDrive's NVMe Identify rules: which drives and namespaces it may
// use, and how much one command may carry. Run by scripts/sexdrive_dma_host_gate.sh.
#[path = "../apps/sexdrive/src/identify.rs"]
mod identify;
use identify::*;

fn namespace_page(sectors: u64, flbas: u8, formats: &[(u16, u8)]) -> Vec<u8> {
    let mut ns = vec![0u8; 4096];
    ns[0..8].copy_from_slice(&sectors.to_le_bytes());
    ns[26] = flbas;
    for (i, &(ms, lbads)) in formats.iter().enumerate() {
        let at = 128 + 4 * i;
        ns[at..at + 2].copy_from_slice(&ms.to_le_bytes());
        ns[at + 2] = lbads;
    }
    ns
}

#[test]
fn a_512_byte_namespace_is_used_whole() {
    // A Samsung PM9A1 1 TB as shipped: format 0 = 512 B, format 1 = 4 KiB.
    let ns = namespace_page(2_000_409_264, 0, &[(0, 9), (0, 12)]);
    assert_eq!(namespace(&ns), Ok(2_000_409_264));
}

#[test]
fn a_4k_namespace_is_refused_before_any_io() {
    let ns = namespace_page(250_000_000, 1, &[(0, 9), (0, 12)]);
    let r = namespace(&ns).unwrap_err();
    assert_eq!((r.reason, r.lba_bytes, r.fix), ("sector_size", 4096, "nvme_format_lbaf_512"));
}

#[test]
fn metadata_formats_are_refused() {
    // 512 + 8 bytes protection information, extended (FLBAS bit 4) and separate.
    let inline = namespace_page(1000, 0x10 | 2, &[(0, 9), (0, 12), (8, 9)]);
    assert_eq!(namespace(&inline).unwrap_err().reason, "metadata_inline");
    let separate = namespace_page(1000, 2, &[(0, 9), (0, 12), (8, 9)]);
    let r = namespace(&separate).unwrap_err();
    assert_eq!((r.reason, r.metadata_bytes), ("metadata_separate", 8));
}

#[test]
fn an_inactive_namespace_is_refused() {
    let ns = namespace_page(0, 0, &[(0, 9)]);
    assert_eq!(namespace(&ns).unwrap_err().reason, "inactive_namespace");
    assert_eq!(namespace(&[0u8; 64]).unwrap_err().reason, "short_identify");
}

#[test]
fn format_index_uses_the_upper_flbas_bits() {
    // FLBAS bits 6:5 = 1, bits 3:0 = 0 -> format 16.
    let mut formats = vec![(0u16, 12u8); 17];
    formats[16] = (0, 9);
    let ns = namespace_page(1000, 0x20, &formats);
    assert_eq!(namespace(&ns), Ok(1000));
}

#[test]
fn max_transfer_follows_mdts() {
    let ceiling = 128 * 1024;
    assert_eq!(max_transfer(0, ceiling), ceiling, "0 = no limit");
    assert_eq!(max_transfer(9, ceiling), ceiling, "Samsung 980 PRO: 2 MiB, capped");
    assert_eq!(max_transfer(5, ceiling), 128 * 1024);
    assert_eq!(max_transfer(4, ceiling), 64 * 1024);
    assert_eq!(max_transfer(1, ceiling), 8 * 1024);
    assert_eq!(max_transfer(255, ceiling), ceiling);
}

#[test]
fn capabilities_need_4k_pages_and_the_nvm_set() {
    let nvm = 1u64 << 37;
    assert_eq!(capabilities(nvm | 0x20_0000_0F_FF), Ok(()));
    assert_eq!(capabilities(nvm | 1 << 48), Err("page_size_above_4k"));
    assert_eq!(capabilities(1 << 44), Err("no_nvm_command_set"));
}

#[test]
fn controller_identity_and_text() {
    let mut id = vec![0u8; 4096];
    id[0..2].copy_from_slice(&0x144du16.to_le_bytes());
    id[4..24].copy_from_slice(b"S5Z5NX0R123456      ");
    let mut model = [b' '; 40];
    model[..26].copy_from_slice(b"PM9A1 NVMe Samsung 1024GB\"");
    id[24..64].copy_from_slice(&model);
    id[64..72].copy_from_slice(b"GXA7801Q");
    id[77] = 9;
    id[516] = 1;
    let c = controller(&id).unwrap();
    assert_eq!((c.vendor, c.mdts, c.namespaces), (0x144d, 9, 1));
    let mut out = [0u8; 40];
    let n = text(&c.model, &mut out);
    assert_eq!(&out[..n], b"PM9A1 NVMe Samsung 1024GB?", "trailing spaces dropped, quote masked");
    let n = text(&c.serial, &mut out);
    assert_eq!(&out[..n], b"S5Z5NX0R123456");
    assert!(controller(&id[..100]).is_none());
}
