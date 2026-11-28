// SPDX-License-Identifier: GPL-3.0-only
//! Camera and microphone for pages (`navigator.mediaDevices.getUserMedia`).
//!
//! A page only asks (sex-js `op::CAPTURE`); Kaleidoscope answers in its own card under
//! the omnibox, which pages cannot draw over, and keeps the person's answer for the
//! site in Files (`Kaleidoscope/site-permissions.tsv`; private windows keep nothing).
//! The browser runs the devices itself: the webcam through SexUVC's live frame bank
//! (`sex_pdx::usb_uvc::FrameBank`, the same one Flash reads), and the microphone
//! through Bell, which still asks the person before any program hears it. A page's
//! `<video>` with the stream as `srcObject` shows the camera's latest picture as the
//! page picture `video:N`. Devices stop as soon as no live track of the page uses
//! them, and when the page goes away.
//!
//! `--fake-media` (for gates, like Chrome's `--use-fake-device-for-media-stream`)
//! gives a moving test pattern camera and a 440 Hz microphone, and lets plain http
//! pages ask.
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use sex_app::{files, Window};
use sex_js::capture::Device;
use sex_js::{Capture as Shared, CaptureAsk, Decision};
use sex_pdx::usb_uvc;
use sex_web::{Image, PermissionPrompt};

const SITES: &[u8] = b"Kaleidoscope/site-permissions.tsv";
const SITES_MAX: u64 = 64 << 10;
/// Biggest picture shown: what a video element can fill.
const MAX_W: usize = crate::media::MAX_W;
const MAX_H: usize = crate::media::MAX_H;
const FAKE_W: u16 = 640;
const FAKE_H: u16 = 480;
const FAKE_FPS: u32 = 30;

/// The camera while a page uses it.
struct Camera {
    bank: Option<usb_uvc::FrameBank>,
    fake: Option<sex_uvc::SyntheticUvcDevice>,
    last_seq: u32,
    raw: Vec<u8>,
    decoded: Vec<u32>,
    fake_frame: u64,
    next_fake: u64,
    frames: u32,
    errors: u32,
}

/// The microphone while a page uses it.
struct Mic {
    listener: Option<sex_app::sound::Listener>,
    /// What Bell said last (`sound_ring::MIC_*`).
    state: u64,
    heard: u64,
    started: u64,
    logged: u64,
    phase: u32,
}

pub struct Capture {
    pub shared: Rc<RefCell<Shared>>,
    /// The question on screen, and the ones behind it.
    pub asking: Option<(u64, CaptureAsk)>,
    queue: Vec<(u64, CaptureAsk)>,
    /// Dispatches for the page that asked: (page owner, text).
    pub answers: Vec<(u64, String)>,
    /// A note card (no question), such as Bell still waiting to share the microphone.
    pub note: Option<PermissionPrompt>,
    fake: bool,
    keep: bool,
    saved: String,
    /// The page whose tracks hold the devices.
    owner: u64,
    camera: Option<Camera>,
    mic: Option<Mic>,
    /// The camera's latest picture.
    pub picture: Option<Rc<Image>>,
    /// Bumped with each new picture.
    pub seq: u32,
    /// What the shown page uses, for the omnibox's red dot.
    pub using: (bool, bool),
    /// Bell's microphone row for this program (for the note).
    row: usize,
}

impl Capture {
    /// Before the window's files are reachable (replaced by `new`).
    pub fn empty() -> Capture {
        Capture { shared: Rc::new(RefCell::new(Shared::default())), asking: None, queue: Vec::new(), answers: Vec::new(), note: None, fake: false, keep: false,
            saved: String::new(), owner: 0, camera: None, mic: None, picture: None, seq: 0, using: (false, false), row: 0 }
    }

    pub fn new(window: &mut Window, fake: bool, private: bool) -> Capture {
        let mut shared = Shared { any_origin: fake, ..Default::default() };
        let mut saved = String::new();
        if !private {
            if let Some(text) = read(window, SITES) { shared.load(&text); saved = shared.save(); }
        }
        let row = sex_pdx::sound_ring::index_of(window.target).unwrap_or(0);
        let mut c = Capture { shared: Rc::new(RefCell::new(shared)), asking: None, queue: Vec::new(), answers: Vec::new(), note: None, fake, keep: !private,
            saved, owner: 0, camera: None, mic: None, picture: None, seq: 0, using: (false, false), row };
        c.refresh_devices();
        sex_pdx::serial_record!("[web.capture.ready] fake={} sites={} camera={}", fake as u8, c.shared.borrow().sites.len(), c.camera_present() as u8);
        c
    }

    /// The camera there is now, if any: (id, label).
    fn camera_device(&self) -> Option<(String, String)> {
        if self.fake { return Some((String::from("fake-camera"), String::from("SexOS test camera"))); }
        let info = camera_bank().and_then(|b| b.info())?;
        Some((alloc::format!("usb-{:04x}-{:04x}", info.vendor, info.product), alloc::format!("USB camera ({:04x}:{:04x})", info.vendor, info.product)))
    }
    fn camera_present(&self) -> bool { self.camera_device().is_some() }

    /// What `enumerateDevices` can list.
    pub fn refresh_devices(&mut self) {
        let mut devices = Vec::new();
        let mic = if self.fake { "SexOS test microphone" } else { "Default microphone" };
        devices.push(Device { kind: "audioinput", id: String::from(if self.fake { "fake-microphone" } else { "default" }), label: String::from(mic) });
        if let Some((id, label)) = self.camera_device() { devices.push(Device { kind: "videoinput", id, label }); }
        devices.push(Device { kind: "audiooutput", id: String::from("default"), label: String::from("Default speakers") });
        self.shared.borrow_mut().devices = devices;
    }

    /// A page's `getUserMedia` calls. `framed`: it came from inside an iframe, which
    /// may not ask (no `allow=` delegation yet).
    pub fn take(&mut self, owner: u64, asks: Vec<CaptureAsk>, framed: bool) -> bool {
        let mut shown = false;
        for ask in asks {
            sex_pdx::serial_record!("[web.capture.ask] id={} origin={} audio={} video={} framed={}", ask.id, ask.origin, ask.audio as u8, ask.video as u8, framed as u8);
            let (secure, mic, cam) = {
                let s = self.shared.borrow();
                (s.secure(&ask.origin), s.decision(&ask.origin, false), s.decision(&ask.origin, true))
            };
            let wanted = [(ask.audio, mic), (ask.video, cam)];
            if framed || !secure || wanted.iter().any(|(w, d)| *w && *d == Decision::Block) {
                self.answer(owner, &ask, "denied");
            } else if wanted.iter().all(|(w, d)| !*w || *d == Decision::Allow) {
                self.grant(owner, &ask);
            } else if self.asking.is_none() {
                self.asking = Some((owner, ask));
                shown = true;
            } else if self.queue.len() < 8 {
                self.queue.push((owner, ask));
            } else {
                self.answer(owner, &ask, "denied");
            }
        }
        if shown { self.log_prompt(); }
        shown
    }

    fn log_prompt(&self) {
        if let Some((_, a)) = &self.asking {
            sex_pdx::serial_record!("[web.capture.prompt] origin={} audio={} video={}", a.origin, a.audio as u8, a.video as u8);
        }
    }

    /// The card for the question on screen.
    pub fn prompt(&self) -> Option<PermissionPrompt> {
        if let Some((_, a)) = &self.asking {
            let what = match (a.audio, a.video) { (true, true) => "your camera and microphone", (false, true) => "your camera", _ => "your microphone" };
            let site = a.origin.split_once("://").map_or(a.origin.as_str(), |(_, h)| h);
            return Some(PermissionPrompt {
                title: alloc::format!("{site} wants to use {what}"),
                detail: String::from("Kaleidoscope remembers your answer for this site."),
                buttons: alloc::vec![String::from("Allow"), String::from("Block")],
            });
        }
        self.note.clone()
    }

    /// The person answered the card: button 0 (Allow), 1 (Block), or None (Escape:
    /// not now, nothing kept). A note card just closes.
    pub fn answered(&mut self, button: Option<usize>) {
        let Some((owner, ask)) = self.asking.take() else { self.note = None; return };
        sex_pdx::serial_record!("[web.capture.answer] origin={} button={}", ask.origin, button.map_or(-1, |b| b as i64));
        match button {
            Some(0) | Some(1) => {
                let d = if button == Some(0) { Decision::Allow } else { Decision::Block };
                {
                    let mut s = self.shared.borrow_mut();
                    if ask.audio { s.decide(&ask.origin, false, d); }
                    if ask.video { s.decide(&ask.origin, true, d); }
                }
                if d == Decision::Allow { self.grant(owner, &ask); } else { self.answer(owner, &ask, "denied"); }
                // The same site's other questions now have their answer.
                let queued = core::mem::take(&mut self.queue);
                let mut rest = Vec::new();
                for (o, a) in queued {
                    if a.origin == ask.origin { let _ = self.take(o, alloc::vec![a], false); } else { rest.push((o, a)); }
                }
                if self.asking.is_none() && !rest.is_empty() { self.asking = Some(rest.remove(0)); self.log_prompt(); }
                self.queue.extend(rest);
            }
            _ => {
                self.answer(owner, &ask, "denied");
                if !self.queue.is_empty() { self.asking = Some(self.queue.remove(0)); self.log_prompt(); }
            }
        }
    }

    fn answer(&mut self, owner: u64, ask: &CaptureAsk, result: &str) {
        sex_pdx::serial_record!("[web.capture.result] id={} result={}", ask.id, result);
        self.answers.push((owner, alloc::format!("capture\t{}\t{}", ask.id, result)));
    }

    /// Starts what `ask` wants and answers the page.
    fn grant(&mut self, owner: u64, ask: &CaptureAsk) {
        if self.owner != owner { self.stop(); self.owner = owner; }
        let (mut a_label, mut a_id, mut v_label, mut v_id, mut size) = (String::new(), String::new(), String::new(), String::new(), (0u16, 0u16, 0u32));
        if ask.video {
            let Some((id, label)) = self.camera_device() else { return self.answer(owner, ask, "none") };
            if !ask.video_device.is_empty() && ask.video_device != id { return self.answer(owner, ask, "none"); }
            size = if self.fake { (FAKE_W, FAKE_H, FAKE_FPS) } else {
                let info = camera_bank().and_then(|b| b.info());
                info.map_or((1280, 720, 30), |i| (i.width, i.height, i.fps.max(1) as u32))
            };
            self.start_camera();
            (v_id, v_label) = (id, label);
        }
        if ask.audio {
            let devs = self.shared.borrow().devices.clone();
            let Some(d) = devs.iter().find(|d| d.kind == "audioinput") else { return self.answer(owner, ask, "none") };
            if !ask.audio_device.is_empty() && ask.audio_device != d.id && ask.audio_device != "default" { return self.answer(owner, ask, "none"); }
            self.start_mic();
            (a_id, a_label) = (d.id.clone(), d.label.clone());
        }
        self.using = (self.camera.is_some(), self.mic.is_some());
        sex_pdx::serial_record!("[web.capture.result] id={} result=ok audio={} video={} size={}x{}", ask.id, ask.audio as u8, ask.video as u8, size.0, size.1);
        self.answers.push((owner, alloc::format!("capture\t{}\tok\t{a_label}\t{a_id}\t{v_label}\t{v_id}\t{}\t{}\t{}", ask.id, size.0, size.1, size.2)));
        // Bell asks the person itself before this program hears anything.
        if ask.audio && !self.fake {
            if let Some(m) = self.mic.as_ref() { if m.state != sex_pdx::sound_ring::MIC_ON { self.answers.push((owner, String::from("capturemute\taudio\t1"))); } }
        }
    }

    fn start_camera(&mut self) {
        if self.camera.is_some() { return; }
        let bank = if self.fake { None } else { camera_bank() };
        if bank.is_some() { let _ = sex_pdx::pdx_call_checked(usb_uvc::CLIENT_NOTIFY_SLOT, usb_uvc::OP_CLIENT_START, 0, 0, 0); }
        let fake = self.fake.then(|| sex_uvc::SyntheticUvcDevice::new(FAKE_W, FAKE_H, FAKE_FPS));
        sex_pdx::serial_record!("[web.capture.camera] on=1 source={}", if fake.is_some() { "fake" } else { "usb" });
        self.camera = Some(Camera { bank, fake, last_seq: 0, raw: Vec::new(), decoded: Vec::new(), fake_frame: 0, next_fake: 0, frames: 0, errors: 0 });
    }

    fn stop_camera(&mut self) {
        let Some(c) = self.camera.take() else { return };
        if c.bank.is_some() { let _ = sex_pdx::pdx_call_checked(usb_uvc::CLIENT_NOTIFY_SLOT, usb_uvc::OP_CLIENT_STOP, 0, 0, 0); }
        sex_pdx::serial_record!("[web.capture.camera] on=0 frames={} errors={}", c.frames, c.errors);
        self.picture = None;
    }

    fn start_mic(&mut self) {
        if self.mic.is_some() { return; }
        let listener = if self.fake { None } else { sex_app::sound::Listener::open() };
        let now = sex_pdx::get_ticks();
        let state = if self.fake { sex_pdx::sound_ring::MIC_ON } else { listener.as_ref().map_or(sex_pdx::sound_ring::MIC_NONE, |l| l.state()) };
        sex_pdx::serial_record!("[web.capture.mic] on=1 source={} ring={}", if self.fake { "fake" } else { "bell" }, listener.is_some() as u8);
        self.mic = Some(Mic { listener, state, heard: 0, started: now, logged: now, phase: 0 });
    }

    fn stop_mic(&mut self) {
        let Some(m) = self.mic.take() else { return };
        sex_pdx::serial_record!("[web.capture.mic] on=0 heard={}", m.heard);
        if self.note.is_some() { self.note = None; }
    }

    /// The page's live tracks now use these kinds: let the others go.
    pub fn uses(&mut self, owner: u64, audio: bool, video: bool) {
        if owner != self.owner { return; }
        if !video { self.stop_camera(); }
        if !audio { self.stop_mic(); }
        self.using = (self.camera.is_some(), self.mic.is_some());
    }

    /// The page holding the devices went away (or `owner`'s did): stop, and drop its
    /// questions.
    pub fn page_left(&mut self, owner: u64) {
        if self.asking.as_ref().is_some_and(|(o, _)| *o == owner) { self.asking = None; }
        self.queue.retain(|(o, _)| *o != owner);
        self.answers.retain(|(o, _)| *o != owner);
        if self.asking.is_none() && !self.queue.is_empty() { self.asking = Some(self.queue.remove(0)); }
        if owner == self.owner { self.stop(); }
    }

    pub fn stop(&mut self) {
        self.stop_camera();
        self.stop_mic();
        self.using = (false, false);
    }

    pub fn owner(&self) -> u64 { self.owner }

    /// Takes new camera pictures and microphone sound. Returns whether a new picture
    /// came, whether the card changed, and how soon to look again.
    pub fn tick(&mut self, now: u64) -> (bool, bool, Option<u64>) {
        let mut fresh = false;
        let mut card = false;
        let mut next: Option<u64> = None;
        if let Some(c) = self.camera.as_mut() {
            if let Some(dev) = c.fake.as_ref() {
                if now >= c.next_fake {
                    c.next_fake = now + 1000 / FAKE_FPS as u64;
                    let raw = dev.generate_raw_yuy2_frame(c.fake_frame);
                    c.fake_frame += 1;
                    let (w, h) = (FAKE_W as usize, FAKE_H as usize);
                    c.decoded.resize(w * h, 0);
                    if sex_uvc::pixel::yuy2_to_argb(&raw, w, h, &mut c.decoded).is_ok() {
                        // The bars repeat every 16 frames, which can alias with whatever rate
                        // the machine reaches: scroll the whole picture sideways too, so any two
                        // frames differ.
                        let off = (c.fake_frame as usize * 7) % w;
                        for row in c.decoded.chunks_exact_mut(w) { row.rotate_left(off); }
                        // A counter box, so a test sees the picture move.
                        let x0 = (c.fake_frame as usize * 4) % (w - 32);
                        for y in 8..40 { for x in x0..x0 + 32 { c.decoded[y * w + x] = 0xFFFF_FFFF; } }
                        self.picture = Some(Rc::new(Image { width: w, height: h, pixels: c.decoded.clone() }));
                        c.frames += 1;
                        fresh = true;
                    }
                }
                next = Some(c.next_fake.saturating_sub(now).max(1));
            } else if let Some(bank) = c.bank {
                if let Some(img) = read_bank(bank, c) { self.picture = Some(Rc::new(img)); c.frames += 1; fresh = true; }
                next = Some(10);
            }
            if fresh && (c.frames <= 4 || c.frames.is_power_of_two()) {
                let (w, h) = self.picture.as_ref().map_or((0, 0), |p| (p.width, p.height));
                sex_pdx::serial_record!("[web.capture.frame] n={} w={} h={}", c.frames, w, h);
            }
        }
        if fresh { self.seq = self.seq.wrapping_add(1); }
        let owner = self.owner;
        let row = self.row;
        if let Some(m) = self.mic.as_mut() {
            let mut buf = [[0i16; 2]; 1024];
            if let Some(l) = m.listener.as_mut() {
                l.keep();
                loop { let n = l.hear(&mut buf); m.heard += n as u64; if n < buf.len() { break; } }
                let state = l.state();
                if state != m.state {
                    sex_pdx::serial_record!("[web.capture.mic.state] state={}", state);
                    if state == sex_pdx::sound_ring::MIC_ON { self.answers.push((owner, String::from("capturemute\taudio\t0"))); if self.note.take().is_some() { card = true; } }
                    if state == sex_pdx::sound_ring::MIC_NONE { self.answers.push((owner, String::from("capturegone\taudio"))); }
                    if state == sex_pdx::sound_ring::MIC_WAITING && self.note.is_none() {
                        self.note = Some(PermissionPrompt {
                            title: String::from("Let Kaleidoscope hear the microphone"),
                            detail: alloc::format!("SexOS asks first. In Spindle: volume {row} mic on"),
                            buttons: alloc::vec![String::from("OK")],
                        });
                        card = true;
                    }
                    m.state = state;
                }
            } else {
                // The test microphone: a 440 Hz tone, as much as time has passed.
                let due = (now.saturating_sub(m.started)) * 48;
                while m.heard < due {
                    let n = ((due - m.heard) as usize).min(buf.len());
                    for f in buf[..n].iter_mut() { let v = tone(m.phase); m.phase = m.phase.wrapping_add(1); *f = [v, v]; }
                    m.heard += n as u64;
                }
            }
            if now >= m.logged + 1000 { m.logged = now; sex_pdx::serial_record!("[web.capture.mic.heard] frames={}", m.heard); }
            next = Some(next.map_or(20, |n| n.min(20)));
        }
        (fresh, card, next)
    }

    /// Keeps the person's choices in Files when they changed.
    pub fn save(&mut self, window: &mut Window) {
        if !self.keep { return; }
        let text = self.shared.borrow().save();
        if text == self.saved { return; }
        if files::write(window, SITES, text.as_bytes()).is_ok() { self.saved = text; }
    }
}

fn camera_bank() -> Option<usb_uvc::FrameBank> {
    #[cfg(target_os = "none")]
    { usb_uvc::FrameBank::open_reader().filter(|b| b.is_active()) }
    #[cfg(not(target_os = "none"))]
    { None }
}

/// The newest camera picture in the frame bank, decoded, if there is one.
fn read_bank(bank: usb_uvc::FrameBank, c: &mut Camera) -> Option<Image> {
    if !bank.is_active() { return None; }
    if c.raw.len() < usb_uvc::FRAME_SLOT_BYTES { c.raw.resize(usb_uvc::FRAME_SLOT_BYTES, 0); }
    let meta = bank.read_frame(c.last_seq, &mut c.raw)?;
    c.last_seq = meta.seq;
    let raw = &c.raw[..meta.len.min(c.raw.len())];
    let (mut w, mut h) = (meta.width as usize, meta.height as usize);
    let ok = match meta.format {
        usb_uvc::FMT_MJPEG => {
            if let Ok((_, dw, dh)) = sex_image::dimensions(raw) { (w, h) = (dw, dh); }
            if w == 0 || h == 0 || w > 4096 || h > 4096 { false } else {
                c.decoded.resize(w * h, 0);
                match sex_uvc::pixel::mjpeg_to_argb(raw, &mut c.decoded) { Ok((dw, dh)) => { (w, h) = (dw, dh); true } Err(_) => false }
            }
        }
        usb_uvc::FMT_YUY2 if w > 0 && h > 0 && w <= 4096 && h <= 4096 => { c.decoded.resize(w * h, 0); sex_uvc::pixel::yuy2_to_argb(raw, w, h, &mut c.decoded).is_ok() }
        usb_uvc::FMT_NV12 if w > 0 && h > 0 && w <= 4096 && h <= 4096 => { c.decoded.resize(w * h, 0); sex_uvc::pixel::nv12_to_argb(raw, w, h, &mut c.decoded).is_ok() }
        _ => false,
    };
    if !ok { c.errors += 1; return None; }
    if w > MAX_W || h > MAX_H {
        let (fw, fh) = sex_media::fit(w, h, MAX_W, MAX_H);
        let mut out = alloc::vec![0u32; fw * fh];
        sex_uvc::pixel::scale_argb_fit(&c.decoded[..w * h], w, h, &mut out, fw, fh);
        return Some(Image { width: fw, height: fh, pixels: out });
    }
    Some(Image { width: w, height: h, pixels: c.decoded[..w * h].to_vec() })
}

/// One sample of a quiet 440 Hz tone at 48 kHz, without floating point: a triangle.
fn tone(i: u32) -> i16 {
    let period = 48_000 / 440;
    let p = (i % period) as i32;
    let half = period as i32 / 2;
    let tri = if p < half { p * 2 - half } else { (period as i32 - p) * 2 - half };
    (tri * 8000 / half) as i16
}

fn read(window: &mut Window, path: &[u8]) -> Option<String> {
    let size = files::size(window, path).ok()?;
    if size == 0 || size > SITES_MAX { return None; }
    let mut buf = alloc::vec![0u8; size as usize];
    let n = files::read(window, path, &mut buf).ok()?;
    buf.truncate(n);
    String::from_utf8(buf).ok()
}
