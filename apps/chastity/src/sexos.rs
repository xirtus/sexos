// SPDX-License-Identifier: GPL-3.0-only
//! Chastity on SexOS: the window, its keys, the vault file in the program's folder.
use chastity_core::vault::Kdf;
use sex_std::sex_app::{self, Event, Nav};
use sex_std::sex_pdx;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::ui::{Host, Input, Ui};

const VAULT: &str = "vault.kdbx";
const NEW: &str = "vault.kdbx.new";
/// Locks after this long without a key.
const IDLE: Duration = Duration::from_secs(300);

struct SexOs;

impl Host for SexOs {
    fn load(&mut self) -> Option<Vec<u8>> {
        std::fs::read(VAULT).ok()
    }

    /// Written whole to a new file, made durable, then put in place: a crash leaves the
    /// old vault or the new one, never half of one.
    fn store(&mut self, bytes: &[u8]) -> Result<(), String> {
        use sex_std::fs::FileExt;
        use std::io::Write;
        let mut f = std::fs::File::create(NEW).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.fsync().map_err(|e| e.to_string())?;
        drop(f);
        sex_std::fs::rename(NEW, VAULT).map_err(|e| e.to_string())
    }

    /// Shell's clipboard broker takes copies only from catalog apps (Quil, Kaleidoscope):
    /// `sex_pdx::clipboard::Participant::from_catalog` refuses `App::Program`, and a copy
    /// starts from Shell's Export intent, not from the app. So an ordinary program cannot
    /// copy yet; this asks the broker's own rule and says so. When the broker admits
    /// programs, this becomes `clipboard_client::Client::publish` as in Kaleidoscope.
    fn copy(&mut self, _text: &str) -> Result<(), String> {
        let me = sex_pdx::app_lifecycle::catalog(sex_pdx::app_lifecycle::SELF_CATALOG_SLOT).ok();
        match me.and_then(sex_pdx::clipboard::Participant::from_catalog) {
            None => Err("the clipboard does not take copies from ordinary programs yet".into()),
            Some(_) => Err("copying from Chastity is not wired to the clipboard yet".into()),
        }
    }

    fn record(&mut self, line: &str) {
        sex_std::record(line);
    }

    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

fn input(key: sex_pdx::app_key::Key) -> Option<Input> {
    Some(match (key.code, sex_app::nav(key)) {
        (0x1C, _) => Input::Enter,
        (0x0E, _) => Input::Backspace,
        (0x01, _) => Input::Escape,
        (_, Some(Nav::Up)) => Input::Up,
        (_, Some(Nav::Down)) => Input::Down,
        (_, Some(Nav::Delete)) => Input::Delete,
        _ if key.control => return None,
        _ => Input::Char(sex_app::text(key).filter(|c| !c.is_control())?),
    })
}

// Pure Weft is deliberate: std owns the allocator and the existing window.
use chastity_core::{autofill::Candidate, vault::EntryId};
use sex_app::autofill::Provider;
use weft::paint::Painter;
use weft::text::{Face, Fonts};
use weft::theme::Color;
use weft::{Id, Layout, Response, Strings, Theme, Ui as Weft};
use zeroize::Zeroizing;
struct Consent {
    token: u32,
    origin: String,
    candidates: Vec<Candidate>,
    index: usize,
    body: String,
}
struct Scene {
    ui: Weft,
    deny: Id,
    allow: Id,
    next: Id,
    lock: Id,
    screen: Id,
    field: Id,
    actions: Vec<(Id, Input)>,
}
fn scene(size: (usize, usize), consent: Option<&Consent>, pending: bool) -> Scene {
    let mut ui = Weft::new(
        size.0 as u32,
        size.1 as u32,
        Theme::light(),
        Strings::new("en"),
    );
    let root = ui.root();
    ui.set_layout(root, Layout::column().padding(16.0).gap(10.0));
    ui.label(root, "Chastity — KeePass vault");
    let lock = ui.button(
        root,
        if pending {
            "Lock pending acknowledgement…"
        } else {
            "Lock vault"
        },
    );
    ui.set_enabled(lock, !pending);
    let mut actions = Vec::new();
    if consent.is_none() && !pending {
        let row = ui.row(root);
        for (label, key) in [
            ("Add", Input::Char('a')),
            ("Open", Input::Enter),
            ("Search", Input::Char('/')),
            ("Back", Input::Escape),
            ("Next", Input::Down),
            ("Previous", Input::Up),
        ] {
            actions.push((ui.button(row, label), key));
        }
    }
    let screen = ui.paragraph(root, "");
    let field = ui.field(root, "", "Type with the keyboard; Enter continues");
    ui.set_visible(field, consent.is_none() && !pending);
    let (deny, allow, next) = if let Some(c) = consent {
        ui.label(root, "Autofill request — release this login once?");
        ui.paragraph(root, &c.body);
        let ready = crate::consent::reviewable(size.0, size.1, &c.body);
        if !ready {
            ui.paragraph(root,"Enlarge Chastity to at least 800 × 600 to review the complete request. Allow is disabled.");
        }
        let row = ui.row(root);
        let (deny, allow) = crate::consent::buttons(&mut ui, row, size, &c.body);
        let next = ui.button(row, "Next matching entry");
        ui.set_enabled(allow, ready);
        ui.set_enabled(next, c.candidates.len() > 1);
        ui.focus(Some(deny));
        (deny, allow, next)
    } else {
        (lock, lock, lock)
    };
    Scene {
        ui,
        deny,
        allow,
        next,
        lock,
        screen,
        field,
        actions,
    }
}
fn weft_event(e: Event) -> weft::input::Event {
    use weft::input::{Button, Event as E, Key, Pointer, PointerKind as K};
    match e {
        Event::Key(k) => E::Key(Key {
            code: k.code,
            shift: k.shift,
            control: k.control,
            text: k.char(),
        }),
        Event::Close => E::Close,
        Event::Resize { width, height } => E::Resize {
            width: width as u32,
            height: height as u32,
        },
        Event::Pointer(p) => {
            let button = |b| match b {
                1 => Button::Middle,
                2 => Button::Right,
                _ => Button::Left,
            };
            let kind = match p.kind {
                sex_pdx::app_pointer::Kind::Move => K::Move,
                sex_pdx::app_pointer::Kind::Down { button: b } => K::Down(button(b)),
                sex_pdx::app_pointer::Kind::Up { button: b } => K::Up(button(b)),
                sex_pdx::app_pointer::Kind::Wheel { steps } => K::Wheel(steps),
                sex_pdx::app_pointer::Kind::Leave => K::Leave,
            };
            E::Pointer(Pointer {
                x: p.x as f32,
                y: p.y as f32,
                kind,
            })
        }
    }
}
fn epoch() -> Option<u64> {
    let mut b = [0; 8];
    chastity_core::entropy::fill(&mut b).ok()?;
    let n = u64::from_le_bytes(b);
    (n != 0).then_some(n)
}
pub fn main() {
    let mut host = SexOs;
    let (target, task) = sex_std::identity();
    sex_std::record(&format!("[chastity.window] target={target} task={task}"));
    let window = sex_std::window();
    window.title(b"Chastity");
    let mut model = Ui::new(&mut host, Kdf::STANDARD);
    let mut provider: Option<Provider> = None;
    let mut consent: Option<Consent> = None;
    let mut closing = false;
    let mut lock_reason = "asked";
    let mut last_key = Instant::now();
    let mut fonts = Fonts::new();
    let mut view = scene(window.size(), None, false);
    let mut ready_logged = false;
    loop {
        let now = sex_pdx::get_ticks();
        if model.unlocked() && provider.is_none() {
            provider = epoch().and_then(|n| Provider::new(window, n).ok());
            if provider.is_none() {
                model.lock(&mut host, "entropy unavailable");
            }
        }
        // Dropping the canonical vault is immediate; the old provider survives only
        // as a zeroized revocation state machine until the exact run/epoch ACK.
        if !model.unlocked() {
            if let Some(p) = &mut provider {
                if !p.closing() {
                    p.close();
                    consent = None;
                    view = scene(window.size(), None, true);
                    window.pixels().fill(0);
                }
            } else if closing {
                break;
            }
        }
        if let Some(p) = &mut provider {
            p.poll(now);
            if p.registered() && !ready_logged {
                sex_std::record("[chastity.autofill.ready]");
                ready_logged = true;
            }
            if p.revoked() {
                provider = None;
                consent = None;
                ready_logged = false;
                sex_std::record(&format!("[chastity.locked] reason={lock_reason}"));
                if closing {
                    break;
                }
                view = scene(window.size(), None, false);
            }
        }
        if let Some(p) = &mut provider {
            if let Some(o) = p.offer() {
                if consent.as_ref().map(|c| c.token) != Some(o.token) {
                    let candidates = model.candidates(o.origin);
                    if let Some(first) = candidates.first() {
                        if let Some(body) = crate::consent::body(
                            o.name,
                            o.consumer.pd,
                            o.consumer.task,
                            o.origin,
                            &first.title,
                            &first.username,
                        ) {
                            sex_std::record(&format!(
                                "[chastity.autofill.prompt] token={} domain={} task={}",
                                o.token, o.consumer.pd, o.consumer.task
                            ));
                            consent = Some(Consent {
                                token: o.token,
                                origin: o.origin.into(),
                                candidates,
                                index: 0,
                                body,
                            });
                            view = scene(window.size(), consent.as_ref(), false);
                        } else {
                            p.deny();
                            sex_std::record("[chastity.autofill.refused] reason=scope_size");
                        }
                    } else {
                        p.deny();
                        sex_std::record("[chastity.autofill.refused] reason=no_match");
                    }
                }
            } else if consent.take().is_some() {
                view = scene(window.size(), None, p.closing());
            }
        }
        let (w, h) = window.size();
        // Shared Weft lays out and paints the actual screen, controls and masked
        // input. Typed secrets stay in the canonical bounded zeroizing editor;
        // Weft receives only stars, so its ordinary undo history holds no secrets.
        if consent.is_none() {
            let text = if provider.as_ref().is_some_and(Provider::closing) {
                Zeroizing::new(
                    "Locking: waiting for broker acknowledgement. Unlock is disabled.".into(),
                )
            } else {
                model.render_public(&host)
            };
            view.ui.set_text(view.screen, &text);
            view.ui.set_visible(
                view.field,
                model.typing() && !provider.as_ref().is_some_and(Provider::closing),
            );
            view.ui.set_text(view.field, &model.masked_line());
        }
        view.ui.resize(w as u32, h as u32);
        let pixels = window.pixels();
        pixels.fill(0xfff7f7f7);
        view.ui.paint(&mut pixels[..w * h]);
        // Explicitly revealed password/TOTP is transient: never copied into a
        // retained widget String. Weft's shared Painter draws the disclosure.
        if consent.is_none() && model.unlocked() {
            let text = model.disclosed(&host);
            let y = view.ui.rect(view.screen).bottom() + 20.0;
            let mut painter = Painter::new(pixels, w, h, 1.0);
            painter.text(
                &mut fonts,
                Face::Mono,
                14.0,
                16.0,
                y,
                &text,
                Color(0xff202020),
            );
        }
        window.show_pixels(w, h);
        let event = window.next_within(
            if provider.as_ref().is_some_and(|p| p.busy() || p.closing()) {
                1
            } else {
                100
            },
        );
        if let Some(event) = event {
            if matches!(event, Event::Close) {
                lock_reason = "closed";
                model.lock(&mut host, "closed");
                closing = true;
                continue;
            }
            if let Event::Resize { .. } = event {
                view = scene(
                    window.size(),
                    consent.as_ref(),
                    provider.as_ref().is_some_and(Provider::closing),
                );
                sex_std::record(&format!(
                    "[chastity.layout] width={} height={}",
                    window.size().0,
                    window.size().1
                ));
                continue;
            }
            last_key = Instant::now();
            if provider.as_ref().is_some_and(Provider::closing) {
                continue;
            }
            if consent.is_none() {
                if let Event::Key(k) = event {
                    if let Some(i) = input(k) {
                        model.key(&mut host, i);
                    }
                    continue;
                }
            }
            for response in view.ui.handle(weft_event(event)) {
                if let Response::Clicked(id) = response {
                    if let Some((_, key)) = view.actions.iter().find(|(button, _)| *button == id) {
                        model.key(&mut host, *key);
                        continue;
                    }
                    if id == view.lock {
                        lock_reason = "asked";
                        model.lock(&mut host, "asked");
                        continue;
                    }
                    let Some(c) = &mut consent else { continue };
                    let Some(p) = &mut provider else { continue };
                    if id == view.deny {
                        let token = c.token;
                        p.deny();
                        consent = None;
                        view = scene(window.size(), None, false);
                        sex_std::record(&format!("[chastity.autofill.denied] token={token}"));
                    } else if id == view.next {
                        c.index = (c.index + 1) % c.candidates.len();
                        let chosen = &c.candidates[c.index];
                        let o = p.offer().unwrap();
                        if let Some(body) = crate::consent::body(
                            o.name,
                            o.consumer.pd,
                            o.consumer.task,
                            &c.origin,
                            &chosen.title,
                            &chosen.username,
                        ) {
                            c.body = body;
                            view = scene(window.size(), Some(c), false);
                        } else {
                            p.deny();
                            consent = None;
                            view = scene(window.size(), None, false);
                        }
                    } else if id == view.allow
                        && crate::consent::reviewable(window.size().0, window.size().1, &c.body)
                    {
                        // Recheck token, origin, chosen entry and geometry at action
                        // time. A resize or cancelled request cannot reuse a dialog.
                        if p.offer()
                            .is_some_and(|o| o.token == c.token && o.origin == c.origin)
                        {
                            let chosen: EntryId = c.candidates[c.index].id;
                            if let Some(fill) = model.fill(&c.origin, chosen) {
                                if p.allow(&fill.username, &fill.password, now).is_ok() {
                                    sex_std::record(&format!(
                                        "[chastity.autofill.approved] token={}",
                                        c.token
                                    ));
                                }
                            }
                        }
                        consent = None;
                        view = scene(window.size(), None, false);
                    }
                }
            }
        } else if model.unlocked() && last_key.elapsed() >= IDLE {
            lock_reason = "idle";
            model.lock(&mut host, "idle");
        }
    }
    window.pixels().fill(0);
    window.show(b"");
    sex_std::record("[chastity.closed]");
    std::process::exit(0)
}
