#[path = "../kernel/src/pci_bar_bounds.rs"]
mod pci_bar_bounds;
use pci_bar_bounds::decode_bar0;

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
