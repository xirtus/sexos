//! Shell-owned Alt shortcuts: Alt+Tab, and Alt+= / Alt+- / Alt+0 for the focused
//! window's text size. App-local Tab and editor modifiers stay local.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Pass,
    Consume,
    NextFrame,
    /// +1 larger, -1 smaller, 0 back to the default size.
    TextZoom(i8),
}
pub struct Keys {
    alt: bool,
    /// A key pressed with Alt, whose repeat and release Shell keeps from the app.
    captured: Option<u8>,
}
impl Keys {
    pub const fn new() -> Self {
        Self {
            alt: false,
            captured: None,
        }
    }
    pub fn event(&mut self, code: u8, value: u64) -> Action {
        // Repeats pass through, except a held zoom shortcut's.
        if value > 2 || (value == 2 && self.captured != Some(code)) {
            return Action::Pass;
        }
        if code == 0x38 {
            self.alt = value == 1;
            return Action::Consume;
        }
        let zoom = match code {
            0x0D => Some(1),  // =
            0x0C => Some(-1), // -
            0x0B => Some(0),  // 0
            _ => None,
        };
        if code != 0x0F && zoom.is_none() {
            return Action::Pass;
        }
        if self.captured == Some(code) {
            if value == 0 {
                self.captured = None;
                return Action::Consume;
            }
            // Holding Alt+= keeps growing; Tab repeats stay consumed.
            return match zoom { Some(step) if step != 0 && self.alt => Action::TextZoom(step), _ => Action::Consume };
        }
        if value != 1 || !self.alt {
            return Action::Pass;
        }
        self.captured = Some(code);
        match zoom {
            Some(step) => Action::TextZoom(step),
            None => Action::NextFrame,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alt_shortcuts_are_shells_and_plain_keys_are_the_apps() {
        let mut k = Keys::new();
        assert_eq!(k.event(0x0D, 1), Action::Pass);
        assert_eq!(k.event(0x0D, 0), Action::Pass);
        assert_eq!(k.event(0x38, 1), Action::Consume);
        assert_eq!(k.event(0x0D, 1), Action::TextZoom(1));
        assert_eq!(k.event(0x0D, 2), Action::TextZoom(1));
        assert_eq!(k.event(0x0D, 0), Action::Consume);
        assert_eq!(k.event(0x0C, 1), Action::TextZoom(-1));
        assert_eq!(k.event(0x38, 0), Action::Consume);
        // Released after Alt: the app still never sees half a shortcut.
        assert_eq!(k.event(0x0C, 0), Action::Consume);
        assert_eq!(k.event(0x38, 1), Action::Consume);
        assert_eq!(k.event(0x0B, 1), Action::TextZoom(0));
        assert_eq!(k.event(0x0B, 0), Action::Consume);
        assert_eq!(k.event(0x0F, 1), Action::NextFrame);
        assert_eq!(k.event(0x0F, 0), Action::Consume);
        assert_eq!(k.event(0x1E, 1), Action::Pass);
    }
}
