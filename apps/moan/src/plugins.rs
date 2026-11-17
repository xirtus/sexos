//! Offline effect adapter. The original region remains unchanged until the caller
//! explicitly imports the returned PCM as a new region. No live insert is implied.
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use moan_core::Frame;
#[cfg(all(feature = "sexos", target_arch = "x86_64", target_os = "none"))]
use moan_plugin::job::Parameter;
use moan_plugin::job::{Format, MAX_FRAMES};
pub use moan_plugin::lv2::{
    Error as Lv2Error, Lv2Manifest, Lv2PluginInfo, Lv2Port, Lv2PortKind, Lv2Preset,
};
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Memory,
    Busy,
    PendingResult,
    Path,
    Frames,
    Entropy,
    Launch,
    Failed,
    Audio,
    Exhausted,
}
#[allow(dead_code)]
impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::Memory => "Not enough memory for offline render; free memory and retry",
            Self::Busy => "An offline render is already running",
            Self::PendingResult => "Import the finished render first",
            Self::Path => "Choose a shorter plugin path and render folder without spaces",
            Self::Frames => "Select audio up to 60 seconds",
            Self::Entropy => "Random source unavailable; render was not started",
            Self::Launch => "The offline plugin helper could not start",
            Self::Failed => "Offline render failed; original audio kept",
            Self::Audio => "The plugin returned invalid audio",
            Self::Exhausted => "Restart Moan before another plugin render",
        }
    }
}
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    WaitingForMemory,
    Idle,
    Rendering,
    Finishing,
    Cancelling,
    Ready,
    Failed,
}
#[allow(dead_code)]
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingForMemory => "Completed offline render kept; free memory and retry",
            Self::Idle => "Offline render ready",
            Self::Rendering => "Offline render running",
            Self::Finishing => "Finishing offline render",
            Self::Cancelling => "Cancelling offline render; waiting for helper retirement",
            Self::Ready => "Offline render ready to import",
            Self::Failed => "Offline render failed; original region unchanged",
        }
    }
}
pub struct Audio {
    pub frames: Vec<Frame>,
    pub clipped_samples: u32,
    pub format: Format,
}
pub struct Paths {
    pub plugin: Vec<u8>,
    pub input: Vec<u8>,
    pub output: Vec<u8>,
}
fn valid_token(p: &[u8]) -> bool {
    p.starts_with(b"@/")
        && p.len() > 2
        && p.len() <= 192
        && !p.iter().any(|b| *b <= 32 || *b == 127)
        && !p[2..]
            .split(|b| *b == b'/')
            .any(|s| s.is_empty() || s == b"." || s == b"..")
}
impl Paths {
    pub fn new(
        plugin: &[u8],
        folder: &[u8],
        nonce: [u8; 16],
        format: Format,
    ) -> Result<Self, Error> {
        let folder = folder.strip_suffix(b"/").unwrap_or(folder);
        if !valid_token(plugin) || !valid_token(folder) || nonce == [0; 16] {
            return Err(Error::Path);
        }
        //64-bit filename suffix is only a fresh-name convenience; all128 nonce bits
        //remain in the request/result. Existing paths are refused, never overwritten.
        let suffix: String = nonce[..8].iter().map(|b| format!("{b:02x}")).collect();
        let mut base = folder.to_vec();
        base.extend_from_slice(b"/moan-");
        base.extend_from_slice(suffix.as_bytes());
        let mut input = base.clone();
        input.extend_from_slice(b".in");
        let mut output = base;
        output.extend_from_slice(b".out");
        if !valid_token(&input)
            || !valid_token(&output)
            || plugin == input
            || plugin == output
            || 32 + 3 + plugin.len() + input.len() + output.len() + 1 + format.token().len() > 256
        {
            return Err(Error::Path);
        }
        Ok(Self {
            plugin: plugin.to_vec(),
            input,
            output,
        })
    }
}
pub fn float_input(pcm: &[Frame]) -> Result<Vec<[f32; 2]>, Error> {
    if pcm.is_empty() || pcm.len() > MAX_FRAMES as usize {
        return Err(Error::Frames);
    }
    let mut result = Vec::new();
    result
        .try_reserve_exact(pcm.len())
        .map_err(|_| Error::Memory)?;
    result.extend(
        pcm.iter()
            .map(|f| [f[0] as f32 / 32768.0, f[1] as f32 / 32768.0]),
    );
    Ok(result)
}
pub fn quantize(pcm: &[[f32; 2]], format: Format) -> Result<Audio, Error> {
    if pcm.is_empty() || pcm.len() > MAX_FRAMES as usize {
        return Err(Error::Frames);
    }
    let mut frames = Vec::new();
    frames
        .try_reserve_exact(pcm.len())
        .map_err(|_| Error::Memory)?;
    let mut clipped_samples = 0;
    for f in pcm {
        let mut out = [0; 2];
        for i in 0..2 {
            let v = f[i];
            if !v.is_finite() || v.abs() > 16.0 {
                return Err(Error::Audio);
            }
            if !(-1.0..=32767.0 / 32768.0).contains(&v) {
                clipped_samples += 1;
            }
            out[i] = (v * 32768.0).clamp(-32768.0, 32767.0) as i16;
        }
        frames.push(out);
    }
    Ok(Audio {
        frames,
        clipped_samples,
        format,
    })
}
#[cfg(all(feature = "sexos", target_arch = "x86_64", target_os = "none"))]
pub mod guest {
    use super::*;
    use core::sync::atomic::{AtomicU32, Ordering};
    use moan_plugin::{job, supervisor::OfflineJob};
    static NEXT_COOKIE: AtomicU32 = AtomicU32::new(1);
    pub struct Render {
        active: Option<OfflineJob>,
        status: Status,
        result: Option<Audio>,
        format: Format,
    }
    impl Default for Render {
        fn default() -> Self {
            Self {
                active: None,
                status: Status::Idle,
                result: None,
                format: Format::Clap,
            }
        }
    }
    impl Render {
        pub fn status(&self) -> Status {
            self.status
        }
        pub fn busy(&self) -> bool {
            self.active.is_some()
        }
        pub fn retired(&self) -> bool {
            self.active.as_ref().is_none_or(|job| job.retired())
        }
        pub fn start(
            &mut self,
            plugin: &[u8],
            folder: &[u8],
            input: &[Frame],
            format: Format,
            parameters: &[Parameter],
            now: u64,
        ) -> Result<(), Error> {
            if self.busy() {
                return Err(Error::Busy);
            }
            if self.result.is_some() {
                return Err(Error::PendingResult);
            }
            let nonce = moan_plugin::nonce::fresh().ok_or(Error::Entropy)?;
            let paths = Paths::new(plugin, folder, nonce, format)?;
            let pcm = float_input(input)?;
            let sequence = NEXT_COOKIE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                    if value <= 65535 {
                        Some(value + 1)
                    } else {
                        None
                    }
                })
                .map_err(|_| Error::Exhausted)?;
            let cookie = 0x504c_0000 | sequence;
            let job = OfflineJob::start_with(
                &paths.plugin,
                &paths.input,
                &paths.output,
                nonce,
                cookie,
                &pcm,
                parameters,
                format,
                now,
            )
            .map_err(|error| match error {
                moan_plugin::supervisor::Error::Memory
                | moan_plugin::supervisor::Error::Job(job::Error::Memory) => Error::Memory,
                _ => Error::Launch,
            })?;
            self.active = Some(job);
            self.format = format;
            self.status = Status::Rendering;
            Ok(())
        }
        /// Continue polling during cancellation and defer final app exit until retired.
        pub fn poll(&mut self, now: u64) -> bool {
            if self.status == Status::WaitingForMemory {
                return false;
            }
            let before = self.status;
            let Some(job) = self.active.as_mut() else {
                return false;
            };
            job.poll(now);
            if job.result_consumed() {
                if job.retired() {
                    self.active = None;
                    self.status = if self.result.is_some() {
                        Status::Ready
                    } else if before == Status::Failed {
                        Status::Failed
                    } else {
                        Status::Idle
                    };
                }
                return before != self.status;
            }
            if matches!(
                job.status(),
                job::Status::Cancelling(_) | job::Status::CancelWaiting
            ) {
                self.status = Status::Cancelling;
            }
            if job.retired() {
                let result = match job.read_result() {
                    Ok(Some(pcm)) => quantize(&pcm, self.format),
                    Err(moan_plugin::supervisor::Error::Memory)
                    | Err(moan_plugin::supervisor::Error::Job(job::Error::Memory)) => {
                        Err(Error::Memory)
                    }
                    _ => Err(Error::Failed),
                };
                match result {
                    Ok(audio) => {
                        job.accept_result();
                        self.result = Some(audio);
                        self.status = Status::Ready;
                    }
                    Err(Error::Memory) => self.status = Status::WaitingForMemory,
                    Err(_) => {
                        job.cancel();
                        self.status = Status::Failed;
                    }
                }
            }
            before != self.status
        }
        pub fn retry_result(&mut self) {
            if self.status == Status::WaitingForMemory {
                self.status = Status::Rendering;
            }
        }
        pub fn cancel(&mut self) {
            if let Some(job) = self.active.as_mut() {
                job.cancel();
                self.status = Status::Cancelling;
            }
        }
        pub fn take_result(&mut self) -> Option<Audio> {
            let result = self.result.take();
            if result.is_some() {
                self.status = if self.active.is_some() {
                    Status::Finishing
                } else {
                    Status::Idle
                };
            }
            result
        }
    }
}

/// UI-friendly descriptor for one LV2 control input port.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Lv2ControlUi {
    pub index: u32,
    pub symbol: String,
    pub name: String,
    pub default_value: f32,
    pub min_value: Option<f32>,
    pub max_value: Option<f32>,
    pub is_integer: bool,
    pub is_toggled: bool,
    pub is_logarithmic: bool,
}

impl From<&Lv2Port> for Lv2ControlUi {
    fn from(port: &Lv2Port) -> Self {
        Self {
            index: port.index,
            symbol: port.symbol.clone(),
            name: port.name.clone(),
            default_value: port.default_value,
            min_value: port.min_value,
            max_value: port.max_value,
            is_integer: port.is_integer,
            is_toggled: port.is_toggled,
            is_logarithmic: port.is_logarithmic,
        }
    }
}

/// Scanned LV2 bundle entry backed by pure-Rust Turtle (`.ttl`) manifest parsing.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub struct Lv2DiscoveredBundle {
    pub bundle_path: String,
    pub manifest: Lv2Manifest,
}

/// Safe pure-Rust LV2 bundle manifest discovery catalog for Moan's Instruments/Plugins pane.
/// Parsing `.ttl` manifests never loads or executes any native plugin shared object.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lv2Catalog {
    pub bundles: Vec<Lv2DiscoveredBundle>,
}

#[allow(dead_code)]
impl Lv2Catalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `manifest.ttl` and optional `rdfs:seeAlso` `.ttl` documents for `bundle_path`,
    /// replacing any previous scan for the same bundle path. Returns discovered plugin count.
    pub fn discover_ttl_bundle(
        &mut self,
        bundle_path: &str,
        manifest_ttl: &str,
        extra_ttls: &[&str],
    ) -> Result<usize, Lv2Error> {
        let manifest = Lv2Manifest::parse_bundle(manifest_ttl, extra_ttls)?;
        let count = manifest.plugins.len();
        if let Some(existing) = self
            .bundles
            .iter_mut()
            .find(|b| b.bundle_path == bundle_path)
        {
            existing.manifest = manifest;
        } else {
            self.bundles.push(Lv2DiscoveredBundle {
                bundle_path: String::from(bundle_path),
                manifest,
            });
        }
        Ok(count)
    }

    /// Parses a single combined `.ttl` document for `bundle_path`.
    pub fn discover_single_ttl(
        &mut self,
        bundle_path: &str,
        ttl: &str,
    ) -> Result<usize, Lv2Error> {
        self.discover_ttl_bundle(bundle_path, ttl, &[])
    }

    pub fn plugins(&self) -> Vec<(&str, &Lv2PluginInfo)> {
        let mut out = Vec::new();
        for bundle in &self.bundles {
            for plugin in &bundle.manifest.plugins {
                out.push((bundle.bundle_path.as_str(), plugin));
            }
        }
        out
    }

    pub fn stereo_plugins(&self) -> Vec<(&str, &Lv2PluginInfo)> {
        self.plugins()
            .into_iter()
            .filter(|(_, info)| info.is_stereo_capable())
            .collect()
    }

    pub fn find_plugin(&self, uri: &str) -> Option<(&str, &Lv2PluginInfo)> {
        self.plugins()
            .into_iter()
            .find(|(_, info)| info.uri == uri)
    }

    pub fn plugin_count(&self) -> usize {
        self.bundles.iter().map(|b| b.manifest.plugins.len()).sum()
    }

    pub fn control_ui(info: &Lv2PluginInfo) -> Vec<Lv2ControlUi> {
        info.control_inputs().map(Lv2ControlUi::from).collect()
    }

    pub fn summary_text(&self) -> String {
        let total = self.plugin_count();
        if total == 0 {
            return String::from("LV2: no bundles scanned");
        }
        let mut names: Vec<&str> = Vec::new();
        for (_, info) in self.plugins().into_iter().take(3) {
            names.push(info.name.as_str());
        }
        let suffix = if total > 3 { ", ..." } else { "" };
        format!(
            "LV2: {} bundle(s) · {} plugin(s) ({}{})",
            self.bundles.len(),
            total,
            names.join(", "),
            suffix
        )
    }
}
