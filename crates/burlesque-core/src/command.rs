// SPDX-License-Identifier: GPL-3.0-only
//! Presentation undo/redo command stack.

use alloc::string::String;
use alloc::vec::Vec;
use crate::deck::{Deck, Metadata, Slide};
use crate::elements::SlideElement;
use crate::theme::Theme;

/// Individual presentation modification commands for undo/redo.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    AddSlide { index: usize, slide: Slide },
    DeleteSlide { index: usize, slide: Slide },
    MoveSlide { from: usize, to: usize },
    AddElement { slide_index: usize, element_index: usize, element: SlideElement },
    DeleteElement { slide_index: usize, element_index: usize, element: SlideElement },
    UpdateElement { slide_index: usize, element_index: usize, old: SlideElement, new: SlideElement },
    ApplyTheme { old: Theme, new: Theme },
    UpdateNotes { slide_index: usize, old: String, new: String },
    UpdateMetadata { old: Metadata, new: Metadata },
}

impl Command {
    /// Invert this command for undoing.
    pub fn inverted(&self) -> Self {
        match self {
            Command::AddSlide { index, slide } => Command::DeleteSlide {
                index: *index,
                slide: slide.clone(),
            },
            Command::DeleteSlide { index, slide } => Command::AddSlide {
                index: *index,
                slide: slide.clone(),
            },
            Command::MoveSlide { from, to } => Command::MoveSlide {
                from: *to,
                to: *from,
            },
            Command::AddElement { slide_index, element_index, element } => Command::DeleteElement {
                slide_index: *slide_index,
                element_index: *element_index,
                element: element.clone(),
            },
            Command::DeleteElement { slide_index, element_index, element } => Command::AddElement {
                slide_index: *slide_index,
                element_index: *element_index,
                element: element.clone(),
            },
            Command::UpdateElement { slide_index, element_index, old, new } => Command::UpdateElement {
                slide_index: *slide_index,
                element_index: *element_index,
                old: new.clone(),
                new: old.clone(),
            },
            Command::ApplyTheme { old, new } => Command::ApplyTheme {
                old: new.clone(),
                new: old.clone(),
            },
            Command::UpdateNotes { slide_index, old, new } => Command::UpdateNotes {
                slide_index: *slide_index,
                old: new.clone(),
                new: old.clone(),
            },
            Command::UpdateMetadata { old, new } => Command::UpdateMetadata {
                old: new.clone(),
                new: old.clone(),
            },
        }
    }

    /// Execute forward mutation on the deck.
    pub fn execute(&self, deck: &mut Deck) -> bool {
        match self {
            Command::AddSlide { index, slide } => {
                deck.insert_slide(*index, slide.clone());
                true
            }
            Command::DeleteSlide { index, .. } => {
                deck.remove_slide(*index).is_some()
            }
            Command::MoveSlide { from, to } => {
                deck.move_slide(*from, *to)
            }
            Command::AddElement { slide_index, element_index, element } => {
                if let Some(slide) = deck.get_slide_mut(*slide_index) {
                    let pos = (*element_index).min(slide.elements.len());
                    slide.elements.insert(pos, element.clone());
                    true
                } else {
                    false
                }
            }
            Command::DeleteElement { slide_index, element_index, .. } => {
                if let Some(slide) = deck.get_slide_mut(*slide_index) {
                    if *element_index < slide.elements.len() {
                        slide.elements.remove(*element_index);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Command::UpdateElement { slide_index, element_index, new, .. } => {
                if let Some(slide) = deck.get_slide_mut(*slide_index) {
                    if *element_index < slide.elements.len() {
                        slide.elements[*element_index] = new.clone();
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            Command::ApplyTheme { new, .. } => {
                deck.theme = new.clone();
                true
            }
            Command::UpdateNotes { slide_index, new, .. } => {
                if let Some(slide) = deck.get_slide_mut(*slide_index) {
                    slide.speaker_notes = new.clone();
                    true
                } else {
                    false
                }
            }
            Command::UpdateMetadata { new, .. } => {
                deck.metadata = new.clone();
                true
            }
        }
    }
}

/// Presentation command undo/redo history stack.
#[derive(Clone, Debug)]
pub struct CommandStack {
    undo_stack: Vec<Command>,
    redo_stack: Vec<Command>,
    max_depth: usize,
}

impl CommandStack {
    pub fn new(max_depth: usize) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_depth: max_depth.max(1),
        }
    }

    pub fn apply(&mut self, cmd: Command, deck: &mut Deck) -> bool {
        if cmd.execute(deck) {
            self.redo_stack.clear();
            if self.undo_stack.len() >= self.max_depth {
                self.undo_stack.remove(0);
            }
            self.undo_stack.push(cmd);
            true
        } else {
            false
        }
    }

    pub fn undo(&mut self, deck: &mut Deck) -> bool {
        if let Some(cmd) = self.undo_stack.pop() {
            let inv = cmd.inverted();
            if inv.execute(deck) {
                self.redo_stack.push(cmd);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn redo(&mut self, deck: &mut Deck) -> bool {
        if let Some(cmd) = self.redo_stack.pop() {
            if cmd.execute(deck) {
                self.undo_stack.push(cmd);
                true
            } else {
                false
            }
        } else {
            false
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }
}

impl Default for CommandStack {
    fn default() -> Self {
        Self::new(100)
    }
}
