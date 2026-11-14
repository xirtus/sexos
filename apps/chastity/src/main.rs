// SPDX-License-Identifier: GPL-3.0-only
#![cfg_attr(feature = "sexos", no_std)]
#![cfg_attr(feature = "sexos", no_main)]
#[cfg(feature = "sexos")]
mod program {
    extern crate alloc;
    use alloc::{format, string::String, vec, vec::Vec};
    use chastity::View;
    use chastity_core::{Error, Random, Vault, MAX_FILE};
    use sex_app::{files, Event};
    use weft::{sexos::App, Response, Strings, Theme};
    use zeroize::Zeroize;
    #[panic_handler]
    fn panic(_: &core::panic::PanicInfo) -> ! {
        sex_pdx::serial_record!("[chastity.fatal]");
        sex_pdx::task_finish(1)
    }
    struct Entropy;
    impl Random for Entropy {
        fn fill(&mut self, out: &mut [u8]) -> Result<(), Error> {
            // RDSEED is the CPU entropy source already used by SexOS's typed source.
            // No time/counter fallback is ever used for vault material.
            if core::arch::x86_64::__cpuid_count(7, 0).ebx & (1 << 18) == 0 {
                return Err(Error::Entropy);
            }
            for chunk in out.chunks_mut(8) {
                let mut value = 0u64;
                let mut good = false;
                for _ in 0..64 {
                    let ok: u8;
                    unsafe {
                        core::arch::asm!("rdseed {value}","setc {ok}",value=out(reg)value,ok=out(reg_byte)ok,options(nomem,nostack));
                    }
                    if ok != 0 {
                        good = true;
                        break;
                    }
                    sex_pdx::sys_yield();
                }
                if !good {
                    out.zeroize();
                    return Err(Error::Entropy);
                }
                chunk.copy_from_slice(&value.to_le_bytes()[..chunk.len()]);
                value.zeroize();
            }
            Ok(())
        }
    }
    fn read(app: &mut App, path: &[u8]) -> Result<Vec<u8>, Error> {
        let length = files::size(&mut app.window, path).map_err(|_| Error::Format)? as usize;
        if length > MAX_FILE {
            return Err(Error::Limit);
        }
        let mut data = vec![0u8; length];
        let n = files::read_at(&mut app.window, path, 0, &mut data).map_err(|_| Error::Format)?;
        if n != length {
            return Err(Error::Format);
        }
        Ok(data)
    }
    fn save(app: &mut App, path: &[u8], data: &[u8], rng: &mut impl Random) -> Result<(), Error> {
        // Exclusive create + exclusive rename prevents clobbering a prior snapshot.
        let mut nonce = [0u8; 8];
        rng.fill(&mut nonce)?;
        let temp = format!("chastity-{:016x}.partial", u64::from_le_bytes(nonce));
        files::create(&mut app.window, temp.as_bytes()).map_err(|_| Error::Format)?;
        let n = files::write_at(&mut app.window, temp.as_bytes(), 0, data)
            .map_err(|_| Error::Format)?;
        if n != data.len() {
            return Err(Error::Format);
        }
        files::sync(&mut app.window).map_err(|_| Error::Format)?;
        files::rename(&mut app.window, temp.as_bytes(), path).map_err(|_| Error::Format)?;
        files::sync(&mut app.window).map_err(|_| Error::Format)?;
        Ok(())
    }
    fn stash(
        secret: &[u8],
        owner: shibari::Owner,
        rng: &mut impl Random,
    ) -> Result<(shibari::Keeper, shibari::Knot), Error> {
        let mut salt = [0u8; 8];
        rng.fill(&mut salt)?;
        let mut keeper = shibari::Keeper::new(u64::from_le_bytes(salt), 1);
        let knot = keeper
            .tie(owner, "Chastity master", secret)
            .map_err(|_| Error::Limit)?;
        Ok((keeper, knot))
    }
    fn new_provider(
        window: &sex_app::Window,
        rng: &mut impl Random,
    ) -> Result<sex_app::autofill::Provider, Error> {
        let mut bytes = [0; 8];
        for _ in 0..8 {
            rng.fill(&mut bytes)?;
            let epoch = u64::from_le_bytes(bytes);
            if epoch != 0 {
                return sex_app::autofill::Provider::new(window, epoch).map_err(|_| Error::Format);
            }
        }
        Err(Error::Entropy)
    }
    fn controls(app: &mut App, view: &View) {
        app.present();
        for (name, id) in [
            ("master", view.master),
            ("confirm", view.confirmation),
            ("create", view.create),
            ("unlock", view.unlock),
            ("generate", view.generate),
            ("add", view.add),
            ("save", view.save),
            ("lock", view.lock),
            ("file", view.file),
            ("title", view.title),
            ("origin", view.origin),
            ("username", view.username),
            ("password", view.password),
        ] {
            let r = app.ui.rect(id).center();
            sex_pdx::serial_record!(
                "[chastity.control] name={} x={} y={}",
                name,
                r.x as i32,
                r.y as i32
            );
        }
    }
    extern "C" fn main() -> ! {
        let mut app = App::open("Chastity", Theme::light(), Strings::new("en"));
        let mut view = View::build(&mut app.ui);
        let mut rng = Entropy;
        let mut master_key: Option<(shibari::Keeper, shibari::Knot)> = None;
        let owner = shibari::Owner(app.window.target);
        let mut provider: Option<sex_app::autofill::Provider> = None;
        let mut consent: Option<(weft::DialogId, u32, usize, chastity_core::Caller, String)> = None;
        let mut shown_token = 0u32;
        let mut routing_ready = false;
        let mut active = sex_pdx::get_ticks();
        app.present();
        sex_pdx::serial_record!(
            "[chastity.ready] target={} task={}",
            app.window.target,
            app.window.task
        );
        controls(&mut app, &view);
        loop {
            let now = sex_pdx::get_ticks();
            if let Some(p) = &mut provider {
                p.poll(now);
                if p.registered() && !routing_ready {
                    routing_ready = true;
                    app.ui.set_text(
                        view.status,
                        "Autofill ready. Each request needs your approval.",
                    );
                    sex_pdx::serial_record!("[chastity.autofill.ready]");
                }
            }
            if let Some(p) = &mut provider {
                if let Some(offer) = p.offer() {
                    if consent.is_none() && shown_token != offer.token {
                        shown_token = offer.token;
                        let selected = app.ui.selected_row(view.list);
                        let found = view.session.vault().ok().and_then(|v| {
                            selected
                                .filter(|i| {
                                    v.entries()
                                        .get(*i)
                                        .is_some_and(|e| e.origin == offer.origin)
                                })
                                .or_else(|| {
                                    v.entries().iter().position(|e| e.origin == offer.origin)
                                })
                        });
                        if let Some(index) = found {
                            let entry = &view.session.vault().unwrap().entries()[index];
                            let body = chastity::consent_text(
                                offer.name,
                                offer.consumer.pd,
                                offer.consumer.task,
                                offer.origin,
                                &entry.title,
                            );
                            if body.is_err() {
                                p.deny();
                                app.ui.set_text(view.status,"Autofill refused: origin or entry label is too long to review safely.");
                            } else {
                                let dialog = app.ui.message(
                                    "Chastity autofill request",
                                    &body.unwrap(),
                                    &["Deny", "Allow once"],
                                    0,
                                    0,
                                );
                                consent = Some((
                                    dialog,
                                    offer.token,
                                    index,
                                    chastity_core::Caller {
                                        domain: offer.consumer.pd,
                                        task: offer.consumer.task as u64,
                                    },
                                    offer.origin.into(),
                                ));
                                sex_pdx::serial_record!(
                                    "[chastity.autofill.prompt] token={} domain={} task={}",
                                    offer.token,
                                    offer.consumer.pd,
                                    offer.consumer.task
                                );
                            }
                        } else {
                            p.deny();
                            app.ui.set_text(
                                view.status,
                                "Autofill refused: no entry matches that exact origin.",
                            );
                            sex_pdx::serial_record!(
                                "[chastity.autofill.refused] reason=no_matching_entry"
                            );
                        }
                    }
                } else if !p.busy() {
                    if let Some((dialog, ..)) = consent.take() {
                        app.ui.close_dialog(dialog);
                    }
                }
            }
            app.present();
            let wait = if provider.as_ref().is_some_and(|p| p.busy()) {
                1
            } else {
                100
            };
            let input = app.window.next_within(wait);
            let now = sex_pdx::get_ticks();
            if now.saturating_sub(active) >= 300000 && view.session.vault().is_ok() {
                if let Some(mut p) = provider.take() {
                    p.close();
                }
                if let Some((id, ..)) = consent.take() {
                    app.ui.close_dialog(id);
                }
                view.lock_now(&mut app.ui);
                master_key = None;
                sex_pdx::serial_record!("[chastity.locked] reason=idle");
            }
            let Some(input) = input else {
                continue;
            };
            active = now;
            if let Event::Key(k) = &input {
                if view.secret_key(&mut app.ui, k.code, k.control, k.char()) {
                    continue;
                }
            }
            for response in app.ui.handle(weft::sexos::event(input)) {
                if response == Response::Close {
                    if let Some(mut p) = provider.take() {
                        p.close();
                    }
                    if let Some((id, ..)) = consent.take() {
                        app.ui.close_dialog(id);
                    }
                    view.lock_now(&mut app.ui);
                    drop(master_key);
                    sex_pdx::task_finish(0);
                }
                let result = (|| -> Result<(), Error> {
                    if let Response::Dialog(id, value) = &response {
                        if consent.as_ref().is_some_and(|c| c.0 == *id) {
                            let (_, token, index, caller, origin) = consent.take().unwrap();
                            let p = provider.as_mut().ok_or(Error::Locked)?;
                            if *value == weft::DialogValue::Button(1)
                                && p.offer().is_some_and(|o| o.token == token)
                            {
                                let grant = view.session.approve(caller, index, &origin, now)?;
                                let granted =
                                    view.session.redeem(caller, grant, now, |user, pass| {
                                        p.allow(user, pass, now).map_err(|_| Error::Denied)
                                    });
                                if !matches!(granted, Ok(Ok(()))) {
                                    p.deny();
                                    return Err(Error::Denied);
                                }
                                sex_pdx::serial_record!(
                                    "[chastity.autofill.approved] token={}",
                                    token
                                );
                            } else {
                                p.deny();
                                sex_pdx::serial_record!(
                                    "[chastity.autofill.denied] token={}",
                                    token
                                );
                            }
                            return Ok(());
                        }
                    }
                    if view.local_action(&mut app.ui, &response, &mut rng)? {
                        if response == Response::Clicked(view.lock) {
                            if let Some(mut p) = provider.take() {
                                p.close();
                            }
                            if let Some((id, ..)) = consent.take() {
                                app.ui.close_dialog(id);
                            }
                            master_key = None;
                            sex_pdx::serial_record!("[chastity.locked] reason=user");
                        }
                        if response == Response::Clicked(view.generate) {
                            sex_pdx::serial_record!("[chastity.generated] length=24");
                        }
                        return Ok(());
                    }
                    let Response::Clicked(id) = response else {
                        return Ok(());
                    };
                    if id == view.create {
                        if view.master_secret.chars().count() < 12
                            || *view.master_secret != *view.confirmation_secret
                        {
                            return Err(Error::Password);
                        }
                        master_key = Some(stash(view.master_secret.as_bytes(), owner, &mut rng)?);
                        view.master_secret.zeroize();
                        provider = Some(new_provider(&app.window, &mut rng)?);
                        routing_ready = false;
                        view.opened(&mut app.ui, Vault::default());
                        view.dirty = true;
                        app.ui.set_text(
                            view.status,
                            "New vault in memory. Add entries, then save a new snapshot.",
                        );
                        sex_pdx::serial_record!("[chastity.created]");
                    } else if id == view.unlock || id == view.import {
                        let path = view.filename(&app.ui)?;
                        let data = read(&mut app, &path)?;
                        let vault = if id == view.import {
                            chastity_core::kdbx::import(&data, view.master_secret.as_bytes())?
                        } else {
                            Vault::unlock(&data, view.master_secret.as_bytes())?
                        };
                        let count = vault.entries().len();
                        master_key = Some(stash(view.master_secret.as_bytes(), owner, &mut rng)?);
                        view.master_secret.zeroize();
                        provider = Some(new_provider(&app.window, &mut rng)?);
                        routing_ready = false;
                        view.opened(&mut app.ui, vault);
                        view.dirty = id == view.import;
                        app.ui.set_text(
                            view.status,
                            if id == view.import {
                                "Imported into memory. Choose a new vault filename and save."
                            } else {
                                "Vault unlocked. Saves create new snapshots."
                            },
                        );
                        sex_pdx::serial_record!(
                            "[chastity.unlocked] entries={} imported={}",
                            count,
                            (id == view.import) as u8
                        );
                    } else if id == view.save {
                        let path = view.filename(&app.ui)?;
                        let (keeper, knot) = master_key.as_ref().ok_or(Error::Locked)?;
                        let vault = view.session.vault()?;
                        let file = keeper
                            .with(owner, *knot, |secret| vault.seal(secret, &mut rng))
                            .map_err(|_| Error::Denied)??;
                        save(&mut app, &path, &file, &mut rng)?;
                        view.dirty = false;
                        app.ui.set_text(view.status,"Encrypted snapshot saved and synced. Choose a new filename for the next save.");
                        sex_pdx::serial_record!("[chastity.saved] bytes={} synced=1", file.len());
                    }
                    Ok(())
                })();
                controls(&mut app, &view);
                if let Err(error) = result {
                    app.ui.set_text(view.status,match error {Error::Authentication=>"Unlock failed: incorrect password or damaged file.",Error::Entropy=>"Cryptographic entropy unavailable. No secret was generated or saved.",Error::Password=>"Use matching master passwords of at least 12 characters.",Error::Unsupported=>"Unsupported KDBX variant or fields. See Chastity import compatibility.",_=>"Operation failed. Check the filename, file permissions and selected entry; saves require a new filename."});
                    sex_pdx::serial_record!("[chastity.refused] kind={:?}", error);
                }
            }
        }
    }
    #[no_mangle]
    pub extern "C" fn _start() -> ! {
        sex_app::with_stack(2 << 20, main)
    }
}
#[cfg(not(feature = "sexos"))]
fn main() {}
