// SPDX-License-Identifier: GPL-3.0-only
//! Interactive `F1` walkthrough coach banner (`HelpCoach` pattern for Weft).
//!
//! Programs define a static list of [`CoachStep`]s, construct a [`Coach`], call
//! [`Coach::render`] once when building the window, and forward [`Response`]s
//! to [`Coach::on`] (or toggle with `F1` / a toolbar button).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::widget::{ButtonStyle, Id, Layout, TextStyle};
use crate::{Response, Ui};

/// One step in a [`Coach`] guided walkthrough.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoachStep {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
    pub target_hint: &'static str,
}

impl CoachStep {
    /// Creates a walkthrough step.
    pub const fn new(
        id: &'static str,
        title: &'static str,
        body: &'static str,
        target_hint: &'static str,
    ) -> Self {
        Self {
            id,
            title,
            body,
            target_hint,
        }
    }
}

/// Widget handles produced by [`Coach::render`] so the banner can be updated in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoachControls {
    pub card: Id,
    pub progress: Id,
    pub title: Id,
    pub target_hint: Id,
    pub body: Id,
    pub prev: Id,
    pub next: Id,
    pub done: Id,
}

/// Interactive `F1` walkthrough coach state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coach {
    pub active: bool,
    pub index: usize,
    pub steps: Vec<CoachStep>,
}

impl Coach {
    /// Creates a closed coach starting at step 0.
    pub fn new(steps: Vec<CoachStep>) -> Self {
        Self {
            active: false,
            index: 0,
            steps,
        }
    }

    /// Toggles the coach open or closed.
    pub fn toggle(&mut self) {
        if self.active {
            self.close();
        } else {
            self.open();
        }
    }

    /// Opens the coach if it has at least one step.
    pub fn open(&mut self) {
        if !self.steps.is_empty() {
            self.active = true;
            if self.index >= self.steps.len() {
                self.index = 0;
            }
        }
    }

    /// Closes the coach without discarding the current step index.
    pub fn close(&mut self) {
        self.active = false;
    }

    /// Advances to the next step if one exists.
    pub fn next(&mut self) {
        if self.index + 1 < self.steps.len() {
            self.index += 1;
        }
    }

    /// Moves back to the previous step if one exists.
    pub fn prev(&mut self) {
        self.index = self.index.saturating_sub(1);
    }

    /// Returns the currently selected step, if any.
    pub fn current(&self) -> Option<&CoachStep> {
        self.steps.get(self.index)
    }

    /// Formats the step counter as `"1 / N"` (or `"0 / 0"` when empty).
    pub fn progress_label(&self) -> String {
        if self.steps.is_empty() {
            String::from("0 / 0")
        } else {
            let cur = self.index.min(self.steps.len() - 1) + 1;
            format!("{} / {}", cur, self.steps.len())
        }
    }

    fn card_layout(visible: bool) -> Layout {
        let mut l = Layout::column().gap(6.0).fill_width().shrink(0.0);
        if !visible {
            l.style().display = taffy::Display::None;
        }
        l
    }

    /// Builds a themed coach banner card under `parent` and returns its widget handles.
    pub fn render(&self, ui: &mut Ui, parent: Id) -> CoachControls {
        let card = ui.card(parent, Self::card_layout(self.active && self.current().is_some()));
        ui.set_label(card, "Interactive walkthrough");

        let header = ui.group(card, Layout::row().gap(8.0).fill_width());
        let title = ui.text(header, "", TextStyle::Heading);
        ui.set_layout(title, Layout::default().grow(1.0));
        let accent = ui.theme().palette.accent;
        ui.set_color(title, Some(accent));

        let target_hint = ui.text(header, "", TextStyle::Caption);
        let progress = ui.text(header, "", TextStyle::Caption);

        let body = ui.paragraph(card, "");
        ui.set_layout(body, Layout::default().fill_width());

        let footer = ui.group(
            card,
            Layout::row()
                .gap(8.0)
                .justify(taffy::JustifyContent::FLEX_END)
                .fill_width(),
        );
        let prev = ui.button(footer, "Previous");
        let next = ui.button(footer, "Next");
        let done = ui.button(footer, "Done");

        let controls = CoachControls {
            card,
            progress,
            title,
            target_hint,
            body,
            prev,
            next,
            done,
        };
        self.sync(ui, &controls);
        controls
    }

    /// Synchronizes an existing [`CoachControls`] banner with the current [`Coach`] state.
    pub fn sync(&self, ui: &mut Ui, controls: &CoachControls) {
        let step = self.current().filter(|_| self.active);
        let visible = step.is_some();
        ui.set_visible(controls.card, visible);
        ui.set_layout(controls.card, Self::card_layout(visible));

        let accent = ui.theme().palette.accent;
        ui.set_color(controls.title, Some(accent));

        if let Some(s) = step {
            ui.set_text(controls.progress, &self.progress_label());
            ui.set_text(controls.title, s.title);
            if s.target_hint.is_empty() {
                ui.set_visible(controls.target_hint, false);
                ui.set_text(controls.target_hint, "");
            } else {
                ui.set_visible(controls.target_hint, true);
                ui.set_text(controls.target_hint, &format!("Focus: {}", s.target_hint));
            }
            ui.set_text(controls.body, s.body);

            let has_prev = self.index > 0;
            let has_next = self.index + 1 < self.steps.len();
            ui.set_enabled(controls.prev, has_prev);
            ui.set_enabled(controls.next, has_next);
            ui.set_button_style(
                controls.next,
                if has_next {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
            ui.set_button_style(
                controls.done,
                if has_next {
                    ButtonStyle::Normal
                } else {
                    ButtonStyle::Primary
                },
            );
        }
    }

    /// Handles clicks on the coach's `Previous`, `Next`, and `Done` buttons.
    /// Returns `true` if `r` belonged to `controls`.
    pub fn on(&mut self, ui: &mut Ui, controls: &CoachControls, r: &Response) -> bool {
        let Response::Clicked(id) = *r else {
            return false;
        };
        if id == controls.prev {
            self.prev();
            self.sync(ui, controls);
            true
        } else if id == controls.next {
            self.next();
            self.sync(ui, controls);
            true
        } else if id == controls.done {
            self.close();
            self.sync(ui, controls);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Strings, Theme};
    use alloc::vec;

    fn sample_steps() -> Vec<CoachStep> {
        vec![
            CoachStep::new("one", "Step One", "First guidance paragraph.", "Toolbar"),
            CoachStep::new("two", "Step Two", "Second guidance paragraph.", "Inspector"),
            CoachStep::new("three", "Step Three", "Final guidance paragraph.", "Export"),
        ]
    }

    #[test]
    fn coach_state_transitions_and_progress_label() {
        let mut coach = Coach::new(sample_steps());
        assert!(!coach.active);
        assert_eq!(coach.progress_label(), "1 / 3");
        assert_eq!(coach.current().map(|s| s.id), Some("one"));

        coach.toggle();
        assert!(coach.active);

        coach.prev();
        assert_eq!(coach.index, 0);

        coach.next();
        assert_eq!(coach.index, 1);
        assert_eq!(coach.progress_label(), "2 / 3");
        assert_eq!(coach.current().map(|s| s.title), Some("Step Two"));

        coach.next();
        coach.next();
        assert_eq!(coach.index, 2);
        assert_eq!(coach.progress_label(), "3 / 3");

        coach.toggle();
        assert!(!coach.active);
        coach.open();
        assert!(coach.active);
        coach.close();
        assert!(!coach.active);

        let mut empty = Coach::new(Vec::new());
        empty.open();
        assert!(!empty.active);
        assert_eq!(empty.progress_label(), "0 / 0");
    }

    #[test]
    fn coach_render_sync_and_button_responses() {
        let mut ui = Ui::new(720, 480, Theme::light(), Strings::new("en"));
        let root = ui.root();
        let mut coach = Coach::new(sample_steps());
        let controls = coach.render(&mut ui, root);

        ui.layout();
        assert_eq!(ui.rect(controls.card).h, 0.0);

        coach.open();
        coach.sync(&mut ui, &controls);
        ui.layout();
        assert!(ui.rect(controls.card).h > 40.0);
        assert_eq!(ui.text_of(controls.progress), "1 / 3");
        assert_eq!(ui.text_of(controls.title), "Step One");
        assert!(!ui.is_enabled(controls.prev));
        assert!(ui.is_enabled(controls.next));

        assert!(coach.on(&mut ui, &controls, &Response::Clicked(controls.next)));
        assert_eq!(ui.text_of(controls.progress), "2 / 3");
        assert_eq!(ui.text_of(controls.title), "Step Two");
        assert!(ui.is_enabled(controls.prev));

        assert!(coach.on(&mut ui, &controls, &Response::Clicked(controls.done)));
        assert!(!coach.active);
        ui.layout();
        assert_eq!(ui.rect(controls.card).h, 0.0);

        let mut px = vec![0u32; 720 * 480];
        assert!(ui.paint(&mut px).is_some());
    }
}
