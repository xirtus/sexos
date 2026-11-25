// SPDX-License-Identifier: GPL-3.0-only
//! Bounded undo/redo for committed form-field edits. Annotation changes clear it.
use vellum_edit::{Editor, Error, FieldValue};

const LIMIT: usize = 32;

#[derive(Clone)]
struct Change { name: String, before: FieldValue, after: FieldValue }

#[derive(Default)]
pub struct FieldHistory { undo: Vec<Change>, redo: Vec<Change> }

impl FieldHistory {
    pub fn clear(&mut self) { self.undo.clear(); self.redo.clear(); }

    pub fn record(&mut self, name: &str, before: FieldValue, after: FieldValue) {
        if before == after { return; }
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push(Change { name: name.into(), before, after });
        self.redo.clear();
    }

    /// Applies an inverse/forward edit before moving the history cursor.
    pub fn step(&mut self, editor: &mut Editor, redo: bool) -> Result<Option<String>, Error> {
        let Some(change) = (if redo { self.redo.last() } else { self.undo.last() }).cloned() else { return Ok(None) };
        editor.set_field(&change.name, if redo { change.after.clone() } else { change.before.clone() })?;
        if redo { self.redo.pop(); self.undo.push(change.clone()); }
        else { self.undo.pop(); self.redo.push(change.clone()); }
        Ok(Some(change.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FORM: &[u8] = include_bytes!("../../../tests/fixtures/pdf/form1.pdf");

    fn value(editor: &Editor, name: &str) -> FieldValue {
        editor.fields().unwrap().into_iter().find(|f| f.name == name).unwrap().value
    }

    #[test]
    fn undo_redo_changes_real_pdf_field_values() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        let mut history = FieldHistory::default();
        let before = value(&editor, "name");
        let after = FieldValue::Text("Ada".into());
        editor.set_field("name", after.clone()).unwrap();
        history.record("name", before.clone(), after.clone());
        assert_eq!(history.step(&mut editor, false).unwrap(), Some("name".into()));
        let restored = match before { FieldValue::Empty => FieldValue::Text(String::new()), other => other };
        assert_eq!(value(&editor, "name"), restored);
        assert_eq!(history.step(&mut editor, true).unwrap(), Some("name".into()));
        assert_eq!(value(&editor, "name"), after);
        assert_eq!(value(&Editor::open(editor.save_incremental().unwrap()).unwrap(), "name"), after);
    }

    #[test]
    fn history_is_bounded_and_new_edit_clears_redo() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        let mut history = FieldHistory::default();
        for i in 0..40 {
            let before = value(&editor, "name");
            let after = FieldValue::Text(i.to_string());
            editor.set_field("name", after.clone()).unwrap();
            history.record("name", before, after);
        }
        assert_eq!(history.undo.len(), LIMIT);
        history.step(&mut editor, false).unwrap();
        let before = value(&editor, "name");
        let after = FieldValue::Text("new".into());
        editor.set_field("name", after.clone()).unwrap();
        history.record("name", before, after);
        assert!(history.step(&mut editor, true).unwrap().is_none());
    }
}
