//! Seeded before task admission; exact BOOT roles query through sex-pdx.
use super::boot_display_state::Info;
static INFO: spin::Once<Info> = spin::Once::new();
pub fn init(info: Info) {
    assert!(INFO.get().is_none(), "BOOT_DISPLAY_ALREADY_SEEDED");
    assert!(info.width > 0 && info.height > 0 && info.pitch >= info.width * 4);
    INFO.call_once(|| info);
}
pub fn query(caller: u32, kind: u64, a: u64, b: u64) -> Option<u64> {
    INFO.get()?.query(caller, kind, [a, b])
}
