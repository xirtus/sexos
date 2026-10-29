#[path = "../servers/silk-shell/src/desktop_keys.rs"]
mod keys;
use keys::{Action::*, Keys};
#[test]
fn alt_tab_is_one_focus_action_and_release_never_leaks_to_new_app() {
    let mut k = Keys::new();
    assert_eq!(k.event(0x38, 1), Consume);
    assert_eq!(k.event(0x0F, 1), NextFrame);
    assert_eq!(k.event(0x0F, 1), Consume);
    assert_eq!(k.event(0x0F, 0), Consume);
    assert_eq!(k.event(0x0F, 1), NextFrame);
    // Alt may be released before Tab: neither release belongs to an app.
    assert_eq!(k.event(0x38, 0), Consume);
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
