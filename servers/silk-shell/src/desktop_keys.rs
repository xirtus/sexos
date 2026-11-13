//! Shell-owned Alt shortcuts: Alt+Tab (Shift+Alt+Tab back; Escape cancels,
//! letting go of Alt keeps the choice), and Alt+= / Alt+- / Alt+0 for the focused
//! window's text size, and Ctrl+Alt+= / Ctrl+Alt+- / Ctrl+Alt+0 for the whole
//! screen's scale (HiDPI). App-local Tab and editor modifiers stay local. The volume
//! and mute keys always belong to Shell.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Pass,
    Consume,
    /// The window switcher: +1 the next window, -1 the previous one.
    Switch(i8),
    /// The switcher ends: true keeps the selected window, false goes back.
    SwitchEnd(bool),
    /// +1 larger, -1 smaller, 0 back to the default size.
    TextZoom(i8),
    /// The whole desktop's scale: +1 larger, -1 smaller, 0 the screen's own.
    DisplayScale(i8),
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
    ctrl: bool,
    shift: bool,
    /// Alt+Tab opened the switcher and Alt is still held.
    switching: bool,
    /// A key pressed with Alt, whose repeat and release Shell keeps from the app.
    captured: Option<u8>,
}
impl Keys {
    pub const fn new() -> Self {
        Self {
            alt: false,
            ctrl: false,
            shift: false,
            switching: false,
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
        // While the switcher is up, Escape cancels and the arrows step.
        if self.switching && value != 0 {
            match code {
                0x01 if value == 1 => { self.switching = false; return Action::SwitchEnd(false); }
                0x4B => return Action::Switch(-1),
                0x4D => return Action::Switch(1),
                0x01 => return Action::Consume,
                _ => {}
            }
        }
        if self.switching && matches!(code, 0x01 | 0x4B | 0x4D) {
            return Action::Consume;
        }
        // Ctrl stays the app's too; Shell only notes it for Ctrl+Alt shortcuts.
        // Right Ctrl (0x61) too: Input gives it the USB code so AltGr is distinct.
        if code == 0x1D || code == 0x61 {
            self.ctrl = value != 0;
            return Action::Pass;
        }
        // Repeats pass through, except a held zoom shortcut's.
        if value > 2 || (value == 2 && self.captured != Some(code)) {
            return Action::Pass;
        }
        if code == 0x38 {
            self.alt = value == 1;
            if value == 0 && core::mem::replace(&mut self.switching, false) {
                return Action::SwitchEnd(true);
            }
            return Action::Consume;
        }
        // Shift stays the app's; Shell notes it for Shift+Alt+Tab.
        if code == 0x2A || code == 0x36 {
            self.shift = value != 0;
            return Action::Pass;
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
            // Scale steps are big; only text zoom repeats.
            return match zoom { Some(step) if step != 0 && self.alt && !self.ctrl => Action::TextZoom(step), _ => Action::Consume };
        }
        if value != 1 || !self.alt {
            return Action::Pass;
        }
        self.captured = Some(code);
        match zoom {
            Some(step) if self.ctrl => Action::DisplayScale(step),
            Some(step) => Action::TextZoom(step),
            None => {
                self.switching = true;
                Action::Switch(if self.shift { -1 } else { 1 })
            }
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
        assert_eq!(k.event(0x0F, 1), Action::Switch(1));
        assert_eq!(k.event(0x0F, 0), Action::Consume);
        assert_eq!(k.event(0x1E, 1), Action::Pass);
    }
    #[test]
    fn alt_tab_switcher_steps_and_ends_with_alt_or_escape() {
        let mut k = Keys::new();
        // Plain Tab and Escape stay the app's.
        assert_eq!(k.event(0x0F, 1), Action::Pass);
        assert_eq!(k.event(0x0F, 0), Action::Pass);
        assert_eq!(k.event(0x01, 1), Action::Pass);
        assert_eq!(k.event(0x01, 0), Action::Pass);
        assert_eq!(k.event(0x38, 1), Action::Consume);
        assert_eq!(k.event(0x0F, 1), Action::Switch(1));
        assert_eq!(k.event(0x0F, 0), Action::Consume);
        assert_eq!(k.event(0x0F, 1), Action::Switch(1));
        assert_eq!(k.event(0x0F, 0), Action::Consume);
        assert_eq!(k.event(0x2A, 1), Action::Pass);
        assert_eq!(k.event(0x0F, 1), Action::Switch(-1));
        assert_eq!(k.event(0x0F, 0), Action::Consume);
        assert_eq!(k.event(0x2A, 0), Action::Pass);
        assert_eq!(k.event(0x4D, 1), Action::Switch(1));
        assert_eq!(k.event(0x4D, 2), Action::Switch(1));
        assert_eq!(k.event(0x4D, 0), Action::Consume);
        assert_eq!(k.event(0x4B, 1), Action::Switch(-1));
        assert_eq!(k.event(0x4B, 0), Action::Consume);
        assert_eq!(k.event(0x38, 0), Action::SwitchEnd(true));
        // Escape goes back and Alt's release is then plain.
        assert_eq!(k.event(0x38, 1), Action::Consume);
        assert_eq!(k.event(0x0F, 1), Action::Switch(1));
        assert_eq!(k.event(0x0F, 0), Action::Consume);
        assert_eq!(k.event(0x01, 1), Action::SwitchEnd(false));
        assert_eq!(k.event(0x01, 0), Action::Pass);
        assert_eq!(k.event(0x38, 0), Action::Consume);
        // Alt with arrows alone is not the switcher's.
        assert_eq!(k.event(0x38, 1), Action::Consume);
        assert_eq!(k.event(0x4D, 1), Action::Pass);
        assert_eq!(k.event(0x38, 0), Action::Consume);
    }
    #[test]
    fn ctrl_alt_steps_the_screen_scale_once_per_press() {
        let mut k = Keys::new();
        assert_eq!(k.event(0x1D, 1), Action::Pass);
        assert_eq!(k.event(0x38, 1), Action::Consume);
        assert_eq!(k.event(0x0D, 1), Action::DisplayScale(1));
        assert_eq!(k.event(0x0D, 2), Action::Consume);
        assert_eq!(k.event(0x0D, 0), Action::Consume);
        assert_eq!(k.event(0x0C, 1), Action::DisplayScale(-1));
        assert_eq!(k.event(0x0C, 0), Action::Consume);
        assert_eq!(k.event(0x0B, 1), Action::DisplayScale(0));
        assert_eq!(k.event(0x0B, 0), Action::Consume);
        assert_eq!(k.event(0x1D, 0), Action::Pass);
        assert_eq!(k.event(0x0D, 1), Action::TextZoom(1));
    }
}
