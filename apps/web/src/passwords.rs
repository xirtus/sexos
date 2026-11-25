// SPDX-License-Identifier: GPL-3.0-only
//! Explicit Chastity fill for one shown HTTPS document. Nothing is saved in
//! Kaleidoscope's profile or logged from the returned credentials.
use super::Browser;
use alloc::string::String;
use sex_app::autofill::{Client, Error, SaveClient};
use sex_web::autofill_policy::{self, Login, Refusal};
use sex_web::sex_html::{self, style::Display};

pub(super) struct Pending {
    client: Client,
    origin: String,
    owner: u64,
    tab: usize,
    username: usize,
    password: usize,
    cancelled: bool,
}
pub(super) struct PendingSave {
    client: SaveClient,
    origin: String,
    owner: u64,
    tab: usize,
    username: usize,
    password: usize,
    cancelled: bool,
}

fn visible(browser: &Browser, styles: &[Option<sex_html::style::Style>], id: usize) -> bool {
    let styled = styles.get(id).and_then(Option::as_ref).is_some_and(|s| {
        s.display != Display::None && !s.hidden && s.opacity != 0 && s.pointer_events
    });
    styled && browser.page.boxes.get(id).and_then(|b| *b).is_some_and(|[x, y, w, h]| {
        w > 0 && h > 0 && x < browser.view.0 && x.saturating_add(w) > 0
            && y < browser.scroll.saturating_add(browser.view.1)
            && y.saturating_add(h) > browser.scroll
    })
}

fn ready(browser: &Browser) -> Result<Login, Refusal> {
    let pair = autofill_policy::login(&browser.doc.get_ref().dom, &browser.url, browser.in_frame.is_some())?;
    let sheets: alloc::vec::Vec<&str> = browser.sheets.iter().map(String::as_str).collect();
    let (styles, _) = sex_html::styles_at(&browser.doc.get_ref().dom, &sheets, browser.view.0, browser.view.1,
        sex_pdx::get_ticks(), &mut sex_html::Timeline::default());
    if !visible(browser, &styles, pair.username) || !visible(browser, &styles, pair.password) { return Err(Refusal::NoLogin); }
    Ok(pair)
}

impl Browser {
    pub(super) fn request_autofill(&mut self) {
        if self.autofill.is_some() || self.password_save.is_some() {
            self.status = Some("Finish the current password request first");
            self.draw();
            return;
        }
        let pair = match ready(self) {
            Ok(pair) => pair,
            Err(reason) => {
                self.status = Some(match reason {
                    Refusal::Insecure | Refusal::Address => "Passwords require an HTTPS page",
                    Refusal::Frame => "Passwords cannot fill an embedded page",
                    Refusal::UnsafeAction => "This form does not submit safely to this site",
                    Refusal::NoLogin | Refusal::Ambiguous => "No eligible login form is visible",
                    Refusal::Empty | Refusal::Oversize => "This login has no bounded credentials to save",
                });
                sex_pdx::serial_record!("[web.autofill.refused] reason={:?}", reason);
                self.draw();
                return;
            }
        };
        let client = match Client::request(&self.window, &pair.origin, sex_pdx::get_ticks()) {
            Ok(client) => client,
            Err(why) => {
                self.status = Some("Password request unavailable");
                sex_pdx::serial_record!("[web.autofill.refused] reason={:?}", why);
                self.draw();
                return;
            }
        };
        self.autofill = Some(Pending {
            client, origin: pair.origin, owner: self.doc.get_ref().owner(), tab: self.active,
            username: pair.username, password: pair.password, cancelled: false,
        });
        self.status = Some("Approve the password request in Chastity");
        sex_pdx::serial_record!("[web.autofill.requested] tab={}", self.active);
        self.draw();
    }

    pub(super) fn request_password_save(&mut self) {
        if self.autofill.is_some() || self.password_save.is_some() {
            self.status = Some("Finish the current password request first");
            self.draw();
            return;
        }
        if let Err(reason) = ready(self) {
            self.status = Some(match reason {
                Refusal::Insecure | Refusal::Address => "Passwords require an HTTPS page",
                Refusal::Frame => "Passwords cannot save from an embedded page",
                Refusal::UnsafeAction => "This form does not submit safely to this site",
                _ => "No eligible login form is visible",
            });
            sex_pdx::serial_record!("[web.password_save.refused] reason={:?}", reason);
            self.draw();
            return;
        }
        let captured = match autofill_policy::save_login(&self.doc.get_ref().dom, &self.url, self.in_frame.is_some()) {
            Ok(captured) => captured,
            Err(reason) => {
                self.status = Some("Enter a username and password in the visible form");
                sex_pdx::serial_record!("[web.password_save.refused] reason={:?}", reason);
                self.draw();
                return;
            }
        };
        let client = match SaveClient::request(&self.window, &captured.origin, &captured.username, &captured.password, sex_pdx::get_ticks()) {
            Ok(client) => client,
            Err(why) => {
                self.status = Some("Password save request unavailable");
                sex_pdx::serial_record!("[web.password_save.refused] reason={:?}", why);
                self.draw();
                return;
            }
        };
        self.password_save = Some(PendingSave {
            client, origin: captured.origin.clone(), owner: self.doc.get_ref().owner(), tab: self.active,
            username: captured.username_node, password: captured.password_node, cancelled: false,
        });
        self.status = Some("Waiting for Chastity to review the save request");
        sex_pdx::serial_record!("[web.password_save.requested] tab={}", self.active);
        self.draw();
    }

    pub(super) fn cancel_password_save(&mut self) {
        if let Some(p) = self.password_save.as_mut() {
            if !p.cancelled {
                p.client.cancel();
                p.cancelled = true;
                sex_pdx::serial_record!("[web.password_save.cancelled] reason=page_changed");
            }
        }
    }

    pub(super) fn poll_password_save(&mut self) {
        let Some(mut pending) = self.password_save.take() else { return; };
        let valid = !pending.cancelled && pending.tab == self.active
            && pending.owner == self.doc.get_ref().owner()
            && ready(self).is_ok_and(|pair| pair.origin == pending.origin
                && pair.username == pending.username && pair.password == pending.password)
            && {
                let values = &self.doc.get_ref().dom.values;
                values.get(&pending.username).zip(values.get(&pending.password))
                    .is_some_and(|(u, p)| pending.client.matches(u, p))
            };
        if !valid && !pending.cancelled {
            pending.client.cancel();
            pending.cancelled = true;
            sex_pdx::serial_record!("[web.password_save.cancelled] reason=identity_or_values_changed");
        }
        match pending.client.poll(sex_pdx::get_ticks()) {
            None => { if pending.client.active() { self.password_save = Some(pending); } }
            Some(Err(why)) => {
                if !pending.cancelled {
                    self.status = Some(match why {
                        Error::Locked | Error::Unavailable => "Unlock Chastity to review password saves",
                        Error::Declined => "Password save declined",
                        _ => "Password save request expired or failed",
                    });
                    sex_pdx::serial_record!("[web.password_save.refused] reason={:?}", why);
                    self.draw();
                }
                if why == Error::Stale { pending.cancelled = true; }
                if pending.client.active() { self.password_save = Some(pending); }
            }
            Some(Ok(())) => {
                if valid {
                    self.status = Some("Password saved in Chastity");
                    sex_pdx::serial_record!("[web.password_save.approved]");
                    self.draw();
                }
            }
        }
    }

    pub(super) fn cancel_autofill(&mut self) {
        if let Some(p) = self.autofill.as_mut() {
            if !p.cancelled {
                p.client.cancel();
                p.cancelled = true;
                sex_pdx::serial_record!("[web.autofill.cancelled] reason=page_changed");
            }
        }
    }

    pub(super) fn poll_autofill(&mut self) {
        let Some(mut pending) = self.autofill.take() else { return; };
        if !pending.cancelled && (pending.tab != self.active || pending.owner != self.doc.get_ref().owner()
            || autofill_policy::origin(&self.url, self.in_frame.is_some()).ok().as_deref() != Some(pending.origin.as_str())) {
            pending.client.cancel();
            pending.cancelled = true;
            sex_pdx::serial_record!("[web.autofill.cancelled] reason=identity_changed");
        }
        match pending.client.poll(sex_pdx::get_ticks()) {
            None => {
                if pending.client.active() { self.autofill = Some(pending); }
                else if pending.cancelled { sex_pdx::serial_record!("[web.autofill.cleared]"); }
            }
            Some(Err(why)) => {
                if !pending.cancelled {
                    self.status = Some(match why {
                        Error::Locked | Error::Unavailable => "Unlock Chastity to fill passwords",
                        Error::Declined => "Password request declined",
                        _ => "Password request expired or failed",
                    });
                    sex_pdx::serial_record!("[web.autofill.refused] reason={:?}", why);
                    self.draw();
                }
                if why == Error::Stale { pending.cancelled = true; }
                if pending.client.active() { self.autofill = Some(pending); }
                else if pending.cancelled { sex_pdx::serial_record!("[web.autofill.cleared]"); }
            }
            Some(Ok(credentials)) => {
                let valid = !pending.cancelled && pending.tab == self.active
                    && pending.owner == self.doc.get_ref().owner()
                    && ready(self).is_ok_and(|pair| pair.origin == pending.origin
                        && pair.username == pending.username && pair.password == pending.password);
                if valid {
                    credentials.with(|user, pass| {
                        let dom = &mut self.doc.get().dom;
                        dom.values.insert(pending.username, String::from(user));
                        dom.values.insert(pending.password, String::from(pass));
                    });
                    self.relayout();
                    self.status = Some("Password filled from Chastity");
                    sex_pdx::serial_record!("[web.autofill.filled] fields=2");
                    self.draw();
                } else {
                    sex_pdx::serial_record!("[web.autofill.cancelled] reason=stale_document");
                }
            }
        }
    }
}
