// SPDX-License-Identifier: GPL-3.0-only
//! Untrusted labels are escaped without truncating consent scope. Small tiles
//! cannot approve; the runtime rechecks this predicate on every approval/resize.
pub fn reviewable(width: usize, height: usize, body: &str) -> bool {
    width >= 800 && height >= 600 && body.len() <= 400 && body.lines().count() <= 12
}
pub fn body(
    name: &[u8; 16],
    pd: u32,
    task: u32,
    origin: &str,
    title: &str,
    user: &str,
) -> Option<String> {
    if origin.is_empty() || origin.len() > 512 || !origin.bytes().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    let escape = |v: &str| v.chars().flat_map(char::escape_default).collect::<String>();
    let end = name.iter().position(|b| *b == 0).unwrap_or(16);
    let name = core::str::from_utf8(&name[..end]).ok()?;
    // Fixed 64-column lines keep the full escaped origin and selected entry visible.
    let fields = [
        format!("Consumer: {} (PD {pd}, run {task})", escape(name)),
        format!("Origin: {}", escape(origin)),
        format!("Entry: {}", escape(title)),
        format!("User: {}", escape(user)),
    ];
    let mut out = String::new();
    for field in fields {
        for chunk in field.as_bytes().chunks(64) {
            out.push_str(core::str::from_utf8(chunk).ok()?);
            out.push('\n');
        }
    }
    if out.len() > 400 || out.lines().count() > 12 {
        None
    } else {
        Some(out)
    }
}
pub fn buttons(
    ui: &mut weft::Ui,
    row: weft::Id,
    size: (usize, usize),
    body: &str,
) -> (weft::Id, weft::Id) {
    let deny = ui.button(row, "Deny");
    let allow = ui.button(row, "Allow once");
    ui.set_enabled(allow, reviewable(size.0, size.1, body));
    ui.focus(Some(deny));
    (deny, allow)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_allow_cannot_be_activated_by_tab_enter_in_actual_weft() {
        use weft::input::{Event, Key};
        use weft::{Response, Strings, Theme, Ui};
        let mut ui = Ui::new(550, 144, Theme::light(), Strings::new("en"));
        let root = ui.root();
        let (_, allow) = buttons(
            &mut ui,
            root,
            (550, 144),
            "Consumer: client\nOrigin: https://example.org\nEntry: Mail",
        );
        ui.handle(Event::Key(Key::new(0x0f)));
        let responses = ui.handle(Event::Key(Key::new(0x1c)));
        assert!(!ui.is_enabled(allow));
        assert!(!responses.contains(&Response::Clicked(allow)));
    }
    #[test]
    fn tiny_tile_cannot_approve_hidden_scope() {
        assert!(!reviewable(
            550,
            144,
            "Consumer: client\nOrigin: https://example.org\nEntry: Mail"
        ));
        assert!(reviewable(800, 600, "scope"));
    }
    #[test]
    fn controls_and_bidi_cannot_forge_consent_labels() {
        let s = body(
            b"client\0\0\0\0\0\0\0\0\0\0",
            43,
            101,
            "https://example.org",
            "Mail\nOrigin: evil\u{202e}",
            "alice",
        )
        .unwrap();
        assert!(s.contains("\\nOrigin: evil\\u{202e}"));
        assert!(!s.contains('\u{202e}'));
        assert!(body(&[0; 16], 1, 1, "https://example.org\nentry:x", "a", "u").is_none());
    }
}
