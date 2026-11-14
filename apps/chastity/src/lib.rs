// SPDX-License-Identifier: GPL-3.0-only
//! Chastity's retained Weft window. Password keystrokes are intercepted by the
//! platform adapter; Weft receives only dots and never stores the secret text.
#![no_std]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use chastity_core::{Entry, Error, Random, Session, Vault};
fn secret_buffer(value: &str) -> Zeroizing<String> {
    let mut s = Zeroizing::new(String::with_capacity(chastity_core::MAX_FIELD));
    s.push_str(value);
    s
}
use weft::{Id, Layout, Response, Rows, Ui};
use zeroize::{Zeroize, Zeroizing};
pub struct View {
    pub file: Id,
    pub master: Id,
    pub confirmation: Id,
    pub password: Id,
    pub unlock: Id,
    pub create: Id,
    pub import: Id,
    pub generate: Id,
    pub save: Id,
    pub add: Id,
    pub lock: Id,
    pub load: Id,
    pub delete: Id,
    pub title: Id,
    pub origin: Id,
    pub username: Id,
    pub list: Id,
    pub status: Id,
    locked: Id,
    opened: Id,
    pub username_secret: Zeroizing<String>,
    pub master_secret: Zeroizing<String>,
    pub confirmation_secret: Zeroizing<String>,
    pub entry_secret: Zeroizing<String>,
    pub session: Session,
    pub dirty: bool,
}
impl View {
    pub fn build(ui: &mut Ui) -> Self {
        let root = ui.root();
        ui.set_layout(root, Layout::column().padding(8.0).gap(4.0));
        ui.label(root, "Chastity — encrypted password vault");
        let file = ui.field(
            root,
            "chastity.vault",
            "Vault filename (save to a new snapshot name)",
        );
        ui.set_max_length(file, Some(180));
        let locked = ui.column(root);
        ui.label(locked, "Master password");
        let master = ui.password(locked, "Master password");
        let confirmation = ui.password(locked, "Repeat password when creating a new vault");
        let buttons = ui.row(locked);
        let unlock = ui.button(buttons, "Unlock");
        let create = ui.button(buttons, "Create vault");
        let import = ui.button(buttons, "Import KDBX 4");
        let opened = ui.row(root);
        ui.set_layout(opened, Layout::row().gap(8.0).fill());
        let sidebar = ui.column(opened);
        ui.set_layout(sidebar, Layout::column().width(170.0).gap(6.0));
        let list = ui.list(sidebar, &[]);
        ui.set_layout(list, Layout::default().fill());
        let row = ui.row(sidebar);
        let load = ui.button(row, "Load");
        let delete = ui.button(row, "Delete");
        let form = ui.column(opened);
        ui.set_layout(form, Layout::column().gap(3.0).fill());
        let title = ui.field(form, "", "Entry title");
        let origin = ui.field(
            opened,
            "",
            "Exact account origin (e.g. https://example.org)",
        );
        let username = ui.password(form, "Username (protected input)");
        let password = ui.password(form, "Password");
        for field in [title, origin, username] {
            ui.set_max_length(field, Some(4096));
        }
        let row = ui.row(form);
        let generate = ui.button(row, "Generate 24 characters");
        let add = ui.button(row, "Add entry");
        let row = ui.row(form);
        let save = ui.button(row, "Save new snapshot");
        let lock = ui.button(row, "Lock");
        let status = ui.paragraph(
            root,
            "Locked. Saves create new encrypted snapshots; existing files are preserved.",
        );
        ui.set_visible(opened, false);
        ui.focus(Some(master));
        Self {
            file,
            master,
            confirmation,
            password,
            unlock,
            create,
            import,
            generate,
            save,
            add,
            lock,
            load,
            delete,
            title,
            origin,
            username,
            list,
            status,
            locked,
            opened,
            username_secret: secret_buffer(""),
            master_secret: secret_buffer(""),
            confirmation_secret: secret_buffer(""),
            entry_secret: secret_buffer(""),
            session: Session::default(),
            dirty: false,
        }
    }
    pub fn filename(&self, ui: &Ui) -> Result<Vec<u8>, Error> {
        let name = ui.text_of(self.file);
        if name.is_empty()
            || name.len() > 180
            || name.bytes().any(|b| b == 0 || b == b'/' || b == b'@')
            || name == "."
            || name == ".."
        {
            return Err(Error::Format);
        }
        Ok(name.as_bytes().to_vec())
    }
    pub fn secret_key(&mut self, ui: &mut Ui, code: u8, control: bool, ch: Option<char>) -> bool {
        let Some(id) = ui.focused() else {
            return false;
        };
        let secret = if id == self.master {
            &mut self.master_secret
        } else if id == self.confirmation {
            &mut self.confirmation_secret
        } else if id == self.username {
            &mut self.username_secret
        } else if id == self.password {
            &mut self.entry_secret
        } else {
            return false;
        };
        if code == 15 {
            return false;
        } // Tab belongs to toolkit navigation.
        if control && code == 30 {
            secret.zeroize();
        }
        // Ctrl+A clears the secret field.
        else if code == 14 {
            secret.pop();
        } else if !control {
            if let Some(ch) = ch.filter(|c| !c.is_control()) {
                if secret.len() + ch.len_utf8() <= 4096 {
                    secret.push(ch);
                }
            }
        }
        ui.set_text(id, &"•".repeat(secret.chars().count()));
        true
    }
    pub fn opened(&mut self, ui: &mut Ui, vault: Vault) {
        self.session.unlock(vault);
        self.confirmation_secret.zeroize();
        ui.set_text(self.confirmation, "");
        ui.set_text(self.master, "");
        ui.set_visible(self.locked, false);
        ui.set_visible(self.opened, true);
        self.refresh(ui);
        ui.focus(Some(self.title));
    }
    pub fn refresh(&self, ui: &mut Ui) {
        if let Ok(vault) = self.session.vault() {
            let rows = vault
                .entries()
                .iter()
                .map(|e| alloc::vec![format!("{} — {}", e.title, e.origin)])
                .collect();
            ui.set_rows(self.list, Rows(rows));
        }
    }
    pub fn lock_now(&mut self, ui: &mut Ui) {
        self.session.lock();
        self.master_secret.zeroize();
        self.confirmation_secret.zeroize();
        self.entry_secret.zeroize();
        self.username_secret.zeroize();
        for id in [
            self.master,
            self.confirmation,
            self.password,
            self.title,
            self.origin,
            self.username,
        ] {
            ui.set_text(id, "");
        }
        ui.set_rows(self.list, Rows(Vec::new()));
        ui.set_visible(self.opened, false);
        ui.set_visible(self.locked, true);
        ui.set_text(self.status, "Locked. Unsaved changes were discarded.");
        self.dirty = false;
        ui.focus(Some(self.master));
    }
    pub fn local_action(
        &mut self,
        ui: &mut Ui,
        response: &Response,
        rng: &mut impl Random,
    ) -> Result<bool, Error> {
        let Response::Clicked(id) = response else {
            return Ok(false);
        };
        if *id == self.lock {
            self.lock_now(ui);
            return Ok(true);
        }
        if *id == self.generate {
            let generated = chastity_core::generate(24, rng)?;
            self.entry_secret.zeroize();
            self.entry_secret.push_str(&generated);
            ui.set_text(self.password, &"•".repeat(24));
            ui.set_text(
                self.status,
                "Generated from cryptographic entropy. Add the entry to retain it.",
            );
            return Ok(true);
        }
        if *id == self.add {
            let entry = Entry::new(
                ui.text_of(self.title),
                ui.text_of(self.origin),
                &self.username_secret,
                &self.entry_secret,
            )?;
            self.session.vault_mut()?.add(entry)?;
            self.entry_secret.zeroize();
            ui.set_text(self.password, "");
            self.dirty = true;
            self.refresh(ui);
            ui.set_text(
                self.status,
                "Entry added. Save to a new snapshot filename before locking.",
            );
            return Ok(true);
        }
        if *id == self.delete {
            let index = ui.selected_row(self.list).ok_or(Error::Format)?;
            self.session.vault_mut()?.remove(index)?;
            self.dirty = true;
            self.refresh(ui);
            ui.set_text(
                self.status,
                "Entry removed in memory. Save a new snapshot to retain this change.",
            );
            return Ok(true);
        }
        if *id == self.load {
            let index = ui.selected_row(self.list).ok_or(Error::Format)?;
            let e = self
                .session
                .vault()?
                .entries()
                .get(index)
                .ok_or(Error::Format)?;
            ui.set_text(self.title, &e.title);
            ui.set_text(self.origin, &e.origin);
            self.username_secret = secret_buffer(&e.username);
            ui.set_text(
                self.username,
                &"•".repeat(self.username_secret.chars().count()),
            );
            self.entry_secret = secret_buffer(&e.password);
            ui.set_text(
                self.password,
                &"•".repeat(self.entry_secret.chars().count()),
            );
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use weft::{Strings, Theme};
    struct R(u8);
    impl Random for R {
        fn fill(&mut self, out: &mut [u8]) -> Result<(), Error> {
            for b in out {
                *b = self.0;
                self.0 = self.0.wrapping_add(1);
            }
            Ok(())
        }
    }
    #[test]
    fn password_never_enters_toolkit_edit_history() {
        let mut ui = Ui::new(800, 900, Theme::light(), Strings::new("en"));
        let mut v = View::build(&mut ui);
        ui.focus(Some(v.master));
        for c in "huntersecret".chars() {
            assert!(v.secret_key(&mut ui, 30, false, Some(c)));
        }
        assert_eq!(&*v.master_secret, "huntersecret");
        assert_eq!(ui.text_of(v.master), "••••••••••••");
        assert!(v.secret_key(&mut ui, 30, true, None));
        assert!(v.master_secret.is_empty());
        assert!(ui.text_of(v.master).is_empty());
        // A toolkit undo has never seen the password or its previous substrings.
        ui.handle(weft::Event::Key(weft::Key {
            code: 44,
            shift: false,
            control: true,
            text: None,
        }));
        assert!(ui.text_of(v.master).is_empty());
    }
    #[test]
    fn create_add_lock_wipes_form_and_session() {
        let mut ui = Ui::new(800, 900, Theme::light(), Strings::new("en"));
        let mut v = View::build(&mut ui);
        v.master_secret.push_str("mastersecret");
        v.opened(&mut ui, Vault::default());
        ui.set_text(v.title, "test");
        ui.set_text(v.origin, "https://example.org");
        v.username_secret.push_str("alice");
        let generate = v.generate;
        v.local_action(&mut ui, &Response::Clicked(generate), &mut R(1))
            .unwrap();
        assert_eq!(v.entry_secret.len(), 24);
        let add = v.add;
        v.local_action(&mut ui, &Response::Clicked(add), &mut R(1))
            .unwrap();
        assert_eq!(v.session.vault().unwrap().entries().len(), 1);
        assert!(v.entry_secret.is_empty());
        v.lock_now(&mut ui);
        assert!(v.master_secret.is_empty());
        assert!(v.session.vault().is_err());
        assert!(ui.text_of(v.username).is_empty());
        assert!(ui.text_of(v.password).is_empty());
    }
}
#[cfg(test)]
mod layout_tests {
    use super::*;
    #[test]
    fn vault_controls_fit_the_default_window() {
        let mut ui = Ui::new(640, 400, weft::Theme::light(), weft::Strings::new("en"));
        let mut view = View::build(&mut ui);
        view.opened(&mut ui, Vault::default());
        let mut pixels = alloc::vec![0;640*400];
        ui.paint(&mut pixels);
        for id in [view.generate, view.add, view.save, view.lock, view.password] {
            let r = ui.rect(id);
            assert!(r.y + r.h <= 400.0, "{id:?} {r:?}");
            assert!(r.x + r.w <= 640.0, "{id:?} {r:?}");
        }
    }
}
#[cfg(test)]
mod secret_allocation_tests {
    use super::*;
    #[test]
    fn typed_secret_never_reallocates_plaintext_prefixes() {
        let mut ui = Ui::new(640, 400, weft::Theme::light(), weft::Strings::new("en"));
        let mut v = View::build(&mut ui);
        ui.focus(Some(v.master));
        let allocation = v.master_secret.as_ptr();
        for _ in 0..chastity_core::MAX_FIELD {
            v.secret_key(&mut ui, 30, false, Some('a'));
        }
        assert_eq!(v.master_secret.as_ptr(), allocation);
        assert_eq!(v.master_secret.len(), chastity_core::MAX_FIELD);
        v.secret_key(&mut ui, 30, false, Some('b'));
        assert_eq!(v.master_secret.len(), chastity_core::MAX_FIELD);
        v.lock_now(&mut ui);
        assert!(v.master_secret.is_empty());
    }
}
