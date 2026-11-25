// SPDX-License-Identifier: GPL-3.0-only
//! The filter menu's small, host-testable view of GridCraft's active criterion.
use crate::book::FilterState;

pub struct FilterMenu {
    pub col: u32,
    pub values: Vec<String>,
    pub selected: usize,
    pub checked: Vec<bool>,
    /// Checked values can outlive the bounded preview while a search changes.
    pub checked_all: Vec<String>,
    pub toggled: bool,
    pub active: bool,
    pub condition: bool,
    pub query: String,
    /// Visible result row -> original value index; checks always use original indices.
    pub results: Vec<usize>,
    pub searching: bool,
    /// A full-range exact match beyond the bounded preview.
    pub exact: Option<String>,
}

impl FilterMenu {
    pub fn new(col: u32, values: Vec<String>, state: FilterState) -> Self {
        let active = state != FilterState::None;
        let condition = state == FilterState::Other;
        let checked_all = match &state {
            FilterState::Values { values, blanks } => {
                let mut kept = values.clone();
                if *blanks { kept.push(String::new()); }
                kept
            }
            _ => Vec::new(),
        };
        let checked = values.iter().map(|value| checked_all.iter().any(|selected| selected.eq_ignore_ascii_case(value))).collect();
        let results = (0..values.len()).collect();
        Self { col, values, selected: 0, checked, checked_all, toggled: false, active, condition, query: String::new(), results, searching: false, exact: None }
    }
    pub fn untouched_active(&self) -> bool { self.active && !self.toggled && self.selected == 0 }
    pub fn visible_len(&self) -> usize { self.results.len() + 1 + usize::from(self.exact.is_some()) }
    pub fn value_index(&self, row: usize) -> Option<usize> { row.checked_sub(1).and_then(|n| self.results.get(n)).copied() }
    pub fn selected_value_index(&self) -> Option<usize> { self.value_index(self.selected) }
    pub fn selected_exact(&self) -> Option<&str> {
        if self.selected == 1 && self.results.is_empty() { self.exact.as_deref() } else { None }
    }
    pub fn offer_exact(&mut self, value: Option<String>) {
        self.exact = value.filter(|_| self.results.is_empty());
        if self.exact.is_some() { self.selected = 1; }
    }
    pub fn extend_values(&mut self, values: Vec<String>) {
        let mut added = false;
        for value in values {
            if self.values.iter().any(|old| old.eq_ignore_ascii_case(&value)) { continue; }
            self.checked.push(self.checked_all.iter().any(|old| old.eq_ignore_ascii_case(&value)));
            self.values.push(value);
            added = true;
        }
        if added { self.refresh(); }
    }
    pub fn toggle_selected(&mut self) {
        if self.selected == 0 {
            self.checked.fill(false);
            self.checked_all.clear();
        } else if let Some(i) = self.selected_value_index() {
            let value = self.values[i].clone();
            let on = !self.checked[i];
            for (shown, checked) in self.values.iter().zip(&mut self.checked) {
                if shown.eq_ignore_ascii_case(&value) { *checked = on; }
            }
            self.checked_all.retain(|old| !old.eq_ignore_ascii_case(&value));
            if on { self.checked_all.push(value); }
        }
        self.toggled = true;
    }
    pub fn push_query(&mut self, c: char) {
        if self.query.chars().count() < 64 { self.query.push(c); self.refresh(); }
    }
    pub fn pop_query(&mut self) { self.query.pop(); self.refresh(); }
    pub fn clear_query(&mut self) { self.query.clear(); self.refresh(); }
    fn refresh(&mut self) {
        self.exact = None;
        let needle = self.query.to_lowercase();
        self.results = self.values.iter().enumerate().filter_map(|(i, v)| {
            let text = if v.is_empty() { "(blank)" } else { v };
            text.to_lowercase().contains(&needle).then_some(i)
        }).collect();
        self.selected = usize::from(!self.results.is_empty() && !self.query.is_empty());
    }
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

    #[test]
    fn search_keeps_original_checks_and_selects_a_distant_match() {
        let values = (0..1_000).map(|i| format!("Item {i:04}")).collect();
        let mut menu = FilterMenu::new(0, values, FilterState::Values {
            values: vec!["Item 0001".into(), "Item 0999".into()], blanks: false,
        });
        menu.searching = true;
        for c in "0999".chars() { menu.push_query(c); }
        assert_eq!(menu.visible_len(), 2);
        assert_eq!(menu.selected_value_index(), Some(999));
        assert!(menu.checked[1] && menu.checked[999]);
        menu.checked[999] = false;
        menu.pop_query();
        assert_eq!(menu.query, "099");
        assert!(menu.checked[1] && !menu.checked[999]);
        menu.clear_query();
        assert_eq!(menu.visible_len(), 1_001);
    }

    #[test]
    fn exact_value_outside_preview_becomes_an_applyable_row() {
        let mut menu = FilterMenu::new(0, vec!["Item 0001".into()], FilterState::None);
        for c in "item 1100".chars() { menu.push_query(c); }
        assert_eq!(menu.visible_len(), 1);
        menu.offer_exact(Some("Item 1100".into()));
        assert_eq!(menu.visible_len(), 2);
        assert_eq!(menu.selected_exact(), Some("Item 1100"));
        menu.pop_query();
        assert_eq!(menu.selected_exact(), None);
    }

    #[test]
    fn search_adds_distant_matches_and_keeps_multiple_checked_values() {
        let mut menu = FilterMenu::new(0, vec!["Item 0001".into()], FilterState::Values {
            values: vec!["Item 0001".into()], blanks: false,
        });
        for c in "late a".chars() { menu.push_query(c); }
        menu.extend_values(vec!["Late amber".into(), "Late azure".into()]);
        assert_eq!(menu.visible_len(), 3);
        menu.selected = 1;
        menu.toggle_selected();
        menu.selected = 2;
        menu.toggle_selected();
        assert_eq!(menu.checked_all, ["Item 0001", "Late amber", "Late azure"]);
        menu.clear_query();
        assert!(menu.checked[0] && menu.checked[1] && menu.checked[2]);
    }
}
