#[path = "../kernel/src/pci_bar_bounds.rs"]
mod pci_bar_bounds;
use pci_bar_bounds::{decode_bar0,decode_subpage};

#[test]
fn valid_32_and_64_bit_bars_preserve_exact_extent() {
    assert_eq!(decode_bar0(0xf000_0000, 0, 0xffff_c000, 0), Some((0xf000_0000, 0x4000)));
    assert_eq!(decode_bar0(4, 1, 0xffff_0004, u32::MAX), Some((1u64 << 32, 0x10000)));
}

#[test]
fn absent_io_reserved_type_subpage_and_unaligned_bars_are_denied() {
    for (low, mask) in [(0, 0xffff_f000), (0xf000_0001, 0xffff_f001),
        (0xf000_0002, 0xffff_f002), (0xf000_0000, 0xffff_ff00),
        (0xf000_1000, 0xffff_c000), (0xf000_0800, 0xffff_f800)] {
        assert_eq!(decode_bar0(low, 0, mask, 0), None);
    }
}

#[test]
fn malformed_masks_and_out_of_physical_range_are_denied() {
    assert_eq!(decode_bar0(0xf000_0000, 0, 0xffff_a000, 0), None);
    assert_eq!(decode_bar0(0xf000_0000, 0, 0, 0), None);
    assert_eq!(decode_bar0(4, 0x10_0000, 0xffff_f004, u32::MAX), None);
    assert_eq!(decode_bar0(4, 1, 0, 0), None);
}
#[test]fn sub_page_bars_decode_only_when_aligned_and_smaller_than_a_page(){
    assert_eq!(decode_subpage(0xfebf_0800, 0, 0xffff_f800, 0), Some((0xfebf_0800, 0x800)));
    assert_eq!(decode_subpage(0xfebf_1100, 0, 0xffff_ff00, 0), Some((0xfebf_1100, 0x100)));
    assert_eq!(decode_subpage(0xf000_0000, 0, 0xffff_c000, 0), None); // a page or more: decode_bar0's job
    assert_eq!(decode_subpage(0xfebf_0400, 0, 0xffff_f800, 0), None); // misaligned
    assert_eq!(decode_subpage(0xfebf_0801, 0, 0xffff_f801, 0), None); // I/O
    assert_eq!(decode_subpage(0, 0, 0xffff_f800, 0), None);
}
