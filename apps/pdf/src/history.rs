// SPDX-License-Identifier: GPL-3.0-only
//! Bounded undo/redo for committed form fields and annotation changes.
use vellum_edit::{Editor, Error, FieldValue, NewAnnot, ObjId, RemovedAnnot};

const LIMIT: usize = 32;

#[derive(Clone)]
struct Change { name: String, before: FieldValue, after: FieldValue }

#[derive(Default)]
pub struct FieldHistory { undo: Vec<Change>, redo: Vec<Change> }

#[derive(Clone)]
struct AddedAnnotation { page: usize, annot: NewAnnot, id: ObjId, removed: Option<RemovedAnnot> }

#[derive(Clone)]
enum AnnotationChange {
    Added(AddedAnnotation),
    Deleted { page: usize, removed: RemovedAnnot },
    Comment { page: usize, index: usize, before: String, after: String },
}

#[derive(Default)]
pub struct AnnotationHistory { undo: Vec<AnnotationChange>, redo: Vec<AnnotationChange> }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind { Field, Annotation }

/// One edit timeline across forms and markup. The two payload stacks stay bounded,
/// while this order stack decides which kind Ctrl+Z and Ctrl+Shift+Z visit next.
#[derive(Default)]
pub struct EditHistory {
    fields: FieldHistory,
    annotations: AnnotationHistory,
    undo: Vec<Kind>,
    redo: Vec<Kind>,
}

pub enum EditStep { Field(String), Annotation(usize) }

impl EditHistory {
    pub fn clear(&mut self) { self.fields.clear(); self.annotations.clear(); self.undo.clear(); self.redo.clear(); }
    fn record(&mut self, kind: Kind) {
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push(kind);
        self.redo.clear();
        self.fields.redo.clear();
        self.annotations.redo.clear();
    }
    pub fn record_field(&mut self, name: &str, before: FieldValue, after: FieldValue) {
        if before == after { return; }
        self.fields.record(name, before, after);
        self.record(Kind::Field);
    }
    pub fn record_add(&mut self, page: usize, annot: NewAnnot, id: ObjId) {
        self.annotations.record_add(page, annot, id);
        self.record(Kind::Annotation);
    }
    pub fn record_delete(&mut self, page: usize, removed: RemovedAnnot) {
        self.annotations.record_delete(page, removed);
        self.record(Kind::Annotation);
    }
    pub fn record_comment(&mut self, page: usize, index: usize, before: String, after: String) {
        if before == after { return; }
        self.annotations.record_comment(page, index, before, after);
        self.record(Kind::Annotation);
    }
    pub fn step(&mut self, editor: &mut Editor, redo: bool) -> Result<Option<EditStep>, Error> {
        let Some(kind) = (if redo { &self.redo } else { &self.undo }).last().copied() else { return Ok(None) };
        let step = match kind {
            Kind::Field => self.fields.step(editor, redo)?.map(EditStep::Field),
            Kind::Annotation => self.annotations.step(editor, redo)?.map(EditStep::Annotation),
        };
        if step.is_some() {
            if redo { self.redo.pop(); self.undo.push(kind); }
            else { self.undo.pop(); self.redo.push(kind); }
        }
        Ok(step)
    }
}

impl AnnotationHistory {
    pub fn clear(&mut self) { self.undo.clear(); self.redo.clear(); }
    pub fn has_step(&self, redo: bool) -> bool { !(if redo { &self.redo } else { &self.undo }).is_empty() }

    pub fn record_add(&mut self, page: usize, annot: NewAnnot, id: ObjId) {
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push(AnnotationChange::Added(AddedAnnotation { page, annot, id, removed: None }));
        self.redo.clear();
    }

    pub fn record_delete(&mut self, page: usize, removed: RemovedAnnot) {
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push(AnnotationChange::Deleted { page, removed });
        self.redo.clear();
    }

    pub fn record_comment(&mut self, page: usize, index: usize, before: String, after: String) {
        if before == after { return; }
        if self.undo.len() == LIMIT { self.undo.remove(0); }
        self.undo.push(AnnotationChange::Comment { page, index, before, after });
        self.redo.clear();
    }

    /// Undo removes only the annotation this history created, identified by PDF object id.
    /// Redo restores that same object and appearance so a later delete step stays valid.
    pub fn step(&mut self, editor: &mut Editor, redo: bool) -> Result<Option<usize>, Error> {
        let Some(mut change) = (if redo { self.redo.last() } else { self.undo.last() }).cloned() else { return Ok(None) };
        let page = match &mut change {
            AnnotationChange::Added(add) => {
                if redo {
                    if let Some(removed) = &add.removed { editor.restore_annotation(removed)?; }
                    else { add.id = editor.add_annotation(add.page, &add.annot)?; }
                }
                else {
                    let index = editor.annotations(add.page)?.into_iter()
                        .find(|a| a.id == Some(add.id)).map(|a| a.index)
                        .ok_or(Error::NoSuchAnnotation(usize::MAX))?;
                    add.removed = Some(editor.remove_annotation(add.page, index)?);
                }
                add.page
            }
            AnnotationChange::Deleted { page, removed } => {
                if redo { editor.remove_restored_annotation(removed)?; }
                else { editor.restore_annotation(removed)?; }
                *page
            }
            AnnotationChange::Comment { page, index, before, after } => {
                editor.set_annotation_text(*page, *index, if redo { after } else { before })?;
                *page
            }
        };
        if redo { self.redo.pop(); self.undo.push(change); }
        else { self.undo.pop(); self.redo.push(change); }
        Ok(Some(page))
    }
}

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
    use vellum_edit::{AnnotKind, AnnotStyle, NoteIcon};
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
    fn undo_redo_highlight_comment() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        editor.add_annotation(0, &NewAnnot {
            kind: AnnotKind::Highlight { rects: vec![[20.0, 175.0, 80.0, 185.0]] },
            style: AnnotStyle::default(),
        }).unwrap();
        let index = editor.annotations(0).unwrap().into_iter().find(|a| a.subtype == "Highlight").unwrap().index;
        editor.set_annotation_text(0, index, "Review green").unwrap();
        let mut history = AnnotationHistory::default();
        history.record_comment(0, index, String::new(), "Review green".into());
        assert_eq!(history.step(&mut editor, false).unwrap(), Some(0));
        assert_eq!(editor.annotations(0).unwrap()[index].contents, None);
        assert_eq!(history.step(&mut editor, true).unwrap(), Some(0));
        assert_eq!(editor.annotations(0).unwrap()[index].contents.as_deref(), Some("Review green"));
    }

    #[test]
    fn undo_redo_sticky_note_text() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        editor.add_annotation(0, &NewAnnot {
            kind: AnnotKind::Note { at: (60.0, 80.0), icon: NoteIcon::Comment },
            style: AnnotStyle { contents: Some("before".into()), ..Default::default() },
        }).unwrap();
        let index = editor.annotations(0).unwrap().into_iter().find(|a| a.subtype == "Text").unwrap().index;
        editor.set_annotation_text(0, index, "after").unwrap();
        let mut history = AnnotationHistory::default();
        history.record_comment(0, index, "before".into(), "after".into());
        assert_eq!(history.step(&mut editor, false).unwrap(), Some(0));
        assert_eq!(editor.annotations(0).unwrap()[index].contents.as_deref(), Some("before"));
        assert_eq!(history.step(&mut editor, true).unwrap(), Some(0));
        assert_eq!(editor.annotations(0).unwrap()[index].contents.as_deref(), Some("after"));
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

    #[test]
    fn added_annotations_undo_redo_on_real_pdf() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        let mut history = AnnotationHistory::default();
        let annot = NewAnnot { kind: AnnotKind::Note { at: (60.0, 80.0), icon: NoteIcon::Comment },
            style: AnnotStyle { contents: Some("review".into()), ..AnnotStyle::default() } };
        let base = editor.annotations(0).unwrap().len();
        let id = editor.add_annotation(0, &annot).unwrap();
        history.record_add(0, annot.clone(), id);
        assert_eq!(editor.annotations(0).unwrap().len(), base + 1);
        assert_eq!(history.step(&mut editor, false).unwrap(), Some(0));
        assert_eq!(Editor::open(editor.save_incremental().unwrap()).unwrap().annotations(0).unwrap().len(), base);
        assert_eq!(history.step(&mut editor, true).unwrap(), Some(0));
        assert_eq!(Editor::open(editor.save_incremental().unwrap()).unwrap().annotations(0).unwrap().len(), base + 1);
        assert_eq!(history.step(&mut editor, false).unwrap(), Some(0));
        let id = editor.add_annotation(0, &annot).unwrap();
        history.record_add(0, annot, id);
        assert!(!history.has_step(true), "a new annotation clears redo");
    }

    #[test]
    fn deleted_existing_annotation_undo_redo_on_real_pdf() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        let annot = NewAnnot { kind: AnnotKind::Note { at: (60.0, 80.0), icon: NoteIcon::Comment },
            style: AnnotStyle { contents: Some("existing review".into()), ..AnnotStyle::default() } };
        editor.add_annotation(0, &annot).unwrap();
        let mut editor = Editor::open(editor.save_incremental().unwrap()).unwrap();
        let original = editor.annotations(0).unwrap();
        let removed = editor.remove_annotation(0, 0).unwrap();
        let mut history = AnnotationHistory::default();
        history.record_delete(0, removed);
        assert!(editor.annotations(0).unwrap().is_empty());
        assert_eq!(history.step(&mut editor, false).unwrap(), Some(0));
        assert_eq!(editor.annotations(0).unwrap(), original);
        assert_eq!(history.step(&mut editor, true).unwrap(), Some(0));
        assert!(editor.annotations(0).unwrap().is_empty());
        assert_eq!(history.step(&mut editor, false).unwrap(), Some(0));
        assert_eq!(Editor::open(editor.save_incremental().unwrap()).unwrap().annotations(0).unwrap(), original);
    }

    #[test]
    fn add_then_delete_keeps_identity_through_undo_redo() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        let mut history = AnnotationHistory::default();
        let annot = NewAnnot { kind: AnnotKind::Note { at: (60.0, 80.0), icon: NoteIcon::Comment },
            style: AnnotStyle::default() };
        let id = editor.add_annotation(0, &annot).unwrap();
        history.record_add(0, annot, id);
        let removed = editor.remove_annotation(0, 0).unwrap();
        history.record_delete(0, removed);
        history.step(&mut editor, false).unwrap();
        history.step(&mut editor, false).unwrap();
        assert!(editor.annotations(0).unwrap().is_empty());
        history.step(&mut editor, true).unwrap();
        assert_eq!(editor.annotations(0).unwrap()[0].id, Some(id));
        history.step(&mut editor, true).unwrap();
        assert!(editor.annotations(0).unwrap().is_empty());
    }

    #[test]
    fn field_and_markup_share_one_undo_order_and_branch() {
        let mut editor = Editor::open(FORM.to_vec()).unwrap();
        let mut history = EditHistory::default();
        let annot = NewAnnot { kind: AnnotKind::Note { at: (60.0, 80.0), icon: NoteIcon::Comment },
            style: AnnotStyle { contents: Some("note".into()), ..Default::default() } };
        let id = editor.add_annotation(0, &annot).unwrap();
        history.record_add(0, annot, id);
        let original = value(&editor, "name");
        let changed = FieldValue::Text("Ada".into());
        editor.set_field("name", changed.clone()).unwrap();
        history.record_field("name", original.clone(), changed.clone());
        assert!(matches!(history.step(&mut editor, false).unwrap(), Some(EditStep::Field(n)) if n == "name"));
        assert_eq!(editor.annotations(0).unwrap().len(), 1);
        assert!(matches!(history.step(&mut editor, false).unwrap(), Some(EditStep::Annotation(0))));
        assert!(editor.annotations(0).unwrap().is_empty());
        assert!(matches!(history.step(&mut editor, true).unwrap(), Some(EditStep::Annotation(0))));
        assert!(matches!(history.step(&mut editor, true).unwrap(), Some(EditStep::Field(n)) if n == "name"));
        assert_eq!(value(&editor, "name"), changed);
        assert_eq!(Editor::open(editor.save_incremental().unwrap()).unwrap().annotations(0).unwrap().len(), 1);
        history.step(&mut editor, false).unwrap();
        let branch = FieldValue::Text("Grace".into());
        editor.set_field("name", branch.clone()).unwrap();
        history.record_field("name", original, branch.clone());
        assert!(history.step(&mut editor, true).unwrap().is_none(), "new edit clears both redo kinds");
        assert_eq!(value(&editor, "name"), branch);
    }
}
