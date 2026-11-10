//! Shell-owned Alt shortcuts: Alt+Tab, and Alt+= / Alt+- / Alt+0 for the focused
//! window's text size. App-local Tab and editor modifiers stay local. The volume and
//! mute keys always belong to Shell.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Pass,
    Consume,
    NextFrame,
    /// +1 larger, -1 smaller, 0 back to the default size.
    TextZoom(i8),
    /// Master volume by this many percent; 0 toggles mute.
    Volume(i8),
}

/// Media key codes, as `sex_pdx::app_key` names them (kept here so this file builds alone).
pub const KEY_MUTE: u8 = 0x75;
pub const KEY_VOLUME_DOWN: u8 = 0x76;
pub const KEY_VOLUME_UP: u8 = 0x77;

/// One volume key press (or repeat) moves the master volume this far.
pub const VOLUME_STEP: i8 = 5;
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
        // Volume keys: presses and repeats step, mute toggles once, releases are Shell's.
        let volume = match code {
            KEY_VOLUME_UP => Some(VOLUME_STEP),
            KEY_VOLUME_DOWN => Some(-VOLUME_STEP),
            KEY_MUTE => Some(0),
            _ => None,
        };
        if let Some(step) = volume {
            return match value {
                1 => Action::Volume(step),
                2 if step != 0 => Action::Volume(step),
                _ => Action::Consume,
            };
        }
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
        assert_eq!(k.event(KEY_VOLUME_UP, 1), Action::Volume(VOLUME_STEP));
        assert_eq!(k.event(KEY_VOLUME_UP, 2), Action::Volume(VOLUME_STEP));
        assert_eq!(k.event(KEY_VOLUME_UP, 0), Action::Consume);
        assert_eq!(k.event(KEY_VOLUME_DOWN, 1), Action::Volume(-VOLUME_STEP));
        assert_eq!(k.event(KEY_MUTE, 1), Action::Volume(0));
        assert_eq!(k.event(KEY_MUTE, 2), Action::Consume);
        assert_eq!(k.event(0x0F, 1), Action::NextFrame);
        assert_eq!(k.event(0x0F, 0), Action::Consume);
        assert_eq!(k.event(0x1E, 1), Action::Pass);
    }
}
