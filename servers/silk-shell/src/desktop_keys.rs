//! Shell-owned Alt+Tab state. App-local Tab and editor modifiers stay local.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Pass,
    Consume,
    NextFrame,
}
pub struct Keys {
    alt: bool,
    captured_tab: bool,
}
impl Keys {
    pub const fn new() -> Self {
        Self {
            alt: false,
            captured_tab: false,
        }
    }
    pub fn event(&mut self, code: u8, value: u64) -> Action {
        if value > 1 {
            return Action::Pass;
        }
        if code == 0x38 {
            self.alt = value == 1;
            return Action::Consume;
        }
        if code != 0x0F {
            return Action::Pass;
        }
        if value == 0 {
            if self.captured_tab {
                self.captured_tab = false;
                return Action::Consume;
            }
            return Action::Pass;
        }
        if self.captured_tab {
            return Action::Consume;
        }
        if self.alt {
            self.captured_tab = true;
            Action::NextFrame
        } else {
            Action::Pass
        }
    }
}
