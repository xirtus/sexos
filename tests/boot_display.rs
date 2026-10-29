#[path = "../kernel/src/boot_display_state.rs"]
mod production;
use production::Info;
fn info() -> Info {
    Info {
        display: 1,
        shell: 3,
        input: 4,
        address: 0xffff8000fd000000,
        width: 1024,
        height: 768,
        pitch: 4096,
    }
}
#[test]
fn exact_boot_roles_only_with_zero_reserved_args() {
    let i = info();
    for pd in [1, 3, 4] {
        assert_eq!(i.query(pd, 0, [0, 0]), Some(1024 | (768 << 32)));
        assert_eq!(i.query(pd, 2, [0, 0]), Some(4096));
        assert_eq!(i.query(pd, 0, [1, 0]), None);
        assert_eq!(i.query(pd, 0, [0, 1]), None);
    }
    for pd in [0, 2, 9, 12, 16, 17, u32::MAX] {
        for op in [0, 1, 2, u64::MAX] {
            assert_eq!(i.query(pd, op, [0, 0]), None)
        }
    }
}
#[test]
fn address_resolves_only_original_display_authority() {
    let i = info();
    assert_eq!(i.query(1, 1, [0, 0]), Some(i.address));
    assert_eq!(i.query(3, 1, [0, 0]), None);
    assert_eq!(i.query(4, 1, [0, 0]), None);
    assert_eq!(i.query(1, 1 << 32, [0, 0]), None);
}
