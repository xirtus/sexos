// SPDX-License-Identifier: GPL-3.0-only
//! Camera and microphone for pages (Media Capture and Streams): what the browser
//! shares with every page's [`crate::Document`] about the person's choices and the
//! devices there are. Pages only ever ask; the browser answers in its own bar, keeps
//! the person's answer per site here, and starts the devices itself. A page learns a
//! device's name only once its site may use that kind of device, as in other browsers.
use alloc::string::String;
use alloc::vec::Vec;

/// What the person said a site may do with one kind of device.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Decision {
    /// Nothing yet: the browser asks when the page wants it.
    #[default]
    Ask,
    Allow,
    Block,
}

impl Decision {
    /// The `PermissionStatus.state` a page reads.
    pub fn state(self) -> &'static str {
        match self { Decision::Ask => "prompt", Decision::Allow => "granted", Decision::Block => "denied" }
    }
    pub fn word(self) -> &'static str {
        match self { Decision::Ask => "ask", Decision::Allow => "allow", Decision::Block => "block" }
    }
    pub fn from_word(w: &str) -> Decision {
        match w { "allow" => Decision::Allow, "block" => Decision::Block, _ => Decision::Ask }
    }
}

/// One site's choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    /// `scheme://host[:port]`, as [`sex_fetch::Url::origin`] gives it.
    pub origin: String,
    pub microphone: Decision,
    pub camera: Decision,
}

/// A device pages can name: `audioinput`, `videoinput` or `audiooutput`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub kind: &'static str,
    /// Stable for the device, so a page can ask for the same one again.
    pub id: String,
    pub label: String,
}

#[derive(Default)]
pub struct Capture {
    /// Every page may ask, even plain http ones (the browser was started with
    /// `--fake-media` for tests, as Chrome's `--use-fake-ui-for-media-stream`).
    pub any_origin: bool,
    pub sites: Vec<Site>,
    pub devices: Vec<Device>,
}

impl Capture {
    pub fn site(&self, origin: &str) -> Option<&Site> { self.sites.iter().find(|s| s.origin == origin) }

    pub fn decision(&self, origin: &str, camera: bool) -> Decision {
        self.site(origin).map_or(Decision::Ask, |s| if camera { s.camera } else { s.microphone })
    }

    /// Sets what `origin` may do with one kind of device.
    pub fn decide(&mut self, origin: &str, camera: bool, d: Decision) {
        let i = match self.sites.iter().position(|s| s.origin == origin) {
            Some(i) => i,
            None => {
                // A bounded list: the oldest choice goes first.
                if self.sites.len() >= 256 { self.sites.remove(0); }
                self.sites.push(Site { origin: String::from(origin), microphone: Decision::Ask, camera: Decision::Ask });
                self.sites.len() - 1
            }
        };
        if camera { self.sites[i].camera = d } else { self.sites[i].microphone = d }
        if self.sites[i].camera == Decision::Ask && self.sites[i].microphone == Decision::Ask { self.sites.remove(i); }
    }

    /// Forgets everything said about `origin`.
    pub fn forget(&mut self, origin: &str) { self.sites.retain(|s| s.origin != origin); }

    /// Whether pages of `origin` may ask at all: secure pages (https, and http on this
    /// machine), as the specification requires.
    pub fn secure(&self, origin: &str) -> bool {
        if self.any_origin { return true; }
        if origin.starts_with("https://") { return true; }
        let Some(rest) = origin.strip_prefix("http://") else { return false };
        let host = rest.rsplit_once(':').map_or(rest, |(h, _)| h);
        host == "localhost" || host.ends_with(".localhost") || host == "127.0.0.1" || host == "[::1]"
    }

    /// What `enumerateDevices` answers `origin`: one `kind\tid\tlabel\tgroup` line per
    /// device. Before the site may use a kind of device its names are empty, and only
    /// one device of each kind shows, as in other browsers.
    pub fn listing(&self, origin: &str) -> String {
        let mut out = String::new();
        let mut shown: Vec<&str> = Vec::new();
        for d in &self.devices {
            let may = match d.kind {
                "videoinput" => self.decision(origin, true) == Decision::Allow,
                // Sound outputs are named once the microphone may be used.
                _ => self.decision(origin, false) == Decision::Allow,
            };
            if !may && shown.contains(&d.kind) { continue; }
            shown.push(d.kind);
            let (id, label) = if may { (d.id.as_str(), d.label.as_str()) } else { ("", "") };
            out.push_str(&alloc::format!("{}\t{}\t{}\t{}\n", d.kind, id, clean(label), if may { "sexos" } else { "" }));
        }
        out
    }

    /// The choices as kept in Files: `origin\tmicrophone\tcamera` lines.
    pub fn save(&self) -> String {
        let mut out = String::new();
        for s in &self.sites { out.push_str(&alloc::format!("{}\t{}\t{}\n", s.origin, s.microphone.word(), s.camera.word())); }
        out
    }

    pub fn load(&mut self, text: &str) {
        self.sites.clear();
        for line in text.lines() {
            let mut f = line.split('\t');
            let (Some(origin), Some(mic), Some(cam)) = (f.next(), f.next(), f.next()) else { continue };
            if origin.is_empty() || origin.contains(' ') { continue; }
            let (m, c) = (Decision::from_word(mic.trim()), Decision::from_word(cam.trim()));
            if m == Decision::Ask && c == Decision::Ask { continue; }
            if self.sites.len() >= 256 { break; }
            self.sites.push(Site { origin: String::from(origin), microphone: m, camera: c });
        }
    }
}

fn clean(s: &str) -> String { s.chars().map(|c| if c == '\t' || c == '\n' || c == '\r' { ' ' } else { c }).collect() }

/// A page's `getUserMedia` call, for the browser to answer with
/// `capture\tID\tRESULT\t...` (see `op::CAPTURE`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureAsk {
    pub id: u32,
    pub origin: String,
    pub audio: bool,
    pub video: bool,
    /// The device ids the page asked for exactly (empty: any).
    pub audio_device: String,
    pub video_device: String,
}
