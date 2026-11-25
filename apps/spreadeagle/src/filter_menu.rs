// SPDX-License-Identifier: GPL-3.0-only
//! The filter menu's small, host-testable view of GridCraft's active criterion.
use crate::book::FilterState;

pub struct FilterMenu {
    pub col: u32,
    pub values: Vec<String>,
    pub selected: usize,
    pub checked: Vec<bool>,
    pub toggled: bool,
    pub active: bool,
    pub condition: bool,
}

impl FilterMenu {
    pub fn new(col: u32, values: Vec<String>, state: FilterState) -> Self {
        let active = state != FilterState::None;
        let condition = state == FilterState::Other;
        let checked = values.iter().map(|value| match &state {
            FilterState::Values { values, blanks } => (value.is_empty() && *blanks)
                || values.iter().any(|selected| selected.eq_ignore_ascii_case(value)),
            _ => false,
        }).collect();
        Self { col, values, selected: 0, checked, toggled: false, active, condition }
    }
    pub fn untouched_active(&self) -> bool { self.active && !self.toggled && self.selected == 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_values_and_blank_are_checked_without_implicit_clear() {
        let mut menu = FilterMenu::new(0, vec!["".into(), "Food".into(), "Fun".into()],
            FilterState::Values { values: vec!["food".into()], blanks: true });
        assert_eq!(menu.checked, [true, true, false]);
        assert!(menu.untouched_active());
        menu.selected = 2;
        assert!(!menu.untouched_active());
        menu.selected = 0;
        menu.toggled = true;
        assert!(!menu.untouched_active());
        let condition = FilterMenu::new(0, vec!["Food".into()], FilterState::Other);
        assert!(condition.condition && condition.untouched_active());
        assert_eq!(condition.checked, [false]);
    }
}
