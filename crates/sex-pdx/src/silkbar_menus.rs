// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Interactive SilkBar dropdown menus, right-side status popovers, and
//! desktop/window right-click context menus.
//!
//! Shell owns menu state, hit-testing, hover-switching, and action dispatch;
//! Display renders the frosted candy-lavender glass panel using the exact same
//! `Layout`, so every pointer click lands on the drawn row or control.
//!
//! Wire (`OP_SILKBAR_MENU`, Shell -> Display):
//! - `arg0 = w0`, `arg1 = w1` from `MenuState::wire()`.
//! - `arg0 == 0` hides the menu.
//! Self-contained (`#![no_std]`, zero heap) so host tests can compile it directly.

/// Shell -> Display PDX opcode for SilkBar menus, popovers, and context menus.
pub const OP_SILKBAR_MENU: u64 = 0x2FC;

/// Maximum rows in any single menu or popover.
pub const MAX_ROWS: usize = 16;

/// Power profile identifiers packed into `MenuState::flags`.
pub const PROFILE_BALANCED: u8 = 0;
pub const PROFILE_PERFORMANCE: u8 = 1;
pub const PROFILE_QUIET: u8 = 2;

/// Menu flags packed into `MenuState::flags`.
pub const FLAG_MAXIMIZED: u8 = 1 << 0;
pub const FLAG_FLOATING: u8 = 1 << 1;
pub const FLAG_MUTED: u8 = 1 << 2;
pub const FLAG_ON_AC: u8 = 1 << 3;
pub const FLAG_CHARGING: u8 = 1 << 4;
pub const FLAG_GROUPED: u8 = 1 << 5;
pub const FLAG_PROFILE_SHIFT: u8 = 6;
pub const FLAG_PROFILE_MASK: u8 = 0b11 << FLAG_PROFILE_SHIFT;

/// Which dropdown, status popover, or context menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MenuKind {
    /// Left SexOS / System pill dropdown.
    System = 1,
    /// Top-bar `File` contextual dropdown.
    AppFile = 2,
    /// Top-bar `Edit` contextual dropdown.
    AppEdit = 3,
    /// Top-bar `View` contextual dropdown.
    AppView = 4,
    /// Top-bar `Window` contextual dropdown (includes tabbing controls).
    AppWindow = 5,
    /// Right-side Wi-Fi candy pill status popover.
    WifiPopover = 6,
    /// Right-side Volume candy pill status popover.
    VolumePopover = 7,
    /// Right-side Battery/Power candy pill status popover.
    BatteryPopover = 8,
    /// Right-side Clock candy pill status popover.
    ClockPopover = 9,
    /// Right-click context menu on desktop background.
    DesktopContext = 10,
    /// Right-click context menu on window titlebar.
    WindowContext = 11,
}

impl MenuKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::System),
            2 => Some(Self::AppFile),
            3 => Some(Self::AppEdit),
            4 => Some(Self::AppView),
            5 => Some(Self::AppWindow),
            6 => Some(Self::WifiPopover),
            7 => Some(Self::VolumePopover),
            8 => Some(Self::BatteryPopover),
            9 => Some(Self::ClockPopover),
            10 => Some(Self::DesktopContext),
            11 => Some(Self::WindowContext),
            _ => None,
        }
    }

    /// True for top-bar dropdowns/popovers (eligible for macOS-style hover switching).
    pub const fn is_top_bar(self) -> bool {
        matches!(
            self,
            Self::System
                | Self::AppFile
                | Self::AppEdit
                | Self::AppView
                | Self::AppWindow
                | Self::WifiPopover
                | Self::VolumePopover
                | Self::BatteryPopover
                | Self::ClockPopover
        )
    }

    /// True for right-anchored status popovers.
    pub const fn is_right_popover(self) -> bool {
        matches!(
            self,
            Self::WifiPopover | Self::VolumePopover | Self::BatteryPopover | Self::ClockPopover
        )
    }

    /// Base logical width of the panel at 1x scale.
    pub const fn base_width(self) -> usize {
        match self {
            Self::System => 316,
            Self::AppFile => 224,
            Self::AppEdit => 196,
            Self::AppView => 228,
            Self::AppWindow => 256,
            Self::WifiPopover | Self::BatteryPopover | Self::ClockPopover => 256,
            Self::VolumePopover => 264,
            Self::DesktopContext => 216,
            Self::WindowContext => 240,
        }
    }
}

/// Top-bar interactive trigger slots for click & hover-switching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopBarTrigger {
    System,
    File,
    Edit,
    View,
    Window,
    Wifi,
    Volume,
    Battery,
    Clock,
}

impl TopBarTrigger {
    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "SexOS",
            Self::File => "File",
            Self::Edit => "Edit",
            Self::View => "View",
            Self::Window => "Window",
            Self::Wifi => "Wi-Fi",
            Self::Volume => "Volume",
            Self::Battery => "Battery",
            Self::Clock => "Clock",
        }
    }

    pub const fn menu_kind(self) -> MenuKind {
        match self {
            Self::System => MenuKind::System,
            Self::File => MenuKind::AppFile,
            Self::Edit => MenuKind::AppEdit,
            Self::View => MenuKind::AppView,
            Self::Window => MenuKind::AppWindow,
            Self::Wifi => MenuKind::WifiPopover,
            Self::Volume => MenuKind::VolumePopover,
            Self::Battery => MenuKind::BatteryPopover,
            Self::Clock => MenuKind::ClockPopover,
        }
    }
}

/// Concrete action dispatched when a menu row or popover control is clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    None,
    AboutSexOs,
    LaunchBrowser,
    LaunchTerminal,
    LaunchTailor,
    LaunchFlash,
    LaunchPdGraph,
    LaunchFiles,
    LaunchSettings,
    RefreshDesktop,
    Reboot,
    Shutdown,
    NewWindow,
    CloseWindow,
    MinimizeWindow,
    ToggleMaximizeWindow,
    ToggleFloatWindow,
    TileLeft,
    TileRight,
    MoveToNextSpace,
    MoveToPrevSpace,
    MoveToSpace(u8),
    BringToFront,
    GroupWindow,
    UngroupTab,
    NextTab,
    PrevTab,
    EditCopy,
    EditPaste,
    EditSelectAll,
    DisplaySettings,
    WifiRefresh,
    NetworkSettings,
    VolumeMuteToggle,
    VolumeDown,
    VolumeUp,
    VolumeSet(u8),
    PowerProfileQuiet,
    PowerProfileBalanced,
    PowerProfilePerformance,
    OpenCalendar,
    OpenSpotlight,
    LaunchAppStore,
    OpenAppsSwitcher,
    ForceQuitApps,
    LockScreen,
    QuitApp,
}

impl MenuAction {
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::AboutSexOs => "about_sexos",
            Self::LaunchBrowser => "launch_browser",
            Self::LaunchTerminal => "launch_terminal",
            Self::LaunchTailor => "launch_tailor",
            Self::LaunchFlash => "launch_flash",
            Self::LaunchPdGraph => "launch_pd_graph",
            Self::LaunchFiles => "launch_files",
            Self::LaunchSettings => "launch_settings",
            Self::RefreshDesktop => "refresh_desktop",
            Self::Reboot => "reboot",
            Self::Shutdown => "shutdown",
            Self::NewWindow => "new_window",
            Self::CloseWindow => "close_window",
            Self::MinimizeWindow => "minimize_window",
            Self::ToggleMaximizeWindow => "toggle_maximize",
            Self::ToggleFloatWindow => "toggle_float",
            Self::TileLeft => "tile_left",
            Self::TileRight => "tile_right",
            Self::MoveToNextSpace => "move_next_space",
            Self::MoveToPrevSpace => "move_prev_space",
            Self::MoveToSpace(_) => "move_to_space",
            Self::BringToFront => "bring_to_front",
            Self::GroupWindow => "group_window",
            Self::UngroupTab => "ungroup_tab",
            Self::NextTab => "next_tab",
            Self::PrevTab => "prev_tab",
            Self::EditCopy => "edit_copy",
            Self::EditPaste => "edit_paste",
            Self::EditSelectAll => "edit_select_all",
            Self::DisplaySettings => "display_settings",
            Self::WifiRefresh => "wifi_refresh",
            Self::NetworkSettings => "network_settings",
            Self::VolumeMuteToggle => "volume_mute",
            Self::VolumeDown => "volume_down",
            Self::VolumeUp => "volume_up",
            Self::VolumeSet(_) => "volume_set",
            Self::PowerProfileQuiet => "profile_quiet",
            Self::PowerProfileBalanced => "profile_balanced",
            Self::PowerProfilePerformance => "profile_performance",
            Self::OpenCalendar => "open_calendar",
            Self::OpenSpotlight => "open_spotlight",
            Self::LaunchAppStore => "launch_app_store",
            Self::OpenAppsSwitcher => "open_apps_switcher",
            Self::ForceQuitApps => "force_quit_apps",
            Self::LockScreen => "lock_screen",
            Self::QuitApp => "quit_app",
        }
    }

    /// True if clicking this control should keep the popover open (e.g. volume/profile).
    pub const fn keeps_open(self) -> bool {
        matches!(
            self,
            Self::VolumeMuteToggle
                | Self::VolumeDown
                | Self::VolumeUp
                | Self::VolumeSet(_)
                | Self::PowerProfileQuiet
                | Self::PowerProfileBalanced
                | Self::PowerProfilePerformance
                | Self::WifiRefresh
        )
    }
}

/// One row descriptor inside a menu or popover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowSpec {
    pub label: &'static str,
    pub shortcut: &'static str,
    pub action: MenuAction,
    pub sep_before: bool,
    pub danger: bool,
    pub header: bool,
    pub active: bool,
}

impl RowSpec {
    const fn item(label: &'static str, action: MenuAction) -> Self {
        Self { label, shortcut: "", action, sep_before: false, danger: false, header: false, active: false }
    }
    const fn key(mut self, shortcut: &'static str) -> Self { self.shortcut = shortcut; self }
    const fn sep(mut self) -> Self { self.sep_before = true; self }
    const fn danger(mut self) -> Self { self.danger = true; self }
    const fn header(label: &'static str) -> Self {
        Self { label, shortcut: "", action: MenuAction::None, sep_before: false, danger: false, header: true, active: false }
    }
    const fn active(mut self, yes: bool) -> Self { self.active = yes; self }
}

/// Complete state of an open SilkBar dropdown, status popover, or context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub kind: MenuKind,
    /// Anchor X/Y (for top-bar menus, `y` is bar height + gap; for context menus, pointer pos).
    pub x: u16,
    pub y: u16,
    /// Hovered row index, if any.
    pub hover: Option<u8>,
    /// Bitflags (`FLAG_*` + power profile in bits 6..7).
    pub flags: u8,
    /// Master volume 0..=100.
    pub volume: u8,
    /// Battery charge 0..=100, or `0xFF` when unavailable.
    pub battery: u8,
    /// `sex_pdx::wifi_status` phase, or `0xFF` when unknown.
    pub wifi_phase: u8,
    /// System uptime in minutes (for Clock popover).
    pub uptime_mins: u32,
    /// Active Desktop Space index (0..=4) for Space-move indicators.
    pub active_space: u8,
}

impl MenuState {
    pub const fn new(kind: MenuKind, x: i32, y: i32) -> Self {
        let cx = if x < 0 { 0 } else if x > u16::MAX as i32 { u16::MAX } else { x as u16 };
        let cy = if y < 0 { 0 } else if y > u16::MAX as i32 { u16::MAX } else { y as u16 };
        Self {
            kind,
            x: cx,
            y: cy,
            hover: None,
            flags: 0,
            volume: 75,
            battery: 0xFF,
            wifi_phase: 0xFF,
            uptime_mins: 0,
            active_space: 0,
        }
    }

    pub const fn maximized(&self) -> bool { self.flags & FLAG_MAXIMIZED != 0 }
    pub const fn floating(&self) -> bool { self.flags & FLAG_FLOATING != 0 }
    pub const fn muted(&self) -> bool { self.flags & FLAG_MUTED != 0 }
    pub const fn on_ac(&self) -> bool { self.flags & FLAG_ON_AC != 0 }
    pub const fn charging(&self) -> bool { self.flags & FLAG_CHARGING != 0 }
    pub const fn grouped(&self) -> bool { self.flags & FLAG_GROUPED != 0 }
    pub const fn power_profile(&self) -> u8 { (self.flags & FLAG_PROFILE_MASK) >> FLAG_PROFILE_SHIFT }

    pub fn with_power_profile(mut self, profile: u8) -> Self {
        self.flags = (self.flags & !FLAG_PROFILE_MASK) | ((profile & 0b11) << FLAG_PROFILE_SHIFT);
        self
    }

    /// Number of rows in this menu/popover.
    pub const fn row_count(&self) -> usize {
        match self.kind {
            MenuKind::System => 9,
            MenuKind::AppFile => 4,
            MenuKind::AppEdit => 3,
            MenuKind::AppView => 4,
            MenuKind::AppWindow => 16,
            MenuKind::WifiPopover => 4,
            MenuKind::VolumePopover => 3,
            MenuKind::BatteryPopover => 5,
            MenuKind::ClockPopover => 4,
            MenuKind::DesktopContext => 6,
            MenuKind::WindowContext => 13,
        }
    }

    /// Row descriptor at `index`.
    pub fn row(&self, index: usize) -> Option<RowSpec> {
        let max_lbl = if self.maximized() { "Restore Window" } else { "Maximize Window" };
        let float_lbl = if self.floating() { "Tile Window" } else { "Float Window" };
        let prof = self.power_profile();
        Some(match (self.kind, index) {
            // Left SexOS Lips / System menu (Apple  + KDE polish)
            (MenuKind::System, 0) => RowSpec::item("About This SexOS", MenuAction::AboutSexOs),
            (MenuKind::System, 1) => RowSpec::item("System Settings...", MenuAction::LaunchSettings).key("Super+,").sep(),
            (MenuKind::System, 2) => RowSpec::item("App Store (SexShop)...", MenuAction::LaunchAppStore),
            (MenuKind::System, 3) => RowSpec::item("Spotlight Search...", MenuAction::OpenSpotlight).key("Super+Spc").sep(),
            (MenuKind::System, 4) => RowSpec::item("Active & Background Apps...", MenuAction::OpenAppsSwitcher),
            (MenuKind::System, 5) => RowSpec::item("Force Quit Applications...", MenuAction::ForceQuitApps).key("Super+Alt+Esc").sep(),
            (MenuKind::System, 6) => RowSpec::item("Lock Screen", MenuAction::LockScreen).key("Super+L").sep(),
            (MenuKind::System, 7) => RowSpec::item("Restart...", MenuAction::Reboot).danger(),
            (MenuKind::System, 8) => RowSpec::item("Shut Down...", MenuAction::Shutdown).danger(),

            // App contextual menus (with macOS Finder / KDE-style right-aligned shortcut hints)
            (MenuKind::AppFile, 0) => RowSpec::item("New Window", MenuAction::NewWindow).key("Super+Ret"),
            (MenuKind::AppFile, 1) => RowSpec::item("Open Files", MenuAction::LaunchFiles),
            (MenuKind::AppFile, 2) => RowSpec::item("Close Window", MenuAction::CloseWindow).key("Super+W").sep(),
            (MenuKind::AppFile, 3) => RowSpec::item("Quit App", MenuAction::QuitApp).key("Super+Q").danger(),

            (MenuKind::AppEdit, 0) => RowSpec::item("Copy", MenuAction::EditCopy).key("Ctrl+C"),
            (MenuKind::AppEdit, 1) => RowSpec::item("Paste", MenuAction::EditPaste).key("Ctrl+V"),
            (MenuKind::AppEdit, 2) => RowSpec::item("Select All", MenuAction::EditSelectAll).key("Ctrl+A"),

            (MenuKind::AppView, 0) => RowSpec::item(max_lbl, MenuAction::ToggleMaximizeWindow).key("Super+F"),
            (MenuKind::AppView, 1) => RowSpec::item(float_lbl, MenuAction::ToggleFloatWindow).key("Super+V"),
            (MenuKind::AppView, 2) => RowSpec::item("Refresh Desktop", MenuAction::RefreshDesktop).sep(),
            (MenuKind::AppView, 3) => RowSpec::item("Display Settings", MenuAction::DisplaySettings),

            (MenuKind::AppWindow, 0) => RowSpec::item("Minimize", MenuAction::MinimizeWindow),
            (MenuKind::AppWindow, 1) => RowSpec::item(max_lbl, MenuAction::ToggleMaximizeWindow).key("Super+F"),
            (MenuKind::AppWindow, 2) => RowSpec::item(float_lbl, MenuAction::ToggleFloatWindow).key("Super+V"),
            (MenuKind::AppWindow, 3) => RowSpec::item("Tile Left", MenuAction::TileLeft).key("Super+H").sep(),
            (MenuKind::AppWindow, 4) => RowSpec::item("Tile Right", MenuAction::TileRight).key("Super+L"),
            (MenuKind::AppWindow, 5) => RowSpec::item("Move to Next Space", MenuAction::MoveToNextSpace).sep(),
            (MenuKind::AppWindow, 6) => RowSpec::item("Move to Prev Space", MenuAction::MoveToPrevSpace),
            (MenuKind::AppWindow, 7) => RowSpec::item("Move to Space 1", MenuAction::MoveToSpace(0)).active(self.active_space == 0),
            (MenuKind::AppWindow, 8) => RowSpec::item("Move to Space 2", MenuAction::MoveToSpace(1)).active(self.active_space == 1),
            (MenuKind::AppWindow, 9) => RowSpec::item("Move to Space 3", MenuAction::MoveToSpace(2)).active(self.active_space == 2),
            (MenuKind::AppWindow, 10) => RowSpec::item("Move to Space 4", MenuAction::MoveToSpace(3)).active(self.active_space == 3),
            (MenuKind::AppWindow, 11) => RowSpec::item("Move to Space 5", MenuAction::MoveToSpace(4)).active(self.active_space == 4),
            (MenuKind::AppWindow, 12) => RowSpec::item("Group Tab", MenuAction::GroupWindow).key("Super+G").sep(),
            (MenuKind::AppWindow, 13) => RowSpec::item(
                if self.grouped() { "Ungroup Tab" } else { "Next Tab" },
                if self.grouped() { MenuAction::UngroupTab } else { MenuAction::NextTab },
            ).key(if self.grouped() { "Shift+G" } else { "Super+." }),
            (MenuKind::AppWindow, 14) => RowSpec::item("Bring to Front", MenuAction::BringToFront),
            (MenuKind::AppWindow, 15) => RowSpec::item("Close Window", MenuAction::CloseWindow).key("Super+W").sep().danger(),

            // Right-side Wi-Fi popover
            (MenuKind::WifiPopover, 0) => RowSpec::header(wifi_status_label(self.wifi_phase)),
            (MenuKind::WifiPopover, 1) => RowSpec::header(wifi_ssid_ip_label(self.wifi_phase)),
            (MenuKind::WifiPopover, 2) => RowSpec::item("Refresh Network", MenuAction::WifiRefresh).sep(),
            (MenuKind::WifiPopover, 3) => RowSpec::item("Network Settings", MenuAction::NetworkSettings),

            // Right-side Volume popover (row 0 = header, row 1 = slider bar, row 2 = - / Mute / +)
            (MenuKind::VolumePopover, 0) => RowSpec::header(if self.muted() { "Audio: Muted" } else { "Audio Output" }),
            (MenuKind::VolumePopover, 1) => RowSpec::item("Volume", MenuAction::None),
            (MenuKind::VolumePopover, 2) => RowSpec::item("Mute", MenuAction::VolumeMuteToggle).sep(),

            // Right-side Battery/Power popover
            (MenuKind::BatteryPopover, 0) => RowSpec::header("Power & Battery"),
            (MenuKind::BatteryPopover, 1) => RowSpec::header(if self.charging() {
                "AC Connected (Charging)"
            } else if self.on_ac() {
                "AC Power Connected"
            } else {
                "Running on Battery"
            }),
            (MenuKind::BatteryPopover, 2) => RowSpec::item("Profile: Quiet", MenuAction::PowerProfileQuiet)
                .sep()
                .active(prof == PROFILE_QUIET),
            (MenuKind::BatteryPopover, 3) => RowSpec::item("Profile: Balanced", MenuAction::PowerProfileBalanced)
                .active(prof == PROFILE_BALANCED),
            (MenuKind::BatteryPopover, 4) => RowSpec::item("Profile: Performance", MenuAction::PowerProfilePerformance)
                .active(prof == PROFILE_PERFORMANCE),

            // Right-side Clock popover
            (MenuKind::ClockPopover, 0) => RowSpec::header("Date & Time"),
            (MenuKind::ClockPopover, 1) => RowSpec::header("Timezone: UTC"),
            (MenuKind::ClockPopover, 2) => RowSpec::header("Uptime"),
            (MenuKind::ClockPopover, 3) => RowSpec::item("Open Month Calendar", MenuAction::OpenCalendar).sep(),

            // Desktop background right-click context menu
            (MenuKind::DesktopContext, 0) => RowSpec::item("New Terminal", MenuAction::LaunchTerminal).key("Super+Ret"),
            (MenuKind::DesktopContext, 1) => RowSpec::item("Open Browser", MenuAction::LaunchBrowser),
            (MenuKind::DesktopContext, 2) => RowSpec::item("Open Tailor AI", MenuAction::LaunchTailor),
            (MenuKind::DesktopContext, 3) => RowSpec::item("Open Files", MenuAction::LaunchFiles),
            (MenuKind::DesktopContext, 4) => RowSpec::item("Refresh Desktop", MenuAction::RefreshDesktop).sep(),
            (MenuKind::DesktopContext, 5) => RowSpec::item("Display Settings", MenuAction::DisplaySettings),

            // Window titlebar right-click / Ctrl+Click context menu
            (MenuKind::WindowContext, 0) => RowSpec::item("Tile Left", MenuAction::TileLeft),
            (MenuKind::WindowContext, 1) => RowSpec::item("Tile Right", MenuAction::TileRight),
            (MenuKind::WindowContext, 2) => RowSpec::item(max_lbl, MenuAction::ToggleMaximizeWindow).key("Super+F"),
            (MenuKind::WindowContext, 3) => RowSpec::item(float_lbl, MenuAction::ToggleFloatWindow).key("Super+V"),
            (MenuKind::WindowContext, 4) => RowSpec::item("Move to Next Space", MenuAction::MoveToNextSpace).sep(),
            (MenuKind::WindowContext, 5) => RowSpec::item("Move to Prev Space", MenuAction::MoveToPrevSpace),
            (MenuKind::WindowContext, 6) => RowSpec::item("Move to Space 1", MenuAction::MoveToSpace(0)).active(self.active_space == 0),
            (MenuKind::WindowContext, 7) => RowSpec::item("Move to Space 2", MenuAction::MoveToSpace(1)).active(self.active_space == 1),
            (MenuKind::WindowContext, 8) => RowSpec::item("Move to Space 3", MenuAction::MoveToSpace(2)).active(self.active_space == 2),
            (MenuKind::WindowContext, 9) => RowSpec::item("Move to Space 4", MenuAction::MoveToSpace(3)).active(self.active_space == 3),
            (MenuKind::WindowContext, 10) => RowSpec::item("Move to Space 5", MenuAction::MoveToSpace(4)).active(self.active_space == 4),
            (MenuKind::WindowContext, 11) => RowSpec::item("Minimize", MenuAction::MinimizeWindow).sep(),
            (MenuKind::WindowContext, 12) => RowSpec::item("Close Window", MenuAction::CloseWindow).key("Super+W").sep().danger(),

            _ => return None,
        })
    }

    /// Encode into two `u64` words for `OP_SILKBAR_MENU`.
    pub const fn wire(&self) -> (u64, u64) {
        let hover = match self.hover {
            Some(h) if h < 31 => h as u64,
            _ => 31u64,
        };
        let w0 = (self.x as u64)
            | ((self.y as u64) << 16)
            | ((self.kind as u64) << 32)
            | (hover << 36)
            | ((self.flags as u64) << 41)
            | (1u64 << 63);
        let vol = if self.volume > 100 { 100 } else { self.volume };
        let active = if self.active_space <= 7 { self.active_space } else { 0 };
        let w1 = (vol as u64)
            | ((self.battery as u64) << 8)
            | ((self.wifi_phase as u64) << 16)
            | (((self.uptime_mins & 0x00FF_FFFF) as u64) << 24)
            | ((active as u64) << 48);
        (w0, w1)
    }

    /// Decode from `OP_SILKBAR_MENU` `(arg0, arg1)`. Returns `None` for hidden (`w0 == 0`) or invalid words.
    pub fn decode(w0: u64, w1: u64) -> Option<Self> {
        if w0 == 0 || (w0 >> 63) == 0 {
            return None;
        }
        // Reserved bits 49..63 must be zero.
        if ((w0 >> 49) & 0x3FFF) != 0 || (w1 >> 52) != 0 {
            return None;
        }
        let active_space = ((w1 >> 48) & 0xF) as u8;
        if active_space > 7 {
            return None;
        }
        let kind = MenuKind::from_u8(((w0 >> 32) & 0xF) as u8)?;
        let raw_hover = ((w0 >> 36) & 0x1F) as u8;
        let state = Self {
            kind,
            x: (w0 & 0xFFFF) as u16,
            y: ((w0 >> 16) & 0xFFFF) as u16,
            hover: None,
            flags: ((w0 >> 41) & 0xFF) as u8,
            volume: ((w1 & 0xFF) as u8).min(100),
            battery: ((w1 >> 8) & 0xFF) as u8,
            wifi_phase: ((w1 >> 16) & 0xFF) as u8,
            uptime_mins: ((w1 >> 24) & 0x00FF_FFFF) as u32,
            active_space,
        };
        if raw_hover != 31 && raw_hover as usize >= state.row_count() {
            return None;
        }
        Some(Self { hover: (raw_hover != 31).then_some(raw_hover), ..state })
    }
}

pub const fn wifi_status_label(phase: u8) -> &'static str {
    match phase {
        0 => "Wired LAN Connected",
        1 => "Wi-Fi: Starting...",
        2 => "Wi-Fi: Scanning...",
        3 => "Wi-Fi: Disconnected",
        4 => "Wi-Fi: Connection Failed",
        5..=9 => "Wi-Fi: Connecting...",
        10 => "Wi-Fi: Connected",
        _ => "Network: Ready",
    }
}

pub const fn wifi_ssid_ip_label(phase: u8) -> &'static str {
    match phase {
        0 => "eth0  10.0.2.15",
        10 => "SSID: SexOS-5G  10.0.2.15",
        3 | 4 => "No IP address assigned",
        _ => "DHCP  10.0.2.15",
    }
}

pub type Rect = (usize, usize, usize, usize);

#[inline]
pub const fn rect_contains(r: Rect, x: usize, y: usize) -> bool {
    x >= r.0 && y >= r.1 && x - r.0 < r.2 && y - r.1 < r.3
}

/// Computes the 4 top-bar contextual menu pill rects (`File`, `Edit`, `View`, `Window`)
/// immediately to the right of the active window badge, while keeping strictly left of
/// `290 * scale` (so pixel `(300, 14)` in `virtio_gpu_gate.py` remains untouched bar glass).
pub fn top_bar_menu_rects(
    bar_h: usize,
    scale: u32,
    app_pill_right: usize,
    ws0_x: usize,
) -> [(TopBarTrigger, Rect); 4] {
    let s = scale.clamp(1, 3) as usize;
    let max_right = ws0_x.saturating_sub(8 * s).min(290 * s);
    let y = 3 * s;
    let h = bar_h.saturating_sub(6 * s);
    let specs: [(TopBarTrigger, usize); 4] = [
        (TopBarTrigger::File, 32 * s),
        (TopBarTrigger::Edit, 32 * s),
        (TopBarTrigger::View, 34 * s),
        (TopBarTrigger::Window, 48 * s),
    ];
    let gap = 2 * s;
    // Start just after the active app pill, clamped so all 4 fit before max_right if possible.
    let total_w: usize = 32 * s + 32 * s + 34 * s + 48 * s + 3 * gap;
    let min_start = 114 * s;
    let mut x = app_pill_right.saturating_add(4 * s).max(min_start);
    if x + total_w > max_right {
        // Still keep menus strictly < max_right; drop trailing items if narrow.
        x = app_pill_right.saturating_add(4 * s);
    }
    let mut out = [(TopBarTrigger::File, (0, 0, 0, 0)); 4];
    for (i, &(trig, w)) in specs.iter().enumerate() {
        if x + w <= max_right && h > 0 {
            out[i] = (trig, (x, y, w, h));
            x += w + gap;
        } else {
            out[i] = (trig, (0, 0, 0, 0));
        }
    }
    out
}

/// Hit result inside a `Layout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    VolumeBar(u8),
    VolumeDown,
    VolumeUp,
    VolumeMute,
    Inside,
    Outside,
}

/// Screen-clamped layout shared by Shell hit-testing and Display rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub kind: MenuKind,
    pub panel: Rect,
    pub rows: usize,
    pub row_h: usize,
    pub pad: usize,
    pub scale: u32,
    /// Interactive controls for `MenuKind::VolumePopover`.
    pub vol_bar: Rect,
    pub vol_down: Rect,
    pub vol_mute: Rect,
    pub vol_up: Rect,
}

impl Layout {
    pub const ROW_H: usize = 26;
    pub const PAD: usize = 6;

    pub fn new(state: &MenuState, screen_w: usize, screen_h: usize, scale: u32) -> Option<Self> {
        let rows = state.row_count();
        (1..=scale.clamp(1, 3)).rev().find_map(|sc| {
            let s = sc as usize;
            let pw = state.kind.base_width() * s;
            let row_h = Self::ROW_H * s;
            let pad = Self::PAD * s;
            let ph = rows * row_h + 2 * pad;
            if screen_w < pw + 8 * s || screen_h < ph + 8 * s {
                return None;
            }
            let margin = 6 * s;
            let raw_x = if state.kind.is_right_popover() {
                (state.x as usize).saturating_sub(pw / 2)
            } else {
                state.x as usize
            };
            let x = raw_x.clamp(margin, screen_w.saturating_sub(pw + margin));
            let y = (state.y as usize).min(screen_h.saturating_sub(ph + margin));
            let panel = (x, y, pw, ph);

            // Volume popover interactive controls on rows 1 & 2.
            let r1 = (x + pad, y + pad + row_h, pw - 2 * pad, row_h);
            let r2 = (x + pad, y + pad + 2 * row_h, pw - 2 * pad, row_h);
            let vol_bar = (r1.0 + 4 * s, r1.1 + 6 * s, r1.2.saturating_sub(8 * s), r1.3.saturating_sub(12 * s));
            let btn_gap = 6 * s;
            let btn_w = r2.2.saturating_sub(2 * btn_gap) / 3;
            let btn_h = r2.3.saturating_sub(4 * s);
            let btn_y = r2.1 + 2 * s;
            let vol_down = (r2.0, btn_y, btn_w, btn_h);
            let vol_mute = (r2.0 + btn_w + btn_gap, btn_y, btn_w, btn_h);
            let vol_up = (r2.0 + 2 * (btn_w + btn_gap), btn_y, r2.2.saturating_sub(2 * (btn_w + btn_gap)), btn_h);

            Some(Self {
                kind: state.kind,
                panel,
                rows,
                row_h,
                pad,
                scale: sc,
                vol_bar,
                vol_down,
                vol_mute,
                vol_up,
            })
        })
    }

    pub fn row_rect(&self, index: usize) -> Rect {
        let (x, y, w, _) = self.panel;
        (x + self.pad, y + self.pad + index * self.row_h, w - 2 * self.pad, self.row_h)
    }

    pub fn hit(&self, x: usize, y: usize) -> Hit {
        if !rect_contains(self.panel, x, y) {
            return Hit::Outside;
        }
        if self.kind == MenuKind::VolumePopover {
            if rect_contains(self.vol_down, x, y) {
                return Hit::VolumeDown;
            }
            if rect_contains(self.vol_mute, x, y) {
                return Hit::VolumeMute;
            }
            if rect_contains(self.vol_up, x, y) {
                return Hit::VolumeUp;
            }
            let r1 = self.row_rect(1);
            if rect_contains(r1, x, y) && self.vol_bar.2 > 0 {
                let pct = x.saturating_sub(self.vol_bar.0).min(self.vol_bar.2) * 100 / self.vol_bar.2;
                return Hit::VolumeBar(pct as u8);
            }
            return Hit::Inside;
        }
        for i in 0..self.rows {
            if rect_contains(self.row_rect(i), x, y) {
                return Hit::Row(i);
            }
        }
        Hit::Inside
    }
}

/// Outcome of a left-click while a SilkBar menu/popover/context menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressResult {
    /// Action chosen; `closed` is true when the menu dismissed itself.
    Action { action: MenuAction, closed: bool },
    /// Clicked padding or non-interactive header inside the panel.
    Inside,
    /// Clicked outside the panel; menu closed.
    Dismiss,
}

/// Pure, host-testable state machine for SilkBar dropdowns, popovers, and context menus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controller {
    open: Option<MenuState>,
}

impl Controller {
    pub const fn new() -> Self {
        Self { open: None }
    }

    pub const fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub const fn state(&self) -> Option<MenuState> {
        self.open
    }

    pub fn open(&mut self, state: MenuState) {
        self.open = Some(state);
    }

    pub fn close(&mut self) -> bool {
        self.open.take().is_some()
    }

    pub fn wire(&self) -> (u64, u64) {
        self.open.map_or((0, 0), |s| s.wire())
    }

    /// Escape key dismisses any open menu. Returns true if it closed a menu.
    pub fn key_escape(&mut self) -> bool {
        self.close()
    }

    /// Smoothly switches active top-bar dropdown/popover when hovering over a different
    /// top-bar trigger while a top-bar menu is already open (macOS top bar behavior).
    pub fn hover_top_bar(&mut self, trigger: TopBarTrigger, anchor_x: i32, bar_h: i32, scale: u32) -> bool {
        let Some(cur) = self.open else { return false };
        if !cur.kind.is_top_bar() {
            return false;
        }
        let next_kind = trigger.menu_kind();
        if cur.kind == next_kind {
            return false;
        }
        let y = bar_h + 4 * scale.clamp(1, 3) as i32;
        let mut next = MenuState::new(next_kind, anchor_x, y);
        next.flags = cur.flags;
        next.volume = cur.volume;
        next.battery = cur.battery;
        next.wifi_phase = cur.wifi_phase;
        next.uptime_mins = cur.uptime_mins;
        next.active_space = cur.active_space;
        self.open = Some(next);
        true
    }

    /// Updates row highlight on pointer movement. Returns true if wire state changed.
    pub fn hover(&mut self, x: i32, y: i32, screen_w: usize, screen_h: usize, scale: u32) -> bool {
        let Some(mut cur) = self.open else { return false };
        let next_hover = if x >= 0 && y >= 0 {
            Layout::new(&cur, screen_w, screen_h, scale).and_then(|l| match l.hit(x as usize, y as usize) {
                Hit::Row(i) if cur.row(i).is_some_and(|r| !r.header && r.action != MenuAction::None) => Some(i as u8),
                _ => None,
            })
        } else {
            None
        };
        if next_hover == cur.hover {
            return false;
        }
        cur.hover = next_hover;
        self.open = Some(cur);
        true
    }

    /// Handles a left press while a menu is open.
    pub fn press(&mut self, x: i32, y: i32, screen_w: usize, screen_h: usize, scale: u32) -> PressResult {
        let Some(cur) = self.open else { return PressResult::Dismiss };
        let hit = if x >= 0 && y >= 0 {
            Layout::new(&cur, screen_w, screen_h, scale)
                .map_or(Hit::Outside, |l| l.hit(x as usize, y as usize))
        } else {
            Hit::Outside
        };
        match hit {
            Hit::Outside => {
                self.open = None;
                PressResult::Dismiss
            }
            Hit::Inside => PressResult::Inside,
            Hit::VolumeDown => PressResult::Action { action: MenuAction::VolumeDown, closed: false },
            Hit::VolumeUp => PressResult::Action { action: MenuAction::VolumeUp, closed: false },
            Hit::VolumeMute => PressResult::Action { action: MenuAction::VolumeMuteToggle, closed: false },
            Hit::VolumeBar(pct) => PressResult::Action { action: MenuAction::VolumeSet(pct), closed: false },
            Hit::Row(i) => match cur.row(i) {
                Some(r) if !r.header && r.action != MenuAction::None => {
                    let keep = r.action.keeps_open();
                    if !keep {
                        self.open = None;
                    }
                    PressResult::Action { action: r.action, closed: !keep }
                }
                _ => PressResult::Inside,
            },
        }
    }
}
