//! craft-update's interface (storytold/craft-libs, MIT OR Apache-2.0) without the
//! self-updater. A program on SexOS does not replace itself: the store (sex-shop) installs
//! new versions. So no check ever starts, nothing is downloaded, and Help ▸ Check for
//! Updates says where updates come from.
#![forbid(unsafe_code)]

pub mod app {
    /// An app as the updater knows it.
    #[derive(Clone, Copy, Debug)]
    pub struct App {
        pub id: &'static str,
        pub name: &'static str,
        pub tagline: &'static str,
        pub repo: &'static str,
        pub former_ids: &'static [&'static str],
        pub former_names: &'static [&'static str],
        pub icon_png: Option<&'static [u8]>,
        pub accent: Option<[u8; 3]>,
        pub developer: &'static str,
    }
}

pub mod suite {
    use super::app::App;
    pub const SPLICY: App = App {
        id: "os.sex.splicy",
        name: "Splicy",
        tagline: "Video editor",
        repo: "xirtus/sexos",
        former_ids: &[],
        former_names: &[],
        icon_png: None,
        accent: None,
        developer: "SexOS",
    };
}

/// A version, as `parse_version` reads one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

pub fn parse_version(text: &str) -> Version {
    let mut parts = text.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0));
    Version(parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// A release offered to the updater.
#[derive(Clone, Debug, Default)]
pub struct Release {
    pub tag: String,
    pub version: Option<Version>,
    pub notes: String,
    pub url: String,
}

pub mod updater {
    /// What the updater asks of the app.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Action {
        None,
        OpenUrl(String),
        Close,
    }
    /// Where a check stands. Always idle: no check runs here.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Status {
        Idle,
    }
    pub struct Updater {
        status: Status,
    }
    impl Updater {
        pub fn new(_app: &crate::app::App, _version: &str) -> Self {
            Self { status: Status::Idle }
        }
        pub fn check_now(&mut self, _ctx: &egui::Context) {}
        pub fn check_at_start(&mut self, _ctx: &egui::Context) {}
        pub fn tick(&mut self, _ctx: &egui::Context) {}
        pub fn show_if(&mut self, _ctx: &egui::Context, _clean: bool) -> Action {
            Action::None
        }
        pub fn status(&self) -> &Status {
            &self.status
        }
        pub fn dismiss_status(&mut self) {}
        pub fn restart_to_update(&mut self) {}
        pub fn start_pending_update(&mut self) {}
        pub fn offer(&mut self, _release: crate::Release) {}
    }
}

/// The answer to Help ▸ Check for Updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckResultResponse {
    None,
    Close,
    CheckAgain,
    RestartToUpdate,
}

pub struct CheckResultDialog {
    name: &'static str,
    version: String,
}

impl CheckResultDialog {
    pub fn new(app: &app::App, version: &str) -> Self {
        Self { name: app.name, version: version.to_owned() }
    }
    pub fn show(&self, ctx: &egui::Context, _status: &updater::Status) -> CheckResultResponse {
        let mut response = CheckResultResponse::None;
        egui::Window::new("Updates").collapsible(false).resizable(false).pivot(egui::Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
            ui.label(format!("{} {} on SexOS is updated by the store, with the rest of the system.", self.name, self.version));
            ui.add_space(8.0);
            if ui.button("OK").clicked() {
                response = CheckResultResponse::Close;
            }
        });
        response
    }
}
