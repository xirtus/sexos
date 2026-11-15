//! eframe's application interface without its native window, for FilmCraft's `App` on
//! SexOS. sex-egui owns the window: it feeds input, calls [`App::raw_input_hook`],
//! [`App::logic`] and [`App::ui`], and paints. There is no wgpu: the `wgpu` and
//! `egui_wgpu` modules hold only the names FilmCraft's GPU path mentions, and
//! [`Frame::wgpu_render_state`] is always `None`, so that path never starts.
//!
//! The trait mirrors eframe 0.37 (storytold's fork, rev 6d518fc), MIT OR Apache-2.0.
#![forbid(unsafe_code)]

pub use egui;

/// Somewhere to keep settings between runs (eframe's `Storage`).
pub trait Storage {
    fn get_string(&self, key: &str) -> Option<String>;
    fn set_string(&mut self, key: &str, value: String);
    fn flush(&mut self);
}

/// What eframe hands an app each pass.
#[derive(Default)]
pub struct Frame {
    ui_language: Option<String>,
}

impl Frame {
    pub fn new() -> Self {
        Self::default()
    }
    /// The language the interface is in (accessibility metadata on desktops).
    pub fn set_ui_language(&mut self, language: Option<&str>) {
        self.ui_language = language.map(str::to_owned);
    }
    pub fn ui_language(&self) -> Option<&str> {
        self.ui_language.as_deref()
    }
    /// No GPU on SexOS.
    pub fn wgpu_render_state(&self) -> Option<&egui_wgpu::RenderState> {
        None
    }
    pub fn storage(&self) -> Option<&dyn Storage> {
        None
    }
}

/// eframe's `App`, as FilmCraft implements it.
pub trait App {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut Frame) {
        _ = (ctx, frame);
    }
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame);
    fn save(&mut self, _storage: &mut dyn Storage) {}
    fn on_exit(&mut self) {}
    fn auto_save_interval(&self) -> core::time::Duration {
        core::time::Duration::from_secs(30)
    }
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::from_rgba_unmultiplied(12, 12, 12, 180).to_normalized_gamma_f32()
    }
    fn persist_egui_memory(&self) -> bool {
        true
    }
    fn raw_input_hook(&mut self, _ctx: &egui::Context, _raw_input: &mut egui::RawInput) {}
}

/// The wgpu names FilmCraft's GPU path uses. Nothing here is ever made.
pub mod wgpu {
    use std::sync::Arc;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum DeviceType {
        Other,
        IntegratedGpu,
        DiscreteGpu,
        VirtualGpu,
        Cpu,
    }
    #[derive(Clone, Debug)]
    pub struct AdapterInfo {
        pub name: String,
        pub backend: Backend,
        pub device_type: DeviceType,
        pub driver: String,
        pub driver_info: String,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Backend {
        Noop,
    }
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Limits {
        pub max_texture_dimension_2d: u32,
    }
    #[derive(Clone, Copy, Debug)]
    pub enum TextureFormat {
        Rgba16Float,
    }
    #[derive(Clone, Copy, Debug)]
    pub enum FilterMode {
        Nearest,
        Linear,
    }
    macro_rules! flags {
        ($name:ident { $($flag:ident = $bit:expr),* }) => {
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub struct $name(u32);
            impl $name {
                $(pub const $flag: Self = Self($bit);)*
                pub fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
            }
            impl std::ops::BitOr for $name {
                type Output = Self;
                fn bitor(self, o: Self) -> Self { Self(self.0 | o.0) }
            }
        };
    }
    flags!(TextureUsages { RENDER_ATTACHMENT = 1, TEXTURE_BINDING = 2 });
    flags!(TextureFormatFeatureFlags { BLENDABLE = 1, FILTERABLE = 2 });
    pub struct TextureFormatFeatures {
        pub allowed_usages: TextureUsages,
        pub flags: TextureFormatFeatureFlags,
    }
    /// A GPU error (never raised: there is no GPU).
    #[derive(Debug)]
    pub struct Error(String);
    impl std::fmt::Display for Error {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }
    pub struct Adapter(());
    impl Adapter {
        pub fn get_info(&self) -> AdapterInfo {
            AdapterInfo { name: String::from("none"), backend: Backend::Noop, device_type: DeviceType::Cpu, driver: String::new(), driver_info: String::new() }
        }
        pub fn get_texture_format_features(&self, _format: TextureFormat) -> TextureFormatFeatures {
            TextureFormatFeatures { allowed_usages: TextureUsages(0), flags: TextureFormatFeatureFlags(0) }
        }
        pub fn limits(&self) -> Limits {
            Limits::default()
        }
    }
    pub struct Device(());
    impl Device {
        pub fn limits(&self) -> Limits {
            Limits::default()
        }
        pub fn on_uncaptured_error(&self, _handler: Arc<dyn Fn(Error) + Send + Sync>) {}
    }
    pub struct Queue(());
    #[derive(Clone)]
    pub struct TextureView(());
}

/// egui-wgpu's render state, by name only.
pub mod egui_wgpu {
    use std::sync::{Arc, RwLock, RwLockWriteGuard};

    pub struct Renderer(());
    impl Renderer {
        pub fn free_texture(&mut self, _id: &egui::TextureId) {}
        pub fn register_native_texture(&mut self, _device: &super::wgpu::Device, _view: &super::wgpu::TextureView, _filter: super::wgpu::FilterMode) -> egui::TextureId {
            egui::TextureId::User(0)
        }
        pub fn update_egui_texture_from_wgpu_texture(&mut self, _device: &super::wgpu::Device, _view: &super::wgpu::TextureView, _filter: super::wgpu::FilterMode, _id: egui::TextureId) {}
    }
    /// A lock whose `write` hands the guard straight back, as egui-wgpu's does.
    pub struct Lock<T>(RwLock<T>);
    impl<T> Lock<T> {
        pub fn write(&self) -> RwLockWriteGuard<'_, T> {
            self.0.write().unwrap_or_else(|e| e.into_inner())
        }
    }
    pub struct RenderState {
        pub adapter: Arc<super::wgpu::Adapter>,
        pub device: Arc<super::wgpu::Device>,
        pub queue: Arc<super::wgpu::Queue>,
        pub renderer: Arc<Lock<Renderer>>,
    }
}
