// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! OLauncher / macOS Spotlight-style Frosted Candy Lavender App Launcher & Search.
//!
//! Shell owns Spotlight state, keyboard/pointer navigation, live search,
//! inline arithmetic evaluation, and app launch dispatch. Display renders the
//! frosted Candy Lavender overlay using the exact same `Layout`, `CATALOG`,
//! `filter_catalog`, and `eval_calc` so hit-testing matches drawn pixels 1:1.
//!
//! Wire (Shell -> Display):
//! - `OP_SPOTLIGHT_QUERY` (`0x2FE`): packs up to 24 ASCII query bytes across `(q0, q1, q2)`.
//! - `OP_SPOTLIGHT` (`0x2FD`): `arg0 = w0` (0 = hidden; non-zero encodes open, category,
//!   selected index, hovered tile, hovered category, and query length).
//!
//! Self-contained (`#![no_std]`, zero heap) so host tests compile it directly.

/// Shell -> Display PDX opcode for Spotlight visibility, selection, category, and hover.
pub const OP_SPOTLIGHT: u64 = 0x2FD;
/// Shell -> Display PDX opcode for the 24-byte live search query text.
pub const OP_SPOTLIGHT_QUERY: u64 = 0x2FE;

/// Maximum ASCII characters kept in the Spotlight search input.
pub const QUERY_MAX: usize = 24;

/// Logical overlay dimensions at 1x scale (`~720x540`).
pub const PANEL_W: usize = 720;
pub const PANEL_H: usize = 540;

/// Grid dimensions: 5 columns x 4 rows = 20 visible SexOS apps per view.
pub const GRID_COLS: usize = 5;
pub const GRID_ROWS: usize = 4;

/// Search bar placeholder text.
pub const PLACEHOLDER: &str = "Search apps, calculate, or run...";
/// Footer shortcut bar text.
pub const FOOTER_HINTS: &str = "Arrows Select   Enter Open   Tab Filter   Esc Close";

/// Rectangle `(x, y, width, height)` in framebuffer pixels.
pub type Rect = (usize, usize, usize, usize);

/// True when `(px, py)` lies inside `r`.
pub const fn rect_contains(r: Rect, px: usize, py: usize) -> bool {
    px >= r.0 && py >= r.1 && px < r.0 + r.2 && py < r.1 + r.3
}

/// Spotlight category filter pills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Category {
    All = 0,
    Apps = 1,
    Media = 2,
    AiDev = 3,
    System = 4,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::All,
        Category::Apps,
        Category::Media,
        Category::AiDev,
        Category::System,
    ];

    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::All),
            1 => Some(Self::Apps),
            2 => Some(Self::Media),
            3 => Some(Self::AiDev),
            4 => Some(Self::System),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Apps => "Apps",
            Self::Media => "Media",
            Self::AiDev => "AI & Dev",
            Self::System => "System",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Apps,
            Self::Apps => Self::Media,
            Self::Media => Self::AiDev,
            Self::AiDev => Self::System,
            Self::System => Self::All,
        }
    }

    pub const fn prev(self) -> Self {
        match self {
            Self::All => Self::System,
            Self::Apps => Self::All,
            Self::Media => Self::Apps,
            Self::AiDev => Self::Media,
            Self::System => Self::AiDev,
        }
    }
}

/// Real SexOS launch target for a Spotlight app tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchTarget {
    /// Kaleidoscope browser (`open_browser_in_active_scene`).
    Browser,
    /// Spindle terminal (`open_spindle_in_active_scene`).
    Spindle,
    /// Quil document editor (`open_quil_in_active_scene`).
    Quil,
    /// PD Graph / Mesh visualizer (`open_mesh_in_active_scene`).
    PdGraph,
    /// Files manager (`program_start_own("loom")` with Linen fallback).
    Files,
    /// System Settings (`program_start_own("tatami")` with scene settings fallback).
    Settings,
    /// Audio/video player (`program_start_own("qupid")` with Quick Volume fallback).
    Audio,
    /// Named ELF program launched via `program_start_own(name)`.
    Program(&'static str),
    /// Direct custom program launch using the typed query when no catalog app matched.
    QueryProgram(Query),
}

impl LaunchTarget {
    /// Canonical ELF binary name for this launch target, when applicable.
    pub const fn program_name(self) -> Option<&'static str> {
        match self {
            Self::Browser => Some("kaleidoscope"),
            Self::Spindle => Some("spindle"),
            Self::Quil => Some("quil"),
            Self::PdGraph => Some("mesh"),
            Self::Files => Some("loom"),
            Self::Settings => Some("tatami"),
            Self::Audio => Some("qupid"),
            Self::Program(name) => Some(name),
            Self::QueryProgram(_) => None,
        }
    }
}

/// Canonical human-friendly program aliases mapped to SexOS program binary names.
const PROGRAM_ALIASES: &[(&str, &str)] = &[
    ("settings", "tatami"),
    ("preferences", "tatami"),
    ("prefs", "tatami"),
    ("system-settings", "tatami"),
    ("store", "sexshop"),
    ("appstore", "sexshop"),
    ("app-store", "sexshop"),
    ("packages", "sexshop"),
    ("pkg", "sexshop"),
    ("files", "loom"),
    ("finder", "loom"),
    ("explorer", "loom"),
    ("browser", "kaleidoscope"),
    ("web", "kaleidoscope"),
    ("internet", "kaleidoscope"),
    ("monitor", "mesh"),
    ("activity", "mesh"),
    ("tasks", "mesh"),
    ("top", "mesh"),
    ("htop", "mesh"),
    ("calendar", "date"),
    ("schedule", "date"),
    ("music", "qupid"),
    ("audio", "qupid"),
    ("player", "qupid"),
    ("vlc", "qupid"),
    ("chat", "flirt"),
    ("messages", "flirt"),
    ("messaging", "flirt"),
    ("daw", "moan-studio"),
    ("moan", "moan-studio"),
    ("synth", "moan-studio"),
    ("notes", "jot"),
    ("notepad", "jot"),
    ("editor", "vellum"),
    ("textedit", "vellum"),
    ("calc", "abacus"),
    ("calculator", "abacus"),
    ("ai", "tailor"),
    ("assistant", "tailor"),
    ("claude", "tailor"),
    ("codex", "tailor"),
    ("bridge", "tether"),
    ("remote", "tether"),
    ("mcp", "tether"),
    ("torrent", "swingers"),
    ("p2p", "swingers"),
    ("webcam", "flash"),
    ("camera", "flash"),
];

/// Zero-alloc lookup mapping human-friendly app aliases (case-insensitive) to
/// canonical SexOS program names. Unrecognized names are returned trimmed as-is.
pub fn resolve_program_alias(name: &str) -> &str {
    let trimmed = name.trim();
    for &(alias, target) in PROGRAM_ALIASES {
        if trimmed.eq_ignore_ascii_case(alias) {
            return target;
        }
    }
    trimmed
}

/// Descriptor for one app tile in the Spotlight grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpotlightApp {
    pub id: &'static str,
    pub name: &'static str,
    /// Exact `sex_icons` (`Kind::App`) icon name from `assets/icons/icons.txt`.
    pub icon: &'static str,
    /// Fallback 2-letter monogram used only if `sex_icons::find` returns `None`.
    pub monogram: &'static str,
    pub category: Category,
    /// Fallback rounded Candy icon badge RGB color (`0x00RRGGBB`).
    pub badge_rgb: u32,
    pub keywords: &'static str,
    pub target: LaunchTarget,
}

/// Curated SexOS Spotlight catalog — every tile launches a real shipped SexOS program.
/// First 20 entries populate the default 5x4 "All" grid; all entries are searchable and
/// browsable via category pills.
pub const CATALOG: [SpotlightApp; 42] = [
    SpotlightApp {
        id: "tailor",
        name: "Tailor AI",
        icon: "flirt",
        monogram: "AI",
        category: Category::AiDev,
        badge_rgb: 0x008B_5CF6,
        keywords: "tailor ai assistant chat llm gemini claude codex agent",
        target: LaunchTarget::Program("tailor"),
    },
    SpotlightApp {
        id: "tether",
        name: "Tether",
        icon: "flirt",
        monogram: "TH",
        category: Category::AiDev,
        badge_rgb: 0x00C6_A0F6,
        keywords: "tether antigravity acp remote agent bridge mcp waves snapshots hotswap",
        target: LaunchTarget::Program("tether"),
    },
    SpotlightApp {
        id: "kaleidoscope",
        name: "Kaleidoscope",
        icon: "kaleidoscope",
        monogram: "KB",
        category: Category::Apps,
        badge_rgb: 0x000E_A5E9,
        keywords: "kaleidoscope browser web internet http",
        target: LaunchTarget::Browser,
    },
    SpotlightApp {
        id: "brootlite",
        name: "BrootLite",
        icon: "splicy",
        monogram: "BL",
        category: Category::Media,
        badge_rgb: 0x007C_3AED,
        keywords: "brootlite stremio movies tv swarm torrent stream media",
        target: LaunchTarget::Program("brootlite"),
    },
    SpotlightApp {
        id: "qupid",
        name: "Qupid",
        icon: "qupid",
        monogram: "QP",
        category: Category::Media,
        badge_rgb: 0x00EC_4899,
        keywords: "qupid music audio video player vlc media sound mixer volume play",
        target: LaunchTarget::Audio,
    },
    SpotlightApp {
        id: "flash",
        name: "Flash",
        icon: "mirror",
        monogram: "FL",
        category: Category::Media,
        badge_rgb: 0x00F4_3F5E,
        keywords: "flash camera webcam photo booth video capture mirror",
        target: LaunchTarget::Program("flash"),
    },
    SpotlightApp {
        id: "spindle",
        name: "Spindle",
        icon: "spindle",
        monogram: ">_",
        category: Category::AiDev,
        badge_rgb: 0x0010_B981,
        keywords: "spindle terminal shell console bash tty",
        target: LaunchTarget::Spindle,
    },
    SpotlightApp {
        id: "files",
        name: "Loom",
        icon: "loom",
        monogram: "FS",
        category: Category::Apps,
        badge_rgb: 0x00F5_9E0B,
        keywords: "files loom linen finder folder explorer storage",
        target: LaunchTarget::Files,
    },
    SpotlightApp {
        id: "quil",
        name: "Quil",
        icon: "quil",
        monogram: "QL",
        category: Category::Apps,
        badge_rgb: 0x0063_66F1,
        keywords: "quil editor text notes write document",
        target: LaunchTarget::Quil,
    },
    SpotlightApp {
        id: "swingers",
        name: "Swingers",
        icon: "mesh",
        monogram: "SW",
        category: Category::Media,
        badge_rgb: 0x00D9_46EF,
        keywords: "swingers torrent p2p magnet download",
        target: LaunchTarget::Program("swingers"),
    },
    SpotlightApp {
        id: "settings",
        name: "Tatami",
        icon: "tatami",
        monogram: "ST",
        category: Category::System,
        badge_rgb: 0x007C_6A9A,
        keywords: "settings tatami preferences prefs system-settings wifi network display theme config",
        target: LaunchTarget::Settings,
    },
    SpotlightApp {
        id: "pd-graph",
        name: "Mesh",
        icon: "mesh",
        monogram: "PD",
        category: Category::System,
        badge_rgb: 0x00A8_55F7,
        keywords: "pd graph mesh domains capabilities monitor activity tasks top htop",
        target: LaunchTarget::PdGraph,
    },
    SpotlightApp {
        id: "sexshop",
        name: "SexShop",
        icon: "sexshop",
        monogram: "SS",
        category: Category::System,
        badge_rgb: 0x0014_B8A6,
        keywords: "store appstore app-store sexshop shop packages pkg install apps software",
        target: LaunchTarget::Program("sexshop"),
    },
    SpotlightApp {
        id: "moan",
        name: "Moan Studio",
        icon: "moan",
        monogram: "MN",
        category: Category::Media,
        badge_rgb: 0x00E1_1D48,
        keywords: "moan moan-studio daw synth audio music studio midi",
        target: LaunchTarget::Program("moan-studio"),
    },
    SpotlightApp {
        id: "splicy",
        name: "Splicy",
        icon: "splicy",
        monogram: "SP",
        category: Category::Media,
        badge_rgb: 0x00FF_2D7A,
        keywords: "splicy video editor fcp premiere film cut timeline transcript nle",
        target: LaunchTarget::Program("splicy"),
    },
    SpotlightApp {
        id: "cabaret",
        name: "Cabaret",
        icon: "splicy",
        monogram: "CB",
        category: Category::Media,
        badge_rgb: 0x00F9_7316,
        keywords: "cabaret live broadcast stream obs recorder studio video",
        target: LaunchTarget::Program("cabaret"),
    },
    SpotlightApp {
        id: "polyamour",
        name: "Polyamour",
        icon: "polyamour",
        monogram: "3D",
        category: Category::Apps,
        badge_rgb: 0x0038_BDF8,
        keywords: "polyamour 3d modeler sculpt cad mesh blender",
        target: LaunchTarget::Program("polyamour"),
    },
    SpotlightApp {
        id: "gossip",
        name: "Gossip",
        icon: "bell",
        monogram: "GS",
        category: Category::Media,
        badge_rgb: 0x00FB_7185,
        keywords: "gossip rss atom opml feeds podcasts news reader",
        target: LaunchTarget::Program("gossip"),
    },
    SpotlightApp {
        id: "vellum",
        name: "Vellum",
        icon: "vellum",
        monogram: "VL",
        category: Category::Apps,
        badge_rgb: 0x00EF_4444,
        keywords: "vellum editor textedit pdf document reader viewer",
        target: LaunchTarget::Program("vellum"),
    },
    SpotlightApp {
        id: "flirt",
        name: "Flirt",
        icon: "flirt",
        monogram: "FT",
        category: Category::Apps,
        badge_rgb: 0x00F4_72B6,
        keywords: "flirt talk chat messages messaging social",
        target: LaunchTarget::Program("flirt"),
    },
    SpotlightApp {
        id: "gaze",
        name: "Gaze",
        icon: "gaze",
        monogram: "GZ",
        category: Category::Media,
        badge_rgb: 0x0006_B6D4,
        keywords: "gaze view image photo gallery svg jxl picture",
        target: LaunchTarget::Program("gaze"),
    },
    SpotlightApp {
        id: "batik",
        name: "Batik",
        icon: "gaze",
        monogram: "BT",
        category: Category::Media,
        badge_rgb: 0x00C0_84FC,
        keywords: "batik paint draw image editor layers art",
        target: LaunchTarget::Program("batik"),
    },
    SpotlightApp {
        id: "abacus",
        name: "Abacus",
        icon: "abacus",
        monogram: "AB",
        category: Category::Apps,
        badge_rgb: 0x0022_C55E,
        keywords: "abacus calc calculator math compute",
        target: LaunchTarget::Program("abacus"),
    },
    SpotlightApp {
        id: "date",
        name: "Date",
        icon: "date",
        monogram: "DT",
        category: Category::Apps,
        badge_rgb: 0x00E8_79F9,
        keywords: "date calendar schedule events agenda",
        target: LaunchTarget::Program("date"),
    },
    SpotlightApp {
        id: "assuit",
        name: "Assuit",
        icon: "assuit",
        monogram: "AS",
        category: Category::Apps,
        badge_rgb: 0x003B_82F6,
        keywords: "assuit word processor docx writer document",
        target: LaunchTarget::Program("assuit"),
    },
    SpotlightApp {
        id: "spreadeagle",
        name: "Spreadeagle",
        icon: "spreadeagle",
        monogram: "SE",
        category: Category::Apps,
        badge_rgb: 0x0010_B981,
        keywords: "spreadeagle spreadsheet excel xlsx sheets table",
        target: LaunchTarget::Program("spreadeagle"),
    },
    SpotlightApp {
        id: "gestalt",
        name: "Gestalt",
        icon: "gestalt",
        monogram: "GT",
        category: Category::Apps,
        badge_rgb: 0x00F5_9E0B,
        keywords: "gestalt vector svg illustrator paths design",
        target: LaunchTarget::Program("gestalt"),
    },
    SpotlightApp {
        id: "burlesque",
        name: "Burlesque",
        icon: "vellum",
        monogram: "BQ",
        category: Category::Apps,
        badge_rgb: 0x00FB_923C,
        keywords: "burlesque slides presentations deck keynote",
        target: LaunchTarget::Program("burlesque"),
    },
    SpotlightApp {
        id: "bodice",
        name: "Bodice",
        icon: "vellum",
        monogram: "BD",
        category: Category::Apps,
        badge_rgb: 0x00A7_8BFA,
        keywords: "bodice ebook epub comics books reader",
        target: LaunchTarget::Program("bodice"),
    },
    SpotlightApp {
        id: "jot",
        name: "Jot",
        icon: "jot",
        monogram: "JT",
        category: Category::Apps,
        badge_rgb: 0x0081_8CF8,
        keywords: "jot notes notepad memo scratchpad quick",
        target: LaunchTarget::Program("jot"),
    },
    SpotlightApp {
        id: "filigree",
        name: "Filigree",
        icon: "quil",
        monogram: "FG",
        category: Category::Apps,
        badge_rgb: 0x0094_A3B8,
        keywords: "filigree fonts typography specimen",
        target: LaunchTarget::Program("filigree"),
    },
    SpotlightApp {
        id: "rendezvous",
        name: "Rendezvous",
        icon: "kaleidoscope",
        monogram: "RV",
        category: Category::Apps,
        badge_rgb: 0x0034_D399,
        keywords: "rendezvous maps navigation routes location",
        target: LaunchTarget::Program("rendezvous"),
    },
    SpotlightApp {
        id: "chastity",
        name: "Chastity",
        icon: "tatami",
        monogram: "CH",
        category: Category::System,
        badge_rgb: 0x00F4_3F5E,
        keywords: "chastity password vault keepass kdbx secrets",
        target: LaunchTarget::Program("chastity"),
    },
    SpotlightApp {
        id: "corset",
        name: "Corset",
        icon: "tatami",
        monogram: "CS",
        category: Category::System,
        badge_rgb: 0x0064_748B,
        keywords: "corset firewall security disks partitions format storage",
        target: LaunchTarget::Program("corset"),
    },
    SpotlightApp {
        id: "aftercare",
        name: "Aftercare",
        icon: "loom",
        monogram: "AC",
        category: Category::System,
        badge_rgb: 0x002D_D4BF,
        keywords: "aftercare backup encrypted snapshots restore",
        target: LaunchTarget::Program("aftercare"),
    },
    SpotlightApp {
        id: "climax",
        name: "Climax",
        icon: "mesh",
        monogram: "CX",
        category: Category::System,
        badge_rgb: 0x00EF_4444,
        keywords: "climax benchmark thermal stress cpu gpu",
        target: LaunchTarget::Program("climax"),
    },
    SpotlightApp {
        id: "hookup",
        name: "Hookup",
        icon: "flirt",
        monogram: "HK",
        category: Category::System,
        badge_rgb: 0x0038_BDF8,
        keywords: "hookup bluetooth bt wireless phone link kdeconnect mobile sync",
        target: LaunchTarget::Program("hookup"),
    },
    SpotlightApp {
        id: "lurk",
        name: "Lurk",
        icon: "mirror",
        monogram: "LK",
        category: Category::System,
        badge_rgb: 0x0063_66F1,
        keywords: "lurk vnc rfb remote desktop viewer",
        target: LaunchTarget::Program("lurk"),
    },
    SpotlightApp {
        id: "needle",
        name: "Needle",
        icon: "vellum",
        monogram: "ND",
        category: Category::System,
        badge_rgb: 0x008B_5CF6,
        keywords: "needle print scan ipp printer scanner",
        target: LaunchTarget::Program("needle"),
    },
    SpotlightApp {
        id: "drag",
        name: "Drag",
        icon: "spindle",
        monogram: "VM",
        category: Category::AiDev,
        badge_rgb: 0x004F_46E5,
        keywords: "drag vm virtual machine linux kvm vmm",
        target: LaunchTarget::Program("drag"),
    },
    SpotlightApp {
        id: "velvet",
        name: "Velvet",
        icon: "polyamour",
        monogram: "VV",
        category: Category::Apps,
        badge_rgb: 0x00EC_4899,
        keywords: "velvet games solitaire chess arcade",
        target: LaunchTarget::Program("velvet"),
    },
    SpotlightApp {
        id: "driftspark",
        name: "Driftspark",
        icon: "splicy",
        monogram: "DS",
        category: Category::Media,
        badge_rgb: 0x00F9_7316,
        keywords: "driftspark game arcade racing sdl2",
        target: LaunchTarget::Program("driftspark"),
    },
];

pub const CATALOG_LEN: usize = CATALOG.len();

/// Fixed-capacity ASCII search query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query {
    buf: [u8; QUERY_MAX],
    len: u8,
}

impl Query {
    pub const fn new() -> Self {
        Self { buf: [0; QUERY_MAX], len: 0 }
    }

    pub fn push(&mut self, c: u8) -> bool {
        if self.len as usize >= QUERY_MAX || !(32..=126).contains(&c) {
            return false;
        }
        if c == b' ' && self.len == 0 {
            return false;
        }
        self.buf[self.len as usize] = c;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        self.len -= 1;
        self.buf[self.len as usize] = 0;
        true
    }

    pub fn clear(&mut self) {
        self.buf = [0; QUERY_MAX];
        self.len = 0;
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.len as usize]
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }

    /// Pack all 24 query bytes into three `u64` words for `OP_SPOTLIGHT_QUERY`.
    pub fn pack_words(&self) -> (u64, u64, u64) {
        let mut w = [0u64; 3];
        let mut i = 0usize;
        while i < QUERY_MAX {
            let b = if i < self.len as usize { self.buf[i] } else { 0 };
            w[i / 8] |= (b as u64) << ((i % 8) * 8);
            i += 1;
        }
        (w[0], w[1], w[2])
    }

    /// Unpack 24 query bytes from `OP_SPOTLIGHT_QUERY` with explicit length.
    pub fn unpack_words(q0: u64, q1: u64, q2: u64, len: u8) -> Self {
        let words = [q0, q1, q2];
        let mut q = Self::new();
        let clamped = (len as usize).min(QUERY_MAX);
        let mut i = 0usize;
        while i < clamped {
            let b = ((words[i / 8] >> ((i % 8) * 8)) & 0xFF) as u8;
            q.buf[i] = if (32..=126).contains(&b) { b } else { b' ' };
            i += 1;
        }
        q.len = clamped as u8;
        q
    }
}

/// Set-1 scancode to ASCII for Spotlight typing (letters, digits, space, and
/// arithmetic symbols `+ - * / % ( ) .`).
pub fn key_char(code: u8, shift: bool) -> Option<u8> {
    const TOP: &[u8; 10] = b"qwertyuiop";
    const HOME: &[u8; 9] = b"asdfghjkl";
    const BOTTOM: &[u8; 7] = b"zxcvbnm";
    match (code, shift) {
        (0x0D, true) | (0x4E, _) => Some(b'+'),
        (0x09, true) | (0x37, _) => Some(b'*'),
        (0x0A, true) => Some(b'('),
        (0x0B, true) => Some(b')'),
        (0x06, true) => Some(b'%'),
        (0x0C, false) | (0x4A, _) => Some(b'-'),
        (0x0C, true) => Some(b'_'),
        (0x34, false) => Some(b'.'),
        (0x35, false) => Some(b'/'),
        (0x02..=0x0A, false) => Some(b'1' + (code - 0x02)),
        (0x0B, false) => Some(b'0'),
        (0x10..=0x19, _) => Some(TOP[(code - 0x10) as usize]),
        (0x1E..=0x26, _) => Some(HOME[(code - 0x1E) as usize]),
        (0x2C..=0x32, _) => Some(BOTTOM[(code - 0x2C) as usize]),
        (0x39, _) => Some(b' '),
        _ => None,
    }
}

/// Case-insensitive substring/subsequence score; higher is better.
fn score_text(query: &str, text: &str) -> Option<u32> {
    let q = query.as_bytes();
    let t = text.as_bytes();
    let mut q_clean = [0u8; QUERY_MAX];
    let mut q_len = 0usize;
    for &b in q {
        if b != b' ' && q_len < QUERY_MAX {
            q_clean[q_len] = b.to_ascii_lowercase();
            q_len += 1;
        }
    }
    if q_len == 0 {
        return Some(0);
    }

    // 1. Direct case-insensitive prefix or contiguous substring match scores highest.
    if q_len <= t.len() {
        for start in 0..=(t.len() - q_len) {
            let mut matched = true;
            for j in 0..q_len {
                if t[start + j].to_ascii_lowercase() != q_clean[j] {
                    matched = false;
                    break;
                }
            }
            if matched {
                let prefix_bonus = if start == 0 { 400 } else if !t[start - 1].is_ascii_alphanumeric() { 260 } else { 180 };
                return Some(prefix_bonus + 64u32.saturating_sub(t.len() as u32));
            }
        }
    }

    // 2. Ordered fuzzy subsequence fallback.
    let mut score = 0u32;
    let mut at = 0usize;
    let mut last: Option<usize> = None;
    for &qc in &q_clean[..q_len] {
        let found = (at..t.len()).find(|&i| t[i].to_ascii_lowercase() == qc)?;
        score += 2;
        if found == 0 || !t[found - 1].is_ascii_alphanumeric() {
            score += 8;
        }
        if last.is_some_and(|l| l + 1 == found) {
            score += 5;
        }
        last = Some(found);
        at = found + 1;
    }
    Some(score * 4 + 32u32.saturating_sub(t.len() as u32))
}

/// Filter and rank `CATALOG` indices by `category` and live `query`.
/// Returns `(indices, count)` in best-match order (preserving catalog order on ties).
pub fn filter_catalog(category: Category, query: &str) -> ([usize; CATALOG_LEN], usize) {
    let mut out = [0usize; CATALOG_LEN];
    let mut scores = [0u32; CATALOG_LEN];
    let mut n = 0usize;
    let trimmed = query.trim();
    let canonical = if trimmed.is_empty() { "" } else { resolve_program_alias(trimmed) };
    let has_alias = !trimmed.is_empty() && !canonical.eq_ignore_ascii_case(trimmed);

    for (idx, app) in CATALOG.iter().enumerate() {
        if category != Category::All && app.category != category {
            continue;
        }
        let s = if trimmed.is_empty() {
            0
        } else {
            let alias_bonus = if has_alias && app.target.program_name() == Some(canonical) {
                1000
            } else {
                0
            };
            let by_name = score_text(trimmed, app.name).map(|v| v * 2);
            let by_id = score_text(trimmed, app.id).map(|v| v * 2);
            let by_kw = app
                .keywords
                .split(' ')
                .filter_map(|w| score_text(trimmed, w))
                .max()
                .or_else(|| trimmed.contains(' ').then(|| score_text(trimmed, app.keywords)).flatten());
            match by_name.max(by_id).max(by_kw) {
                Some(v) => v + alias_bonus,
                None if alias_bonus > 0 => alias_bonus,
                None => continue,
            }
        };
        let mut j = n;
        while j > 0 && scores[j - 1] < s {
            out[j] = out[j - 1];
            scores[j] = scores[j - 1];
            j -= 1;
        }
        out[j] = idx;
        scores[j] = s;
        n += 1;
    }
    (out, n)
}

/// Formatted inline calculator result (e.g. `"= 4"` or `"= 3.5"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalcResult {
    buf: [u8; 32],
    len: usize,
}

impl CalcResult {
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

/// Fixed-point 1e-4 scaled integer evaluator (`SCALE = 10_000`) for `#![no_std]`
/// arithmetic without float rounding noise.
const CALC_SCALE: i64 = 10_000;

struct CalcParser<'a> {
    src: &'a [u8],
    pos: usize,
    saw_op: bool,
}

impl<'a> CalcParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos] == b' ' {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.src.get(self.pos).copied()
    }

    fn parse_expr(&mut self) -> Option<i64> {
        let mut acc = self.parse_term()?;
        while let Some(op) = self.peek() {
            if op != b'+' && op != b'-' {
                break;
            }
            self.pos += 1;
            self.saw_op = true;
            let rhs = self.parse_term()?;
            acc = if op == b'+' { acc.checked_add(rhs)? } else { acc.checked_sub(rhs)? };
        }
        Some(acc)
    }

    fn parse_term(&mut self) -> Option<i64> {
        let mut acc = self.parse_unary()?;
        while let Some(op) = self.peek() {
            if op != b'*' && op != b'/' && op != b'%' {
                break;
            }
            self.pos += 1;
            self.saw_op = true;
            let rhs = self.parse_unary()?;
            acc = match op {
                b'*' => {
                    let prod = (acc as i128).checked_mul(rhs as i128)? / (CALC_SCALE as i128);
                    i64::try_from(prod).ok()?
                }
                b'/' => {
                    if rhs == 0 {
                        return None;
                    }
                    let num = (acc as i128).checked_mul(CALC_SCALE as i128)?;
                    let q = num / (rhs as i128);
                    i64::try_from(q).ok()?
                }
                b'%' => {
                    if rhs == 0 {
                        return None;
                    }
                    acc % rhs
                }
                _ => return None,
            };
        }
        Some(acc)
    }

    fn parse_unary(&mut self) -> Option<i64> {
        match self.peek()? {
            b'-' => {
                self.pos += 1;
                self.saw_op = true;
                self.parse_unary()?.checked_neg()
            }
            b'+' => {
                self.pos += 1;
                self.saw_op = true;
                self.parse_unary()
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Option<i64> {
        if self.peek()? == b'(' {
            self.pos += 1;
            self.saw_op = true;
            let v = self.parse_expr()?;
            if self.peek()? != b')' {
                return None;
            }
            self.pos += 1;
            return Some(v);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Option<i64> {
        self.skip_ws();
        let mut int_part: i64 = 0;
        let mut digits = 0usize;
        while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
            int_part = int_part.checked_mul(10)?.checked_add((self.src[self.pos] - b'0') as i64)?;
            self.pos += 1;
            digits += 1;
        }
        let mut frac_part: i64 = 0;
        let mut frac_scale: i64 = CALC_SCALE;
        if self.pos < self.src.len() && self.src[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                digits += 1;
                if frac_scale >= 10 {
                    frac_scale /= 10;
                    frac_part += ((self.src[self.pos] - b'0') as i64) * frac_scale;
                }
                self.pos += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        int_part.checked_mul(CALC_SCALE)?.checked_add(frac_part)
    }
}

/// Evaluates an inline arithmetic expression (e.g. `"2+2"` -> `"= 4"`, `"7/2"` -> `"= 3.5"`).
/// Returns `None` if the query is not a valid arithmetic expression or has no operators.
pub fn eval_calc(query: &str) -> Option<CalcResult> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parser = CalcParser { src: trimmed.as_bytes(), pos: 0, saw_op: false };
    let val = parser.parse_expr()?;
    parser.skip_ws();
    if parser.pos != parser.src.len() || !parser.saw_op {
        return None;
    }

    let mut res = CalcResult { buf: [0; 32], len: 0 };
    let push = |r: &mut CalcResult, b: u8| {
        if r.len < r.buf.len() {
            r.buf[r.len] = b;
            r.len += 1;
        }
    };
    push(&mut res, b'=');
    push(&mut res, b' ');
    let neg = val < 0;
    let abs_val = (val as i128).abs();
    if neg {
        push(&mut res, b'-');
    }
    let int_part = (abs_val / CALC_SCALE as i128) as u64;
    let frac_part = (abs_val % CALC_SCALE as i128) as u32;

    let mut digits = [0u8; 20];
    let mut dn = 0usize;
    let mut v = int_part;
    loop {
        digits[dn] = b'0' + (v % 10) as u8;
        dn += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    while dn > 0 {
        dn -= 1;
        push(&mut res, digits[dn]);
    }

    if frac_part != 0 {
        push(&mut res, b'.');
        let f = [
            b'0' + ((frac_part / 1000) % 10) as u8,
            b'0' + ((frac_part / 100) % 10) as u8,
            b'0' + ((frac_part / 10) % 10) as u8,
            b'0' + (frac_part % 10) as u8,
        ];
        let mut flen = 4usize;
        while flen > 0 && f[flen - 1] == b'0' {
            flen -= 1;
        }
        for &b in &f[..flen] {
            push(&mut res, b);
        }
    }
    Some(res)
}

/// Shared Spotlight state transferred from Shell to Display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpotlightState {
    pub open: bool,
    pub category: Category,
    pub selected: u8,
    pub hover_tile: Option<u8>,
    pub hover_cat: Option<u8>,
    pub query: Query,
}

impl SpotlightState {
    pub const fn closed() -> Self {
        Self {
            open: false,
            category: Category::All,
            selected: 0,
            hover_tile: None,
            hover_cat: None,
            query: Query::new(),
        }
    }

    /// Encode state into `OP_SPOTLIGHT` `(w0, w1, w2)`. `w0 == 0` means closed.
    pub fn wire(&self) -> (u64, u64, u64) {
        if !self.open {
            return (0, 0, 0);
        }
        let ht = self.hover_tile.unwrap_or(0xFF) as u64;
        let hc = self.hover_cat.unwrap_or(0xFF) as u64;
        let w0 = 1u64
            | ((self.category as u64 & 0x7) << 1)
            | ((self.selected as u64) << 8)
            | (ht << 16)
            | ((self.query.len() as u64 & 0xFF) << 24)
            | (hc << 32);
        (w0, 0, 0)
    }

    /// Decode `OP_SPOTLIGHT` `(w0, _w1, _w2)` paired with query bytes `(q0, q1, q2)`.
    pub fn decode(w0: u64, q0: u64, q1: u64, q2: u64) -> Option<Self> {
        if w0 == 0 {
            return Some(Self::closed());
        }
        if (w0 & 1) == 0 || (w0 >> 40) != 0 {
            return None;
        }
        let cat = Category::from_u8(((w0 >> 1) & 0x7) as u8)?;
        let selected = ((w0 >> 8) & 0xFF) as u8;
        let ht_raw = ((w0 >> 16) & 0xFF) as u8;
        let hover_tile = if ht_raw == 0xFF { None } else { Some(ht_raw) };
        let qlen = ((w0 >> 24) & 0xFF) as u8;
        if qlen as usize > QUERY_MAX {
            return None;
        }
        let hc_raw = ((w0 >> 32) & 0xFF) as u8;
        let hover_cat = if hc_raw == 0xFF {
            None
        } else if hc_raw < Category::ALL.len() as u8 {
            Some(hc_raw)
        } else {
            return None;
        };
        let query = Query::unpack_words(q0, q1, q2, qlen);
        Some(Self {
            open: true,
            category: cat,
            selected,
            hover_tile,
            hover_cat,
            query,
        })
    }
}

/// Shared pixel geometry for Spotlight rendering and pointer hit-testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub scale: u32,
    pub panel: Rect,
    pub search_box: Rect,
    pub category_pills: [Rect; 5],
    pub grid_area: Rect,
    pub tile_w: usize,
    pub tile_h: usize,
    pub tile_gap_x: usize,
    pub tile_gap_y: usize,
    pub footer: Rect,
}

/// Hit-test result for a pointer position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Outside,
    SearchBox,
    Category(Category),
    Tile(usize),
    Panel,
}

impl Layout {
    /// Computes the centered, screen-clamped `~720x540` logical px Spotlight layout.
    pub fn new(screen_w: usize, screen_h: usize, bar_h: usize, scale: u32) -> Option<Self> {
        let sc = scale.clamp(1, 3);
        let s = sc as usize;
        let margin = 8 * s;
        let avail_w = screen_w.saturating_sub(2 * margin);
        let avail_h = screen_h.saturating_sub(bar_h + 2 * margin);
        if avail_w < 240 * s || avail_h < 220 * s {
            return None;
        }

        let pw = (PANEL_W * s).min(avail_w);
        let ph = (PANEL_H * s).min(avail_h);
        let px = (screen_w.saturating_sub(pw)) / 2;
        let py = (bar_h + margin + avail_h.saturating_sub(ph) / 3)
            .min(screen_h.saturating_sub(ph + margin))
            .max(bar_h + margin);
        let panel = (px, py, pw, ph);

        let pad = 18 * s;
        let inner_x = px + pad;
        let inner_w = pw.saturating_sub(2 * pad);

        // 1. Top rounded Search Input bar
        let search_h = 46 * s;
        let search_box = (inner_x, py + pad, inner_w, search_h);

        // 2. Category Filter Pills row: All | Apps | Media | AI & Dev | System
        let pill_y = search_box.1 + search_box.3 + 12 * s;
        let pill_h = 28 * s;
        let pill_gap = 8 * s;
        let widths = [52 * s, 66 * s, 72 * s, 96 * s, 82 * s];
        let mut category_pills = [(0, 0, 0, 0); 5];
        let mut cx = inner_x;
        let mut i = 0usize;
        while i < 5 {
            let w = widths[i].min(inner_x + inner_w - cx.min(inner_x + inner_w));
            category_pills[i] = (cx, pill_y, w, pill_h);
            cx += w + pill_gap;
            i += 1;
        }

        // 3. Footer Shortcut Bar
        let footer_h = 34 * s;
        let footer = (px + pad, py + ph - footer_h, inner_w, footer_h);

        // 4. App Icon Grid (5 columns x 4 rows)
        let grid_y = pill_y + pill_h + 14 * s;
        let grid_h = footer.1.saturating_sub(grid_y + 10 * s);
        let grid_area = (inner_x, grid_y, inner_w, grid_h);
        let tile_gap_x = 12 * s;
        let tile_gap_y = 10 * s;
        let tile_w = grid_area.2.saturating_sub(tile_gap_x * (GRID_COLS - 1)) / GRID_COLS;
        let tile_h = grid_area.3.saturating_sub(tile_gap_y * (GRID_ROWS - 1)) / GRID_ROWS;

        Some(Self {
            scale: sc,
            panel,
            search_box,
            category_pills,
            grid_area,
            tile_w,
            tile_h,
            tile_gap_x,
            tile_gap_y,
            footer,
        })
    }

    /// Rectangle of visible grid slot `index` (`0..GRID_COLS * GRID_ROWS`).
    pub fn tile_rect(&self, index: usize) -> Option<Rect> {
        if index >= GRID_COLS * GRID_ROWS {
            return None;
        }
        let col = index % GRID_COLS;
        let row = index / GRID_COLS;
        let x = self.grid_area.0 + col * (self.tile_w + self.tile_gap_x);
        let y = self.grid_area.1 + row * (self.tile_h + self.tile_gap_y);
        Some((x, y, self.tile_w, self.tile_h))
    }

    /// Hit-tests `(px, py)` against the Spotlight layout for `visible_tiles` shown tiles.
    pub fn hit(&self, px: i32, py: i32, visible_tiles: usize) -> Hit {
        if px < 0 || py < 0 {
            return Hit::Outside;
        }
        let (ux, uy) = (px as usize, py as usize);
        if !rect_contains(self.panel, ux, uy) {
            return Hit::Outside;
        }
        if rect_contains(self.search_box, ux, uy) {
            return Hit::SearchBox;
        }
        for (i, &r) in self.category_pills.iter().enumerate() {
            if r.2 > 0 && rect_contains(r, ux, uy) {
                return Hit::Category(Category::ALL[i]);
            }
        }
        let count = visible_tiles.min(GRID_COLS * GRID_ROWS);
        for i in 0..count {
            if let Some(r) = self.tile_rect(i) {
                if rect_contains(r, ux, uy) {
                    return Hit::Tile(i);
                }
            }
        }
        Hit::Panel
    }
}

/// Outcome of handling a key event inside Spotlight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Spotlight was not open; caller should pass the key along.
    Ignored,
    /// Key was consumed with no visual change.
    Consumed,
    /// Visual state changed; Shell should sync Display.
    Redraw,
    /// Spotlight closed (e.g. Escape).
    Close,
    /// User pressed Enter on a tile; Spotlight closed and Shell should launch `target`.
    Launch(LaunchTarget),
}

/// Outcome of handling a left mouse press inside Spotlight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClickOutcome {
    Ignored,
    Inside,
    Redraw,
    Dismiss,
    Launch(LaunchTarget),
}

/// Pure Spotlight controller state machine.
pub struct Controller {
    state: SpotlightState,
}

impl Controller {
    pub const fn new() -> Self {
        Self { state: SpotlightState::closed() }
    }

    pub const fn is_open(&self) -> bool {
        self.state.open
    }

    pub const fn state(&self) -> SpotlightState {
        self.state
    }

    pub fn open(&mut self) {
        self.state = SpotlightState {
            open: true,
            category: Category::All,
            selected: 0,
            hover_tile: None,
            hover_cat: None,
            query: Query::new(),
        };
    }

    pub fn close(&mut self) -> bool {
        if !self.state.open {
            return false;
        }
        self.state = SpotlightState::closed();
        true
    }

    pub fn toggle(&mut self) -> bool {
        if self.state.open {
            self.close();
            false
        } else {
            self.open();
            true
        }
    }

    pub fn filtered(&self) -> ([usize; CATALOG_LEN], usize) {
        filter_catalog(self.state.category, self.state.query.as_str())
    }

    pub fn calc_result(&self) -> Option<CalcResult> {
        eval_calc(self.state.query.as_str())
    }

    pub fn wire(&self) -> (u64, u64, u64) {
        self.state.wire()
    }

    pub fn wire_query(&self) -> (u64, u64, u64) {
        self.state.query.pack_words()
    }

    /// Handles a keyboard event while Spotlight is open.
    pub fn handle_key(&mut self, code: u8, value: u64, ctrl: bool, shift: bool) -> KeyOutcome {
        if !self.state.open {
            return KeyOutcome::Ignored;
        }
        // Consume key releases so nothing leaks to the underlying app.
        if value == 0 {
            return KeyOutcome::Consumed;
        }

        // Escape closes Spotlight cleanly.
        if code == 0x01 {
            if value == 1 {
                self.close();
                return KeyOutcome::Close;
            }
            return KeyOutcome::Consumed;
        }

        // Enter launches the currently selected tile (if any), or runs the typed query
        // directly as a program name when there are 0 catalog matches, and closes Spotlight.
        if code == 0x1C || code == 0x9C {
            if value == 1 {
                let (order, count) = self.filtered();
                if count > 0 {
                    let shown = count.min(GRID_COLS * GRID_ROWS);
                    let sel = (self.state.selected as usize).min(shown - 1);
                    let target = CATALOG[order[sel]].target;
                    self.close();
                    return KeyOutcome::Launch(target);
                }
                let q = self.state.query;
                if !q.as_str().trim().is_empty() && eval_calc(q.as_str()).is_none() {
                    self.close();
                    return KeyOutcome::Launch(LaunchTarget::QueryProgram(q));
                }
            }
            return KeyOutcome::Consumed;
        }

        // Tab / Shift+Tab switches category filter pills.
        if code == 0x0F {
            self.state.category = if shift { self.state.category.prev() } else { self.state.category.next() };
            self.state.selected = 0;
            return KeyOutcome::Redraw;
        }

        // Ctrl+Left / Ctrl+Right switches category filter pills.
        if ctrl && matches!(code, 0x4B | 0x69) {
            self.state.category = self.state.category.prev();
            self.state.selected = 0;
            return KeyOutcome::Redraw;
        }
        if ctrl && matches!(code, 0x4D | 0x6A) {
            self.state.category = self.state.category.next();
            self.state.selected = 0;
            return KeyOutcome::Redraw;
        }

        // Arrow keys navigate the visible 2D app grid.
        let (_, count) = self.filtered();
        let shown = count.min(GRID_COLS * GRID_ROWS);
        let is_left = matches!(code, 0x4B | 0x69);
        let is_right = matches!(code, 0x4D | 0x6A);
        let is_up = matches!(code, 0x48 | 0x67);
        let is_down = matches!(code, 0x50 | 0x6C);
        if is_left || is_right || is_up || is_down {
            if shown == 0 {
                return KeyOutcome::Consumed;
            }
            let cur = (self.state.selected as usize).min(shown - 1);
            let next = if is_left {
                if cur == 0 { shown - 1 } else { cur - 1 }
            } else if is_right {
                if cur + 1 >= shown { 0 } else { cur + 1 }
            } else if is_up {
                if cur >= GRID_COLS { cur - GRID_COLS } else { cur }
            } else {
                // Down
                if cur + GRID_COLS < shown { cur + GRID_COLS } else { cur }
            };
            if next != cur {
                self.state.selected = next as u8;
                return KeyOutcome::Redraw;
            }
            return KeyOutcome::Consumed;
        }

        // Backspace edits query live.
        if code == 0x0E {
            if self.state.query.pop() {
                self.state.selected = 0;
                return KeyOutcome::Redraw;
            }
            return KeyOutcome::Consumed;
        }

        // Printable typing filters live (ignore Ctrl+letter chords).
        if !ctrl {
            if let Some(ch) = key_char(code, shift) {
                if self.state.query.push(ch) {
                    self.state.selected = 0;
                    return KeyOutcome::Redraw;
                }
            }
        }
        KeyOutcome::Consumed
    }

    /// Handles a left mouse press while Spotlight is open.
    pub fn handle_press(&mut self, px: i32, py: i32, w: usize, h: usize, bar_h: usize, scale: u32) -> ClickOutcome {
        if !self.state.open {
            return ClickOutcome::Ignored;
        }
        let Some(layout) = Layout::new(w, h, bar_h, scale) else {
            self.close();
            return ClickOutcome::Dismiss;
        };
        let (order, count) = self.filtered();
        let shown = count.min(GRID_COLS * GRID_ROWS);
        match layout.hit(px, py, shown) {
            Hit::Outside => {
                self.close();
                ClickOutcome::Dismiss
            }
            Hit::Category(cat) => {
                if self.state.category != cat {
                    self.state.category = cat;
                    self.state.selected = 0;
                    ClickOutcome::Redraw
                } else {
                    ClickOutcome::Inside
                }
            }
            Hit::Tile(idx) => {
                if idx < shown {
                    let target = CATALOG[order[idx]].target;
                    self.close();
                    ClickOutcome::Launch(target)
                } else {
                    ClickOutcome::Inside
                }
            }
            Hit::SearchBox | Hit::Panel => ClickOutcome::Inside,
        }
    }

    /// Updates hover highlight for tiles and category pills. Returns true if state changed.
    pub fn handle_hover(&mut self, px: i32, py: i32, w: usize, h: usize, bar_h: usize, scale: u32) -> bool {
        if !self.state.open {
            return false;
        }
        let Some(layout) = Layout::new(w, h, bar_h, scale) else {
            return false;
        };
        let (_, count) = self.filtered();
        let (next_tile, next_cat) = match layout.hit(px, py, count) {
            Hit::Tile(i) => (Some(i as u8), None),
            Hit::Category(c) => (None, Some(c as u8)),
            _ => (None, None),
        };
        if self.state.hover_tile != next_tile || self.state.hover_cat != next_cat {
            self.state.hover_tile = next_tile;
            self.state.hover_cat = next_cat;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flagship_apps_found_by_query_and_category() {
        let cases: &[(&str, Category, &str, LaunchTarget)] = &[
            ("brootlite", Category::Media, "brootlite", LaunchTarget::Program("brootlite")),
            ("qupid", Category::Media, "qupid", LaunchTarget::Audio),
            ("flash", Category::Media, "flash", LaunchTarget::Program("flash")),
            ("splicy", Category::Media, "splicy", LaunchTarget::Program("splicy")),
            ("tailor", Category::AiDev, "tailor", LaunchTarget::Program("tailor")),
            ("kaleidoscope", Category::Apps, "kaleidoscope", LaunchTarget::Browser),
            ("tatami", Category::System, "settings", LaunchTarget::Settings),
            ("swingers", Category::Media, "swingers", LaunchTarget::Program("swingers")),
        ];
        for &(query, cat, expected_id, expected_target) in cases {
            let (order_all, n_all) = filter_catalog(Category::All, query);
            assert!(n_all >= 1, "expected match in All for {}", query);
            assert_eq!(CATALOG[order_all[0]].id, expected_id);
            assert_eq!(CATALOG[order_all[0]].target, expected_target);

            let (order_cat, n_cat) = filter_catalog(cat, query);
            assert!(n_cat >= 1, "expected match in {:?} for {}", cat, query);
            assert_eq!(CATALOG[order_cat[0]].id, expected_id);
        }
    }

    #[test]
    fn wire_round_trip_for_all_valid_tile_indices() {
        let max_tiles = GRID_COLS * GRID_ROWS;
        let mut q = Query::new();
        for &b in b"brootlite" {
            assert!(q.push(b));
        }
        let (q0, q1, q2) = q.pack_words();
        for idx in 0..max_tiles {
            let st = SpotlightState {
                open: true,
                category: Category::ALL[idx % Category::ALL.len()],
                selected: idx as u8,
                hover_tile: Some(idx as u8),
                hover_cat: None,
                query: q,
            };
            let (w0, _, _) = st.wire();
            assert_eq!(SpotlightState::decode(w0, q0, q1, q2), Some(st));
        }
    }

    #[test]
    fn direct_custom_program_launch_when_no_catalog_match() {
        let mut ctrl = Controller::new();
        ctrl.open();
        // Type "mytool" -> scancodes: m=0x32, y=0x15, t=0x14, o=0x18, o=0x18, l=0x26
        for &code in &[0x32u8, 0x15, 0x14, 0x18, 0x18, 0x26] {
            assert_eq!(ctrl.handle_key(code, 1, false, false), KeyOutcome::Redraw);
        }
        assert_eq!(ctrl.state().query.as_str(), "mytool");
        let (_, count) = ctrl.filtered();
        assert_eq!(count, 0);

        let expected_q = ctrl.state().query;
        assert_eq!(
            ctrl.handle_key(0x1C, 1, false, false),
            KeyOutcome::Launch(LaunchTarget::QueryProgram(expected_q))
        );
        assert!(!ctrl.is_open());

        // Arithmetic expressions with 0 catalog matches should NOT launch a program on Enter.
        ctrl.open();
        assert_eq!(ctrl.handle_key(0x03, 1, false, false), KeyOutcome::Redraw); // '2'
        assert_eq!(ctrl.handle_key(0x0D, 1, false, true), KeyOutcome::Redraw);  // '+'
        assert_eq!(ctrl.handle_key(0x03, 1, false, false), KeyOutcome::Redraw); // '2'
        assert_eq!(ctrl.handle_key(0x1C, 1, false, false), KeyOutcome::Consumed);
        assert!(ctrl.is_open());
    }

    #[test]
    fn canonical_aliases_and_keyword_rankings() {
        let alias_cases: &[(&str, &str)] = &[
            ("settings", "tatami"),
            ("Preferences", "tatami"),
            ("prefs", "tatami"),
            ("system-settings", "tatami"),
            ("store", "sexshop"),
            ("appstore", "sexshop"),
            ("app-store", "sexshop"),
            ("packages", "sexshop"),
            ("pkg", "sexshop"),
            ("files", "loom"),
            ("finder", "loom"),
            ("explorer", "loom"),
            ("browser", "kaleidoscope"),
            ("web", "kaleidoscope"),
            ("internet", "kaleidoscope"),
            ("monitor", "mesh"),
            ("activity", "mesh"),
            ("tasks", "mesh"),
            ("top", "mesh"),
            ("htop", "mesh"),
            ("calendar", "date"),
            ("schedule", "date"),
            ("music", "qupid"),
            ("audio", "qupid"),
            ("player", "qupid"),
            ("vlc", "qupid"),
            ("chat", "flirt"),
            ("messages", "flirt"),
            ("messaging", "flirt"),
            ("daw", "moan-studio"),
            ("moan", "moan-studio"),
            ("synth", "moan-studio"),
            ("notes", "jot"),
            ("notepad", "jot"),
            ("editor", "vellum"),
            ("textedit", "vellum"),
            ("calc", "abacus"),
            ("calculator", "abacus"),
            ("ai", "tailor"),
            ("assistant", "tailor"),
            ("claude", "tailor"),
            ("codex", "tailor"),
            ("bridge", "tether"),
            ("remote", "tether"),
            ("mcp", "tether"),
            ("torrent", "swingers"),
            ("p2p", "swingers"),
            ("webcam", "flash"),
            ("camera", "flash"),
            ("tatami", "tatami"),
            ("custom-tool", "custom-tool"),
        ];
        for &(alias, expected) in alias_cases {
            assert_eq!(resolve_program_alias(alias), expected, "alias={}", alias);
        }

        let top_tile_cases: &[(&str, &str)] = &[
            ("store", "sexshop"),
            ("preferences", "settings"),
            ("calculator", "abacus"),
            ("music", "qupid"),
            ("wifi", "settings"),
            ("bluetooth", "hookup"),
            ("firewall", "corset"),
            ("backup", "aftercare"),
            ("daw", "moan"),
            ("moan", "moan"),
        ];
        for &(query, expected_id) in top_tile_cases {
            let (order, n) = filter_catalog(Category::All, query);
            assert!(n >= 1, "expected match for {}", query);
            assert_eq!(CATALOG[order[0]].id, expected_id, "query={}", query);
        }
    }
}
