#[path = "../servers/silk-shell/src/desktop_keys.rs"]
mod keys;
use keys::{Action::*, Keys};
#[test]
fn alt_tab_is_one_focus_action_and_release_never_leaks_to_new_app() {
    let mut k = Keys::new();
    assert_eq!(k.event(0x38, 1), Consume);
    assert_eq!(k.event(0x0F, 1), Switch(1));
    assert_eq!(k.event(0x0F, 1), Consume);
    assert_eq!(k.event(0x0F, 0), Consume);
    assert_eq!(k.event(0x0F, 1), Switch(1));
    // Alt may be released before Tab: it ends the switcher, and neither
    // release belongs to an app.
    assert_eq!(k.event(0x38, 0), SwitchEnd(true));
    assert_eq!(k.event(0x0F, 0), Consume);
    assert_eq!(k.event(0x0F, 1), Pass);
    assert_eq!(k.event(0x0F, 0), Pass);
}
#[test]
fn plain_tab_and_editor_modifiers_keep_existing_routes() {
    let mut k = Keys::new();
    for code in [0x0F, 0x1D, 0x2A, 0x36, 0x1E, 0x01] {
        assert_eq!(k.event(code, 1), Pass);
        assert_eq!(k.event(code, 0), Pass);
    }
    assert_eq!(k.event(0x38, 2), Pass);
    assert_eq!(k.event(0x0F, 1), Pass);
}

#[test]
fn volume_keys_belong_to_shell() {
    let mut k = Keys::new();
    assert_eq!(k.event(keys::KEY_VOLUME_UP, 1), Volume(keys::VOLUME_STEP));
    assert_eq!(k.event(keys::KEY_VOLUME_UP, 2), Volume(keys::VOLUME_STEP));
    assert_eq!(k.event(keys::KEY_VOLUME_UP, 0), Consume);
    assert_eq!(k.event(keys::KEY_VOLUME_DOWN, 1), Volume(-keys::VOLUME_STEP));
    assert_eq!(k.event(keys::KEY_MUTE, 1), Volume(0));
    assert_eq!(k.event(keys::KEY_MUTE, 2), Consume);
    assert_eq!(k.event(keys::KEY_MUTE, 0), Consume);
    assert_eq!(sex_codes(), (0x75, 0x76, 0x77));
}
fn sex_codes() -> (u8, u8, u8) {
    let s = std::fs::read_to_string("crates/sex-pdx/src/app_key.rs").unwrap();
    let get = |name: &str| s.lines().find(|l| l.contains(name)).and_then(|l| l.split("0x").nth(1)).and_then(|h| u8::from_str_radix(&h[..2], 16).ok()).unwrap_or(0);
    (get("KEY_MUTE:"), get("KEY_VOLUME_DOWN:"), get("KEY_VOLUME_UP:"))
}
