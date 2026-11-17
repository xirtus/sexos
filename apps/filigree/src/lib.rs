// SPDX-License-Identifier: GPL-3.0-only
//! Filigree: the SexOS font manager, specimen comparator, and Spindle typography
//! bridge built on Weft (`OmaFont` parity).
//!
//! Pure `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, so host tests drive the
//! entire window directly against `weft::Ui` and `filigree_core`.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use filigree_core::{
    build_comparison, calculate_waterfall, codepoint_page, inspect_repertoire, nonempty_pages,
    spindle_font_config, spindle_font_config_entry, unify_families, validate_input,
    DuplicateGroup, DuplicateKind, FontCatalog, FontClass, FontEntry, InstallPreview,
    SmartFilter, SpindleHandoffError, UnifiedFamily, ValidatedFontAsset, ValidationError,
    DEFAULT_MANIFEST_PATH, DEFAULT_SETTINGS_PATH, DEFAULT_WATERFALL_SIZES, MAX_COMPARE_SLOTS,
};
use weft::menu::{Command, Menu};
use weft::widget::{Column, Id, Layout, Rows, Tab, TextStyle};
use weft::{ButtonStyle, Coach, CoachControls, CoachStep, Response, TabEvent, Ui};

/// Default editable pangram shown in the Families & Waterfall tab.
pub const DEFAULT_SAMPLE_TEXT: &str = "Pack my box with five dozen liquor jugs";

/// Embedded SexOS system font assets seeded into Filigree on startup.
pub const EMBEDDED_FONTS: &[(&str, &[u8])] = &[
    ("/system/fonts/Geist-Regular.ttf", sex_font::GEIST_SANS),
    ("/system/fonts/Geist-Bold.ttf", sex_font::GEIST_SANS_BOLD),
    ("/system/fonts/Geist-Italic.ttf", sex_font::GEIST_SANS_ITALIC),
    ("/system/fonts/Geist-BoldItalic.ttf", sex_font::GEIST_SANS_BOLD_ITALIC),
    ("/system/fonts/GeistMono-Regular.ttf", sex_font::GEIST_MONO),
    ("/system/fonts/GeistMono-Bold.ttf", sex_font::GEIST_MONO_BOLD),
    ("/system/fonts/GeistMono-Italic.ttf", sex_font::GEIST_MONO_ITALIC),
    ("/system/fonts/GeistMono-BoldItalic.ttf", sex_font::GEIST_MONO_BOLD_ITALIC),
    ("/system/fonts/JetBrainsMono-Regular.ttf", sex_font::JETBRAINS_MONO),
    ("/system/fonts/JetBrainsMono-Bold.ttf", sex_font::JETBRAINS_MONO_BOLD),
    ("/system/fonts/JetBrainsMono-Italic.ttf", sex_font::JETBRAINS_MONO_ITALIC),
    ("/system/fonts/JetBrainsMono-BoldItalic.ttf", sex_font::JETBRAINS_MONO_BOLD_ITALIC),
    ("/system/fonts/NotoSansMono-Regular.ttf", sex_font::fallback::NOTO_MONO),
    ("/system/fonts/NotoSansArabic-Regular.ttf", sex_font::fallback::NOTO_ARABIC),
    ("/system/fonts/NotoSansHebrew-Regular.ttf", sex_font::fallback::NOTO_HEBREW),
    ("/system/fonts/NotoSansThai-Regular.ttf", sex_font::fallback::NOTO_THAI),
];

/// Build a [`FontCatalog`] and path->bytes map seeded with SexOS's embedded fonts.
pub fn seeded_catalog() -> (FontCatalog, BTreeMap<String, &'static [u8]>) {
    let mut catalog = FontCatalog::new();
    let mut blobs = BTreeMap::new();
    for &(path, bytes) in EMBEDDED_FONTS {
        if catalog.add_from_bytes(path, 0, bytes).is_some() {
            blobs.insert(String::from(path), bytes);
        }
    }
    (catalog, blobs)
}

/// Top-level tabs in Filigree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FiligreeTab {
    Families = 0,
    Compare = 1,
    Repertoire = 2,
    Duplicates = 3,
}

impl FiligreeTab {
    pub fn from_index(idx: usize) -> Self {
        match idx {
            1 => FiligreeTab::Compare,
            2 => FiligreeTab::Repertoire,
            3 => FiligreeTab::Duplicates,
            _ => FiligreeTab::Families,
        }
    }
}

/// Smart collection filter chips in the Families & Waterfall tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartCollection {
    All,
    Monospace,
    Sans,
    Serif,
    Starred,
    Deactivated,
}

impl SmartCollection {
    pub const ALL: [SmartCollection; 6] = [
        SmartCollection::All,
        SmartCollection::Monospace,
        SmartCollection::Sans,
        SmartCollection::Serif,
        SmartCollection::Starred,
        SmartCollection::Deactivated,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            SmartCollection::All => "All",
            SmartCollection::Monospace => "Monospace",
            SmartCollection::Sans => "Sans",
            SmartCollection::Serif => "Serif",
            SmartCollection::Starred => "Starred",
            SmartCollection::Deactivated => "Deactivated",
        }
    }

    fn to_smart_filter(self) -> SmartFilter {
        let mut f = SmartFilter::new(self.label());
        match self {
            SmartCollection::All => {}
            SmartCollection::Monospace => f.monospace_only = true,
            SmartCollection::Sans => f.classification = Some(FontClass::Sans),
            SmartCollection::Serif => f.classification = Some(FontClass::Serif),
            SmartCollection::Starred => f.starred_only = true,
            SmartCollection::Deactivated => f.deactivated_only = Some(true),
        }
        f
    }
}

/// Widget handles for one side-by-side comparison slot (up to 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompareSlotUi {
    pub card: Id,
    pub title: Id,
    pub sample: Id,
    pub metrics: Id,
    pub class_glyphs: Id,
    pub remove_btn: Id,
}

/// Full retained Weft interface state for Filigree.
pub struct Filigree {
    pub catalog: FontCatalog,
    pub blobs: BTreeMap<String, &'static [u8]>,
    pub settings_toml: String,
    pub active_tab: FiligreeTab,
    pub filter: SmartCollection,
    pub search_query: String,
    pub sample_text: String,
    pub selected_family: Option<String>,
    pub selected_face_idx: usize,
    pub repertoire_page_pos: usize,
    pub compare_families: Vec<String>,
    pub last_spindle_toml: Option<String>,
    pub last_install_preview: Option<InstallPreview>,

    pub coach: Coach,
    pub coach_controls: CoachControls,
    pub tabs: Id,
    pub summary_label: Id,
    pub guide_btn: Id,

    // Tab 1: Families & Waterfall
    pub search_field: Id,
    pub filter_buttons: [(Id, SmartCollection); 6],
    pub sample_field: Id,
    pub reset_sample_btn: Id,
    pub family_list: Id,
    pub selected_title: Id,
    pub selected_meta: Id,
    pub faces_summary: Id,
    pub star_btn: Id,
    pub toggle_active_btn: Id,
    pub add_compare_btn: Id,
    pub waterfall_header: Id,
    pub waterfall_rows: Vec<(u32, Id)>,
    pub manifest_preview: Id,

    // Tab 2: Compare (up to 6 side-by-side)
    pub compare_summary: Id,
    pub compare_empty: Id,
    pub compare_add_btn: Id,
    pub compare_fill_btn: Id,
    pub compare_clear_btn: Id,
    pub compare_grid: Id,
    pub compare_slots: Vec<CompareSlotUi>,

    // Tab 3: Repertoire & Inspector
    pub inspector_title: Id,
    pub inspector_prev_face_btn: Id,
    pub inspector_next_face_btn: Id,
    pub inspector_identity: Id,
    pub inspector_credits: Id,
    pub inspector_metrics: Id,
    pub inspector_sha: Id,
    pub inspector_scripts: Id,
    pub inspector_ranges: Id,
    pub inspector_page_label: Id,
    pub inspector_prev_page_btn: Id,
    pub inspector_next_page_btn: Id,
    pub inspector_codepoints: Id,

    // Tab 4: Duplicates & Install Check
    pub spindle_target_label: Id,
    pub spindle_mono_buttons: Vec<(Id, String)>,
    pub spindle_handoff_btn: Id,
    pub spindle_status: Id,
    pub spindle_preview: Id,
    pub dupes_summary: Id,
    pub dupes_recheck_btn: Id,
    pub install_validate_btn: Id,
    pub install_status: Id,
}

fn walkthrough_steps() -> Vec<CoachStep> {
    vec![
        CoachStep::new(
            "catalog",
            "Browse & Filter Font Families",
            "Use the Search box and Smart Collection buttons (All, Monospace, Sans, Serif, Starred, Deactivated) to filter SexOS fonts by style, rating, or activation state.",
            "Search & Smart Collections",
        ),
        CoachStep::new(
            "waterfall",
            "Live Waterfall & Non-Destructive Toggle",
            "Edit the Sample Text field to preview pangrams across pixel sizes, star your favorite families, and toggle Activate / Deactivate to update /system/fonts/active.toml without deleting files.",
            "Sample Text & Active Manifest",
        ),
        CoachStep::new(
            "compare",
            "Compare Up to 6 Families Side-by-Side",
            "Open the Compare tab to line up up to 6 families on a shared baseline and inspect x-height, cap-height, PANOSE classification, and glyph counts.",
            "Compare Tab",
        ),
        CoachStep::new(
            "repertoire",
            "Repertoire & Font Inspector",
            "Open Repertoire & Inspector to examine foundry, designer, license, version, OS/2 weight, Unicode script coverage, and paged ScalarRange codepoints.",
            "Repertoire & Inspector Tab",
        ),
        CoachStep::new(
            "spindle",
            "Duplicates & 1-Click Spindle Handoff",
            "Check SHA-256 and version-shadowed duplicates, run pre-install bundle validation, and click Hand Monospace to Spindle to stage /system/settings.toml.",
            "Duplicates & Install Check Tab",
        ),
    ]
}

impl Filigree {
    /// Builds the complete Filigree UI tree and seeds it with SexOS's embedded fonts.
    pub fn build(ui: &mut Ui) -> Self {
        Self::register_commands(ui);

        let root = ui.root();
        ui.set_layout(root, Layout::column());
        Self::build_menubar(ui, root);

        let toolbar = ui.group(
            root,
            Layout::row()
                .gap(8.0)
                .padding_xy(12.0, 6.0)
                .fill_width()
                .shrink(0.0),
        );
        let title = ui.text(toolbar, "Filigree", TextStyle::Heading);
        ui.set_color(title, Some(ui.theme().palette.accent));
        let summary_label = ui.text(toolbar, "", TextStyle::Caption);
        ui.set_layout(summary_label, Layout::default().grow(1.0));
        let guide_btn = ui.icon_button(toolbar, "question", "Guide (F1)");
        ui.set_tooltip(guide_btn, "Toggle interactive F1 walkthrough");

        let coach = Coach::new(walkthrough_steps());
        let coach_host = ui.group(
            root,
            Layout::column().padding_xy(12.0, 4.0).fill_width().shrink(0.0),
        );
        let coach_controls = coach.render(ui, coach_host);

        let tabs = ui.tabs(root);
        ui.set_layout(tabs, Layout::default().fill());

        let page_families = ui.tab_page(tabs, Tab::new("Families & Waterfall").icon("font"));
        let page_compare = ui.tab_page(tabs, Tab::new("Compare").icon("split"));
        let page_repertoire = ui.tab_page(tabs, Tab::new("Repertoire & Inspector").icon("info"));
        let page_duplicates = ui.tab_page(tabs, Tab::new("Duplicates & Install Check").icon("check"));

        // ---- Tab 1: Families & Waterfall ----------------------------------------
        let filter_row = ui.group(
            page_families,
            Layout::row().gap(6.0).wrap().fill_width().shrink(0.0),
        );
        let search_field = ui.field(filter_row, "", "Search font families...");
        ui.set_layout(search_field, Layout::default().min_width(180.0).grow(1.0));
        ui.set_label(search_field, "Search families");

        let mut filter_idx = 0usize;
        let filter_buttons = SmartCollection::ALL.map(|sc| {
            let b = ui.button(filter_row, sc.label());
            if filter_idx == 0 {
                ui.set_button_style(b, ButtonStyle::Primary);
            }
            filter_idx += 1;
            (b, sc)
        });

        let sample_row = ui.group(
            page_families,
            Layout::row().gap(8.0).fill_width().shrink(0.0),
        );
        ui.label(sample_row, "Sample:");
        let sample_field = ui.field(
            sample_row,
            DEFAULT_SAMPLE_TEXT,
            "Type specimen sample text...",
        );
        ui.set_layout(sample_field, Layout::default().grow(1.0));
        ui.set_label(sample_field, "Sample text");
        let reset_sample_btn = ui.button(sample_row, "Reset");

        let split = ui.split(page_families, false, 0.36);
        ui.set_layout(split, Layout::default().fill());

        let left_pane = ui.group(split, Layout::column().gap(6.0).fill());
        let family_list = ui.list_with(
            left_pane,
            Box::new(Rows::default()),
            vec![
                Column::new("Family", 130.0),
                Column::new("Class", 78.0),
                Column::new("Faces", 46.0).right(),
                Column::new("State", 76.0),
            ],
        );
        ui.set_layout(family_list, Layout::default().fill());
        ui.set_label(family_list, "Font families");

        let right_scroll = ui.scroll(split);
        ui.set_layout(right_scroll, Layout::column().gap(8.0).fill());

        let detail_card = ui.card(right_scroll, Layout::column().gap(6.0).fill_width().shrink(0.0));
        let selected_title = ui.text(detail_card, "No family selected", TextStyle::Title);
        let selected_meta = ui.text(detail_card, "", TextStyle::Caption);
        ui.set_layout(selected_meta, Layout::default().fill_width());
        let faces_summary = ui.paragraph(detail_card, "");
        ui.set_layout(faces_summary, Layout::default().fill_width());

        let actions_row = ui.group(detail_card, Layout::row().gap(6.0).wrap().fill_width());
        let star_btn = ui.icon_button(actions_row, "star", "Star (0/5)");
        let toggle_active_btn = ui.button(actions_row, "Deactivate");
        let add_compare_btn = ui.button(actions_row, "+ Compare");

        let waterfall_card = ui.card(right_scroll, Layout::column().gap(4.0).fill_width().shrink(0.0));
        let waterfall_header = ui.text(waterfall_card, "Waterfall Specimen", TextStyle::Heading);
        let mut waterfall_rows = Vec::with_capacity(DEFAULT_WATERFALL_SIZES.len());
        for &px in DEFAULT_WATERFALL_SIZES {
            let row_id = ui.paragraph(waterfall_card, &format!("{px}px"));
            ui.set_layout(row_id, Layout::default().fill_width());
            waterfall_rows.push((px, row_id));
        }

        let manifest_card = ui.card(right_scroll, Layout::column().gap(4.0).fill_width().shrink(0.0));
        ui.text(
            manifest_card,
            "Active Manifest (/system/fonts/active.toml)",
            TextStyle::Heading,
        );
        let manifest_preview = ui.text(manifest_card, "", TextStyle::Mono);
        ui.set_layout(manifest_preview, Layout::default().fill_width());

        // ---- Tab 2: Compare (up to 6 side-by-side) ------------------------------
        let compare_bar = ui.group(
            page_compare,
            Layout::row().gap(8.0).wrap().fill_width().shrink(0.0),
        );
        let compare_summary = ui.text(compare_bar, "", TextStyle::Heading);
        ui.set_layout(compare_summary, Layout::default().grow(1.0));
        let compare_add_btn = ui.button(compare_bar, "Add Selected Family");
        let compare_fill_btn = ui.button(compare_bar, "Fill Up to 6");
        let compare_clear_btn = ui.button(compare_bar, "Clear");

        let compare_scroll = ui.scroll(page_compare);
        ui.set_layout(compare_scroll, Layout::column().gap(8.0).fill());
        let compare_empty = ui.paragraph(
            compare_scroll,
            "No families in comparison. Select a family and click + Compare or Add Selected Family.",
        );
        let compare_grid = ui.group(
            compare_scroll,
            Layout::grid(3).gap(8.0).fill_width().shrink(0.0),
        );
        let compare_slots = Vec::new();

        // ---- Tab 3: Repertoire & Inspector --------------------------------------
        let rep_scroll = ui.scroll(page_repertoire);
        ui.set_layout(rep_scroll, Layout::column().gap(8.0).fill());

        let rep_head = ui.group(rep_scroll, Layout::row().gap(8.0).fill_width().shrink(0.0));
        let inspector_title = ui.text(rep_head, "Font Inspector", TextStyle::Title);
        ui.set_layout(inspector_title, Layout::default().grow(1.0));
        let inspector_prev_face_btn = ui.button(rep_head, "Prev Face");
        let inspector_next_face_btn = ui.button(rep_head, "Next Face");

        let meta_card = ui.card(rep_scroll, Layout::column().gap(6.0).fill_width().shrink(0.0));
        ui.text(meta_card, "OpenType / TrueType Metadata", TextStyle::Heading);
        let inspector_identity = ui.paragraph(meta_card, "");
        ui.set_layout(inspector_identity, Layout::default().fill_width());
        let inspector_credits = ui.paragraph(meta_card, "");
        ui.set_layout(inspector_credits, Layout::default().fill_width());
        let inspector_metrics = ui.paragraph(meta_card, "");
        ui.set_layout(inspector_metrics, Layout::default().fill_width());
        let inspector_sha = ui.text(meta_card, "", TextStyle::Mono);
        ui.set_layout(inspector_sha, Layout::default().fill_width());

        let rep_card = ui.card(rep_scroll, Layout::column().gap(6.0).fill_width().shrink(0.0));
        ui.text(
            rep_card,
            "Unicode Coverage & ScalarRange Repertoire",
            TextStyle::Heading,
        );
        let inspector_scripts = ui.paragraph(rep_card, "");
        ui.set_layout(inspector_scripts, Layout::default().fill_width());
        let inspector_ranges = ui.paragraph(rep_card, "");
        ui.set_layout(inspector_ranges, Layout::default().fill_width());

        let page_row = ui.group(rep_card, Layout::row().gap(8.0).fill_width().shrink(0.0));
        let inspector_prev_page_btn = ui.button(page_row, "Prev Block");
        let inspector_page_label = ui.text(page_row, "", TextStyle::Caption);
        ui.set_layout(inspector_page_label, Layout::default().grow(1.0));
        let inspector_next_page_btn = ui.button(page_row, "Next Block");

        let inspector_codepoints = ui.text(rep_card, "", TextStyle::Mono);
        ui.set_layout(inspector_codepoints, Layout::default().fill_width());

        // ---- Tab 4: Duplicates & Install Check ----------------------------------
        let maint_scroll = ui.scroll(page_duplicates);
        ui.set_layout(maint_scroll, Layout::column().gap(8.0).fill());

        let spindle_card = ui.card(maint_scroll, Layout::column().gap(6.0).fill_width().shrink(0.0));
        ui.text(
            spindle_card,
            "Spindle Terminal Monospace Handoff (/system/settings.toml)",
            TextStyle::Heading,
        );
        let spindle_target_label = ui.paragraph(spindle_card, "");
        ui.set_layout(spindle_target_label, Layout::default().fill_width());

        let mono_pick_row = ui.group(spindle_card, Layout::row().gap(6.0).wrap().fill_width());
        let mut spindle_mono_buttons = Vec::new();
        for family in ["Geist Mono", "JetBrains Mono", "Noto Sans Mono"] {
            let b = ui.button(mono_pick_row, &format!("Use {family}"));
            spindle_mono_buttons.push((b, String::from(family)));
        }
        let spindle_handoff_btn = ui.icon_button(
            spindle_card,
            "terminal",
            "Hand Monospace to Spindle",
        );
        ui.set_button_style(spindle_handoff_btn, ButtonStyle::Primary);
        let spindle_status = ui.paragraph(spindle_card, "");
        ui.set_layout(spindle_status, Layout::default().fill_width());
        let spindle_preview = ui.text(spindle_card, "", TextStyle::Mono);
        ui.set_layout(spindle_preview, Layout::default().fill_width());

        let dupes_card = ui.card(maint_scroll, Layout::column().gap(6.0).fill_width().shrink(0.0));
        let dupes_head = ui.group(dupes_card, Layout::row().gap(8.0).fill_width());
        let dupes_h = ui.text(dupes_head, "Duplicates & Tidy-Up Plan", TextStyle::Heading);
        ui.set_layout(dupes_h, Layout::default().grow(1.0));
        let dupes_recheck_btn = ui.button(dupes_head, "Recheck Duplicates");
        let dupes_summary = ui.paragraph(dupes_card, "");
        ui.set_layout(dupes_summary, Layout::default().fill_width());

        let install_card = ui.card(maint_scroll, Layout::column().gap(6.0).fill_width().shrink(0.0));
        let install_head = ui.group(install_card, Layout::row().gap(8.0).fill_width());
        let install_h = ui.text(install_head, "Pre-Install Font & Bundle Validator", TextStyle::Heading);
        ui.set_layout(install_h, Layout::default().grow(1.0));
        let install_validate_btn = ui.button(install_head, "Validate Selected Font");
        let install_status = ui.paragraph(install_card, "");
        ui.set_layout(install_status, Layout::default().fill_width());

        let (catalog, blobs) = seeded_catalog();
        let initial_family = catalog.families().first().map(|f| f.name.clone());

        let mut app = Self {
            catalog,
            blobs,
            settings_toml: String::from("# SexOS system settings\n[appearance]\naccent = \"#D6336C\"\n"),
            active_tab: FiligreeTab::Families,
            filter: SmartCollection::All,
            search_query: String::new(),
            sample_text: String::from(DEFAULT_SAMPLE_TEXT),
            selected_family: initial_family,
            selected_face_idx: 0,
            repertoire_page_pos: 0,
            compare_families: vec![
                String::from("Geist"),
                String::from("Geist Mono"),
                String::from("JetBrains Mono"),
            ],
            last_spindle_toml: None,
            last_install_preview: None,

            coach,
            coach_controls,
            tabs,
            summary_label,
            guide_btn,

            search_field,
            filter_buttons,
            sample_field,
            reset_sample_btn,
            family_list,
            selected_title,
            selected_meta,
            faces_summary,
            star_btn,
            toggle_active_btn,
            add_compare_btn,
            waterfall_header,
            waterfall_rows,
            manifest_preview,

            compare_summary,
            compare_empty,
            compare_add_btn,
            compare_fill_btn,
            compare_clear_btn,
            compare_grid,
            compare_slots,

            inspector_title,
            inspector_prev_face_btn,
            inspector_next_face_btn,
            inspector_identity,
            inspector_credits,
            inspector_metrics,
            inspector_sha,
            inspector_scripts,
            inspector_ranges,
            inspector_page_label,
            inspector_prev_page_btn,
            inspector_next_page_btn,
            inspector_codepoints,

            spindle_target_label,
            spindle_mono_buttons,
            spindle_handoff_btn,
            spindle_status,
            spindle_preview,
            dupes_summary,
            dupes_recheck_btn,
            install_validate_btn,
            install_status,
        };

        app.refresh(ui);
        app
    }

    fn register_commands(ui: &mut Ui) {
        for c in [
            Command::new("filigree.coach", "Walkthrough Guide")
                .shortcut("F1")
                .icon("question")
                .keywords("help coach walkthrough guide f1"),
            Command::new("filigree.tab.families", "Families & Waterfall")
                .shortcut("Ctrl+1")
                .keywords("families waterfall"),
            Command::new("filigree.tab.compare", "Compare Side-by-Side")
                .shortcut("Ctrl+2")
                .keywords("compare metrics"),
            Command::new("filigree.tab.repertoire", "Repertoire & Inspector")
                .shortcut("Ctrl+3")
                .keywords("repertoire unicode inspector"),
            Command::new("filigree.tab.duplicates", "Duplicates & Install Check")
                .shortcut("Ctrl+4")
                .keywords("duplicates install spindle"),
            Command::new("filigree.toggle-active", "Activate / Deactivate Family")
                .shortcut("Ctrl+E")
                .keywords("activate deactivate manifest"),
            Command::new("filigree.star-up", "Cycle Family Star Rating")
                .shortcut("Ctrl+B")
                .icon("star")
                .keywords("star rating favorite"),
            Command::new("filigree.compare-add", "Add Selected Family to Compare")
                .shortcut("Ctrl+M")
                .keywords("compare"),
            Command::new("filigree.spindle-handoff", "Hand Monospace to Spindle")
                .shortcut("Ctrl+H")
                .icon("terminal")
                .keywords("spindle monospace handoff settings"),
            Command::new("filigree.reset-sample", "Reset Sample Text")
                .keywords("sample pangram reset"),
            Command::new("filigree.about", "About Filigree").icon("info"),
        ] {
            ui.commands.add(c);
        }
    }

    fn build_menubar(ui: &mut Ui, root: Id) {
        ui.menubar(
            root,
            vec![
                Menu::new("Catalog")
                    .command("filigree.toggle-active")
                    .command("filigree.star-up")
                    .command("filigree.compare-add")
                    .separator()
                    .command("filigree.reset-sample"),
                Menu::new("View")
                    .command("filigree.tab.families")
                    .command("filigree.tab.compare")
                    .command("filigree.tab.repertoire")
                    .command("filigree.tab.duplicates")
                    .separator()
                    .command("weft.palette"),
                Menu::new("Spindle").command("filigree.spindle-handoff"),
                Menu::new("Help")
                    .command("filigree.coach")
                    .command("filigree.about"),
            ],
        );
    }

    /// Returns the unified families matching the active [`SmartCollection`] and search query.
    pub fn filtered_families(&self) -> Vec<UnifiedFamily> {
        let smart = self.filter.to_smart_filter();
        let q = self.search_query.trim().to_ascii_lowercase();
        let matching_entries: Vec<FontEntry> = self
            .catalog
            .eval_smart(&smart)
            .into_iter()
            .filter(|e| {
                if q.is_empty() {
                    return true;
                }
                e.canonical_family().to_ascii_lowercase().contains(&q)
                    || e.metadata.family.to_ascii_lowercase().contains(&q)
                    || e.tags.iter().any(|t| t.to_ascii_lowercase().contains(&q))
            })
            .cloned()
            .collect();
        unify_families(&matching_entries)
    }

    /// Returns the currently selected unified family (falling back to the first filtered or catalog family).
    pub fn current_family(&self) -> Option<UnifiedFamily> {
        let families = self.filtered_families();
        if let Some(name) = &self.selected_family {
            if let Some(found) = families
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
            {
                return Some(found.clone());
            }
            if let Some(found) = self
                .catalog
                .families()
                .into_iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
            {
                return Some(found);
            }
        }
        families.into_iter().next()
    }

    fn current_face_in_family<'a>(&self, fam: &'a UnifiedFamily) -> Option<&'a FontEntry> {
        if fam.faces.is_empty() {
            return None;
        }
        fam.faces
            .get(self.selected_face_idx.min(fam.faces.len() - 1))
            .or_else(|| fam.primary_face())
    }

    /// Selects a family by canonical name (case-insensitive) and updates all views.
    pub fn select_family(&mut self, ui: &mut Ui, family_name: &str) {
        self.selected_family = Some(String::from(family_name));
        self.selected_face_idx = 0;
        self.repertoire_page_pos = 0;
        self.refresh(ui);
    }

    /// Switches the active tab and syncs the Weft tab strip.
    pub fn set_tab(&mut self, ui: &mut Ui, tab: FiligreeTab) {
        self.active_tab = tab;
        ui.set_active_tab(self.tabs, tab as usize);
        self.refresh_active_views(ui);
    }

    /// Sets the active Smart Collection filter and refreshes the catalog view.
    pub fn set_filter(&mut self, ui: &mut Ui, filter: SmartCollection) {
        self.filter = filter;
        self.refresh(ui);
    }

    /// Updates the live sample text field and refreshes Waterfall + Compare views.
    pub fn set_sample_text(&mut self, ui: &mut Ui, text: &str) {
        self.sample_text = String::from(text);
        if ui.text_of(self.sample_field) != text {
            ui.set_text(self.sample_field, text);
        }
        self.refresh_waterfall(ui);
        self.refresh_compare(ui);
    }

    /// Toggles non-destructive activation for the currently selected family.
    pub fn toggle_selected_active(&mut self, ui: &mut Ui) {
        let Some(fam) = self.current_family() else {
            return;
        };
        let was_active = self
            .catalog
            .entries
            .iter()
            .filter(|e| e.canonical_family().eq_ignore_ascii_case(&fam.name))
            .any(|e| e.active);
        if was_active {
            self.catalog.manifest.deactivate_family(&fam.name);
            ui.toast(
                &format!("Deactivated '{}' in {}", fam.name, DEFAULT_MANIFEST_PATH),
                None,
                3000,
            );
        } else {
            self.catalog.manifest.reactivate_family(&fam.name);
            ui.toast(&format!("Activated '{}'", fam.name), None, 3000);
        }
        self.catalog.apply_manifest();
        self.refresh(ui);
    }

    /// Cycles star rating (0..=5) on all faces of the currently selected family.
    pub fn cycle_selected_stars(&mut self, ui: &mut Ui) {
        let Some(fam) = self.current_family() else {
            return;
        };
        let cur = fam.primary_face().map(|f| f.stars).unwrap_or(0);
        let next = (cur + 1) % 6;
        for entry in &mut self.catalog.entries {
            if entry.canonical_family().eq_ignore_ascii_case(&fam.name) {
                entry.set_stars(next);
            }
        }
        self.refresh(ui);
    }

    /// Adds `family` to the side-by-side Compare board (up to 6 families).
    pub fn add_to_compare(&mut self, ui: &mut Ui, family: &str) -> bool {
        if self
            .compare_families
            .iter()
            .any(|f| f.eq_ignore_ascii_case(family))
        {
            ui.toast(&format!("'{}' is already in Compare", family), None, 2000);
            return false;
        }
        if self.compare_families.len() >= MAX_COMPARE_SLOTS {
            ui.toast(
                &format!("Compare supports up to {MAX_COMPARE_SLOTS} families"),
                None,
                2500,
            );
            return false;
        }
        self.compare_families.push(String::from(family));
        self.refresh(ui);
        true
    }

    /// Removes `family` from the Compare board.
    pub fn remove_from_compare(&mut self, ui: &mut Ui, family: &str) -> bool {
        let before = self.compare_families.len();
        self.compare_families
            .retain(|f| !f.eq_ignore_ascii_case(family));
        let removed = self.compare_families.len() != before;
        if removed {
            self.refresh(ui);
        }
        removed
    }

    /// Runs Spindle terminal monospace handoff on the currently selected face and stages `/system/settings.toml`.
    pub fn hand_off_spindle(&mut self, ui: &mut Ui) -> Result<String, SpindleHandoffError> {
        let Some(fam) = self.current_family() else {
            return Err(SpindleHandoffError::InvalidFont);
        };
        let Some(entry) = fam.primary_face().cloned() else {
            return Err(SpindleHandoffError::InvalidFont);
        };
        let cfg = match spindle_font_config_entry(&entry) {
            Ok(meta_cfg) => {
                if let Some(bytes) = self.blobs.get(&entry.path).copied() {
                    spindle_font_config(&entry, bytes)?
                } else {
                    meta_cfg
                }
            }
            Err(err) => {
                ui.set_text(
                    self.spindle_status,
                    &format!("Handoff blocked for '{}': {}", fam.name, err),
                );
                ui.toast(&format!("Spindle handoff: {err}"), None, 3500);
                return Err(err);
            }
        };

        let merged = cfg.apply_to_settings_toml(&self.settings_toml);
        self.settings_toml = merged.clone();
        self.last_spindle_toml = Some(merged.clone());
        ui.set_text(
            self.spindle_status,
            &format!(
                "Staged '{}' ({}) for Spindle in {}",
                cfg.family, cfg.path, DEFAULT_SETTINGS_PATH
            ),
        );
        ui.set_text(self.spindle_preview, &merged);
        ui.toast(
            &format!("Handed '{}' to Spindle ({})", cfg.family, DEFAULT_SETTINGS_PATH),
            None,
            4000,
        );
        Ok(merged)
    }

    /// Validates raw font or zip bundle bytes and displays the install preview card.
    pub fn validate_install_bytes(
        &mut self,
        ui: &mut Ui,
        bytes: &[u8],
        source_name: &str,
    ) -> Result<Vec<ValidatedFontAsset>, ValidationError> {
        match validate_input(bytes, source_name) {
            Ok(assets) => {
                if let Some(first) = assets.first() {
                    self.last_install_preview = Some(first.preview.clone());
                }
                let mut msg = format!("Validated {} asset(s) from '{}':", assets.len(), source_name);
                for a in &assets {
                    let p = &a.preview;
                    msg.push_str(&format!(
                        "\n• {} [{}] {} bytes, {} face(s), sha256={}...",
                        p.source_name,
                        p.format.as_str(),
                        p.byte_len,
                        p.faces.len(),
                        &p.sha256_hex[..16]
                    ));
                }
                ui.set_text(self.install_status, &msg);
                Ok(assets)
            }
            Err(err) => {
                ui.set_text(
                    self.install_status,
                    &format!("Validation failed for '{}': {}", source_name, err),
                );
                Err(err)
            }
        }
    }

    /// Toggles the `F1` interactive walkthrough coach banner.
    pub fn toggle_coach(&mut self, ui: &mut Ui) {
        self.coach.toggle();
        self.coach.sync(ui, &self.coach_controls);
    }

    /// Handles one Weft [`Response`].
    pub fn on(&mut self, ui: &mut Ui, r: &Response) {
        if self.coach.on(ui, &self.coach_controls, r) {
            return;
        }

        match r {
            Response::Tab(id, TabEvent::Selected(idx)) if *id == self.tabs => {
                self.active_tab = FiligreeTab::from_index(*idx);
                self.refresh_active_views(ui);
            }
            Response::Changed(id) if *id == self.search_field => {
                self.search_query = String::from(ui.text_of(self.search_field));
                self.refresh_catalog_list(ui);
                self.refresh_detail(ui);
                self.refresh_waterfall(ui);
                self.refresh_inspector(ui);
            }
            Response::Changed(id) | Response::Submitted(id) if *id == self.sample_field => {
                self.sample_text = String::from(ui.text_of(self.sample_field));
                self.refresh_waterfall(ui);
                self.refresh_compare(ui);
            }
            Response::Selected(id) | Response::Activated(id, _) if *id == self.family_list => {
                if let Some(row) = ui.selected_row(self.family_list) {
                    let families = self.filtered_families();
                    if let Some(fam) = families.get(row) {
                        self.selected_family = Some(fam.name.clone());
                        self.selected_face_idx = 0;
                        self.repertoire_page_pos = 0;
                        self.refresh_detail(ui);
                        self.refresh_waterfall(ui);
                        self.refresh_inspector(ui);
                        self.refresh_maintenance(ui);
                    }
                }
            }
            Response::Clicked(id) => self.on_click(ui, *id),
            Response::Command(cmd) => self.on_command(ui, cmd),
            _ => {}
        }
    }

    fn on_click(&mut self, ui: &mut Ui, id: Id) {
        if id == self.guide_btn {
            self.toggle_coach(ui);
            return;
        }
        if let Some((_, sc)) = self.filter_buttons.iter().find(|(b, _)| *b == id) {
            self.set_filter(ui, *sc);
            return;
        }
        if id == self.reset_sample_btn {
            self.set_sample_text(ui, DEFAULT_SAMPLE_TEXT);
            return;
        }
        if id == self.star_btn {
            self.cycle_selected_stars(ui);
            return;
        }
        if id == self.toggle_active_btn {
            self.toggle_selected_active(ui);
            return;
        }
        if id == self.add_compare_btn || id == self.compare_add_btn {
            if let Some(fam) = self.current_family() {
                if self
                    .compare_families
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(&fam.name))
                    && id == self.add_compare_btn
                {
                    self.remove_from_compare(ui, &fam.name);
                } else {
                    self.add_to_compare(ui, &fam.name);
                }
            }
            return;
        }
        if id == self.compare_fill_btn {
            let names: Vec<String> = self.catalog.families().into_iter().map(|f| f.name).collect();
            for name in names {
                if self.compare_families.len() >= MAX_COMPARE_SLOTS {
                    break;
                }
                if !self
                    .compare_families
                    .iter()
                    .any(|f| f.eq_ignore_ascii_case(&name))
                {
                    self.compare_families.push(name);
                }
            }
            self.refresh(ui);
            return;
        }
        if id == self.compare_clear_btn {
            self.compare_families.clear();
            self.refresh(ui);
            return;
        }
        if let Some(idx) = self.compare_slots.iter().position(|s| s.remove_btn == id) {
            if let Some(name) = self.compare_families.get(idx).cloned() {
                self.remove_from_compare(ui, &name);
            }
            return;
        }
        if id == self.inspector_prev_face_btn {
            self.selected_face_idx = self.selected_face_idx.saturating_sub(1);
            self.repertoire_page_pos = 0;
            self.refresh_inspector(ui);
            return;
        }
        if id == self.inspector_next_face_btn {
            if let Some(fam) = self.current_family() {
                if self.selected_face_idx + 1 < fam.faces.len() {
                    self.selected_face_idx += 1;
                    self.repertoire_page_pos = 0;
                    self.refresh_inspector(ui);
                }
            }
            return;
        }
        if id == self.inspector_prev_page_btn {
            self.repertoire_page_pos = self.repertoire_page_pos.saturating_sub(1);
            self.refresh_inspector(ui);
            return;
        }
        if id == self.inspector_next_page_btn {
            self.repertoire_page_pos += 1;
            self.refresh_inspector(ui);
            return;
        }
        if let Some((_, family)) = self.spindle_mono_buttons.iter().find(|(b, _)| *b == id) {
            let fam = family.clone();
            self.select_family(ui, &fam);
            return;
        }
        if id == self.spindle_handoff_btn {
            let _ = self.hand_off_spindle(ui);
            return;
        }
        if id == self.dupes_recheck_btn {
            self.refresh_maintenance(ui);
            return;
        }
        if id == self.install_validate_btn {
            if let Some(fam) = self.current_family() {
                if let Some(entry) = fam.primary_face() {
                    if let Some(bytes) = self.blobs.get(&entry.path).copied() {
                        let path = entry.path.clone();
                        let _ = self.validate_install_bytes(ui, bytes, &path);
                    }
                }
            }
        }
    }

    fn on_command(&mut self, ui: &mut Ui, cmd: &str) {
        match cmd {
            "filigree.coach" => self.toggle_coach(ui),
            "filigree.tab.families" => self.set_tab(ui, FiligreeTab::Families),
            "filigree.tab.compare" => self.set_tab(ui, FiligreeTab::Compare),
            "filigree.tab.repertoire" => self.set_tab(ui, FiligreeTab::Repertoire),
            "filigree.tab.duplicates" => self.set_tab(ui, FiligreeTab::Duplicates),
            "filigree.toggle-active" => self.toggle_selected_active(ui),
            "filigree.star-up" => self.cycle_selected_stars(ui),
            "filigree.compare-add" => {
                if let Some(fam) = self.current_family() {
                    self.add_to_compare(ui, &fam.name);
                }
            }
            "filigree.spindle-handoff" => {
                let _ = self.hand_off_spindle(ui);
            }
            "filigree.reset-sample" => self.set_sample_text(ui, DEFAULT_SAMPLE_TEXT),
            "filigree.about" => {
                ui.message(
                    "Filigree",
                    "SexOS Font Manager, Specimen Comparator, and Spindle Typography Bridge on Weft.",
                    &["OK"],
                    0,
                    0,
                );
            }
            _ => {}
        }
    }

    /// Recomputes and updates every view from current catalog and selection state.
    pub fn refresh(&mut self, ui: &mut Ui) {
        let all_families = self.catalog.families();
        let active_faces = self.catalog.entries.iter().filter(|e| e.active).count();
        ui.set_text(
            self.summary_label,
            &format!(
                "{} families • {} faces ({} active)",
                all_families.len(),
                self.catalog.entries.len(),
                active_faces
            ),
        );

        for (id, sc) in self.filter_buttons {
            ui.set_button_style(
                id,
                if sc == self.filter {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }

        self.refresh_catalog_list(ui);
        self.refresh_active_views(ui);
    }

    fn refresh_active_views(&mut self, ui: &mut Ui) {
        self.refresh_detail(ui);
        self.refresh_waterfall(ui);
        self.refresh_compare(ui);
        self.refresh_inspector(ui);
        self.refresh_maintenance(ui);
    }

    fn refresh_catalog_list(&mut self, ui: &mut Ui) {
        let families = self.filtered_families();
        let rows: Vec<Vec<String>> = families
            .iter()
            .map(|f| {
                let state = if f.is_any_active() {
                    if f.active_count() == f.faces.len() {
                        String::from("Active")
                    } else {
                        format!("{}/{} active", f.active_count(), f.faces.len())
                    }
                } else {
                    String::from("Deactivated")
                };
                vec![
                    f.name.clone(),
                    String::from(f.classification().as_str()),
                    format!("{}", f.faces.len()),
                    state,
                ]
            })
            .collect();
        ui.set_rows(self.family_list, Rows(rows));

        let selected_row = self.selected_family.as_ref().and_then(|sel| {
            families
                .iter()
                .position(|f| f.name.eq_ignore_ascii_case(sel))
        });
        let row = selected_row.or_else(|| (!families.is_empty()).then_some(0));
        if let Some(r) = row {
            self.selected_family = Some(families[r].name.clone());
        } else {
            self.selected_family = None;
        }
        ui.select_row(self.family_list, row);
    }

    fn refresh_detail(&mut self, ui: &mut Ui) {
        let manifest_toml = self.catalog.manifest.to_toml();
        ui.set_text(self.manifest_preview, &manifest_toml);

        let Some(fam) = self.current_family() else {
            ui.set_text(self.selected_title, "No matching families");
            ui.set_text(self.selected_meta, "Adjust the search query or Smart Collection filter.");
            ui.set_text(self.faces_summary, "");
            ui.set_enabled(self.star_btn, false);
            ui.set_enabled(self.toggle_active_btn, false);
            ui.set_enabled(self.add_compare_btn, false);
            return;
        };

        ui.set_enabled(self.star_btn, true);
        ui.set_enabled(self.toggle_active_btn, true);
        ui.set_enabled(self.add_compare_btn, true);

        let any_active = fam.is_any_active();
        let stars = fam.primary_face().map(|f| f.stars).unwrap_or(0);
        let min_w = fam.faces.iter().map(|f| f.metadata.weight).min().unwrap_or(400);
        let max_w = fam.faces.iter().map(|f| f.metadata.weight).max().unwrap_or(400);

        ui.set_text(self.selected_title, &fam.name);
        ui.set_text(
            self.selected_meta,
            &format!(
                "Class: {}  •  {}/{} faces active  •  Weight {}..={}  •  ★ {}/5",
                fam.classification().as_str(),
                fam.active_count(),
                fam.faces.len(),
                min_w,
                max_w,
                stars
            ),
        );

        let face_list: Vec<String> = fam
            .faces
            .iter()
            .map(|f| {
                let slant = if f.metadata.is_italic {
                    " Italic"
                } else if f.metadata.is_oblique {
                    " Oblique"
                } else {
                    ""
                };
                format!("{} ({}{})", f.metadata.subfamily, f.metadata.weight, slant)
            })
            .collect();
        ui.set_text(
            self.faces_summary,
            &format!("Faces: {}", face_list.join(", ")),
        );

        ui.set_text(self.star_btn, &format!("Star ({stars}/5)"));
        ui.set_text(
            self.toggle_active_btn,
            if any_active { "Deactivate" } else { "Activate" },
        );
        ui.set_button_style(
            self.toggle_active_btn,
            if any_active {
                ButtonStyle::Normal
            } else {
                ButtonStyle::Primary
            },
        );
        let in_compare = self
            .compare_families
            .iter()
            .any(|f| f.eq_ignore_ascii_case(&fam.name));
        ui.set_text(
            self.add_compare_btn,
            if in_compare { "In Compare ✓" } else { "+ Compare" },
        );
    }

    fn refresh_waterfall(&mut self, ui: &mut Ui) {
        let sample = if self.sample_text.is_empty() {
            " "
        } else {
            self.sample_text.as_str()
        };
        let Some(fam) = self.current_family() else {
            ui.set_text(self.waterfall_header, "Waterfall Specimen");
            for &(px, row_id) in &self.waterfall_rows {
                ui.set_text(row_id, &format!("{px}px — (no family selected)"));
            }
            return;
        };
        let Some(primary) = fam.primary_face() else {
            return;
        };
        let Some(bytes) = self.blobs.get(&primary.path).copied() else {
            return;
        };

        if let Some(wf) = calculate_waterfall(bytes, primary.face_index, sample, DEFAULT_WATERFALL_SIZES, 4) {
            ui.set_text(
                self.waterfall_header,
                &format!(
                    "Waterfall Specimen — {} {} (max width {}px)",
                    wf.family, wf.subfamily, wf.max_width_px
                ),
            );
            for (row_data, &(_, row_id)) in wf.rows.iter().zip(&self.waterfall_rows) {
                ui.set_text(
                    row_id,
                    &format!(
                        "{}px (lh {}px, adv {}px): {}",
                        row_data.size_px,
                        row_data.line_height_px,
                        row_data.estimated_advance_px,
                        row_data.text
                    ),
                );
            }
        }
    }

    fn refresh_compare(&mut self, ui: &mut Ui) {
        let all_families = self.catalog.families();
        let mut resolved: Vec<(UnifiedFamily, &FontEntry, &'static [u8])> = Vec::new();
        for name in &self.compare_families {
            if let Some(fam) = all_families
                .iter()
                .find(|f| f.name.eq_ignore_ascii_case(name))
            {
                if let Some(primary) = fam.primary_face() {
                    if let Some(bytes) = self.blobs.get(&primary.path).copied() {
                        resolved.push((fam.clone(), primary, bytes));
                    }
                }
            }
        }

        let sample = if self.sample_text.is_empty() {
            DEFAULT_SAMPLE_TEXT
        } else {
            self.sample_text.as_str()
        };

        self.compare_slots.clear();
        ui.clear(self.compare_grid);

        if resolved.is_empty() {
            ui.set_text(
                self.compare_summary,
                &format!("Comparing 0 / {MAX_COMPARE_SLOTS} families"),
            );
            ui.set_visible(self.compare_empty, true);
            ui.set_layout(self.compare_empty, Layout::default().fill_width());
            return;
        }

        ui.set_visible(self.compare_empty, false);
        ui.set_layout(self.compare_empty, Layout::default().height(0.0).min_height(0.0));

        let face_refs: Vec<(&[u8], u32)> = resolved
            .iter()
            .map(|(_, entry, bytes)| (*bytes, entry.face_index))
            .collect();

        match build_comparison(&face_refs, sample, 48) {
            Ok(board) => {
                ui.set_text(
                    self.compare_summary,
                    &format!(
                        "Comparing {} / {} families at {}px (baseline y={}px, height={}px)",
                        board.slots.len(),
                        MAX_COMPARE_SLOTS,
                        board.size_px,
                        board.shared_baseline_y_px,
                        board.total_height_px
                    ),
                );

                for (i, slot) in board.slots.iter().enumerate() {
                    let (_, entry, _) = &resolved[i];
                    let meta = &entry.metadata;
                    let card = ui.card(
                        self.compare_grid,
                        Layout::column().gap(6.0).fill_width().shrink(0.0),
                    );
                    let title = ui.text(
                        card,
                        &format!("{} ({})", slot.family, slot.subfamily),
                        TextStyle::Heading,
                    );
                    ui.set_layout(title, Layout::default().fill_width());

                    let missing_note = if slot.metrics.missing_chars > 0 {
                        format!(" [{} missing]", slot.metrics.missing_chars)
                    } else {
                        String::new()
                    };
                    let sample_lbl =
                        ui.paragraph(card, &format!("{}{}", board.sample_word, missing_note));
                    ui.set_layout(sample_lbl, Layout::default().fill_width());

                    let metrics_lbl = ui.paragraph(
                        card,
                        &format!(
                            "x-height: {}u ({}px) • cap-height: {}u ({}px)\nasc: {}px • desc: {}px • adv: {}px",
                            slot.metrics.x_height,
                            slot.metrics.x_height_px,
                            slot.metrics.cap_height,
                            slot.metrics.cap_height_px,
                            slot.metrics.ascender_px,
                            slot.metrics.descender_px,
                            slot.metrics.sample_width_px
                        ),
                    );
                    ui.set_layout(metrics_lbl, Layout::default().fill_width());

                    let p = meta.panose;
                    let class_glyphs = ui.text(
                        card,
                        &format!(
                            "Class: {} • PANOSE {}.{}.{}.{} • Glyphs: {}",
                            meta.classification().as_str(),
                            p.family_type(),
                            p.serif_style(),
                            p.weight(),
                            p.proportion(),
                            meta.glyph_count
                        ),
                        TextStyle::Caption,
                    );
                    ui.set_layout(class_glyphs, Layout::default().fill_width());
                    let remove_btn = ui.button(card, "Remove");
                    self.compare_slots.push(CompareSlotUi {
                        card,
                        title,
                        sample: sample_lbl,
                        metrics: metrics_lbl,
                        class_glyphs,
                        remove_btn,
                    });
                }
            }
            Err(err) => {
                ui.set_text(self.compare_summary, &format!("Compare error: {err}"));
            }
        }
    }

    fn refresh_inspector(&mut self, ui: &mut Ui) {
        let Some(fam) = self.current_family() else {
            ui.set_text(self.inspector_title, "Font Inspector");
            ui.set_text(self.inspector_identity, "No font selected.");
            ui.set_text(self.inspector_credits, "");
            ui.set_text(self.inspector_metrics, "");
            ui.set_text(self.inspector_sha, "");
            ui.set_text(self.inspector_scripts, "");
            ui.set_text(self.inspector_ranges, "");
            ui.set_text(self.inspector_page_label, "");
            ui.set_text(self.inspector_codepoints, "");
            return;
        };

        let clamped_face = self.selected_face_idx.min(fam.faces.len().saturating_sub(1));
        self.selected_face_idx = clamped_face;
        let Some(entry) = self.current_face_in_family(&fam) else {
            return;
        };
        let m = &entry.metadata;

        ui.set_text(
            self.inspector_title,
            &format!(
                "{} — {} (face {}/{})",
                m.family,
                m.subfamily,
                clamped_face + 1,
                fam.faces.len()
            ),
        );
        ui.set_enabled(self.inspector_prev_face_btn, clamped_face > 0);
        ui.set_enabled(
            self.inspector_next_face_btn,
            clamped_face + 1 < fam.faces.len(),
        );

        ui.set_text(
            self.inspector_identity,
            &format!(
                "Family: {} (legacy: {})  •  Subfamily: {}  •  PS: {}  •  Version: {}",
                m.family,
                m.legacy_family,
                m.subfamily,
                m.postscript_name.as_deref().unwrap_or("—"),
                m.version.as_deref().unwrap_or("—")
            ),
        );
        ui.set_text(
            self.inspector_credits,
            &format!(
                "Foundry: {}  •  Designer: {}  •  License: {}",
                m.manufacturer.as_deref().unwrap_or("—"),
                m.designer.as_deref().unwrap_or("—"),
                m.license.as_deref().unwrap_or("—")
            ),
        );
        ui.set_text(
            self.inspector_metrics,
            &format!(
                "OS/2 Weight: {}  •  Width: {}  •  Class: {}  •  Monospace: {}  •  Variable: {}  •  Glyphs: {}  •  UPEM: {}  •  Asc/Desc: {}/{}  •  x-h/cap-h: {:?}/{:?}",
                m.weight,
                m.width_class,
                m.classification().as_str(),
                m.is_monospace,
                m.is_variable,
                m.glyph_count,
                m.units_per_em,
                m.ascender,
                m.descender,
                m.x_height,
                m.cap_height
            ),
        );
        ui.set_text(
            self.inspector_sha,
            &format!(
                "Path: {}#{}  •  Active: {}  •  Stars: {}/5  •  SHA-256: {}",
                entry.path, entry.face_index, entry.active, entry.stars, entry.sha256_hex
            ),
        );

        let scripts = m.coverage.script_names();
        let scripts_joined = if scripts.is_empty() {
            String::from("None detected")
        } else {
            scripts.join(", ")
        };

        if let Some(bytes) = self.blobs.get(&entry.path).copied() {
            if let Some(rep) = inspect_repertoire(bytes, entry.face_index) {
                let pages = nonempty_pages(&rep, 256);
                ui.set_text(
                    self.inspector_scripts,
                    &format!(
                        "Unicode Scripts ({}): {}  •  Supported scalars: {} across {} range(s) ({} blocks)",
                        scripts.len(),
                        scripts_joined,
                        rep.total_codepoints,
                        rep.ranges.len(),
                        pages.len()
                    ),
                );
                let mut range_parts: Vec<String> = rep
                    .ranges
                    .iter()
                    .take(16)
                    .map(|r| format!("U+{:04X}..=U+{:04X} ({})", r.start, r.end, r.len()))
                    .collect();
                if rep.ranges.len() > 16 {
                    range_parts.push(format!("… +{} more", rep.ranges.len() - 16));
                }
                ui.set_text(
                    self.inspector_ranges,
                    &format!("ScalarRanges: {}", range_parts.join(", ")),
                );

                if pages.is_empty() {
                    ui.set_text(self.inspector_page_label, "Block 0 / 0");
                    ui.set_text(self.inspector_codepoints, "");
                    ui.set_enabled(self.inspector_prev_page_btn, false);
                    ui.set_enabled(self.inspector_next_page_btn, false);
                } else {
                    let pos = self.repertoire_page_pos.min(pages.len() - 1);
                    self.repertoire_page_pos = pos;
                    let page_idx = pages[pos];
                    let page = codepoint_page(&rep, 256, page_idx);
                    ui.set_text(
                        self.inspector_page_label,
                        &format!(
                            "Block {} / {} (U+{:04X}..=U+{:04X}, {} glyphs)",
                            pos + 1,
                            pages.len(),
                            page.block_start,
                            page.block_start + 255,
                            page.codepoints.len()
                        ),
                    );
                    ui.set_enabled(self.inspector_prev_page_btn, pos > 0);
                    ui.set_enabled(self.inspector_next_page_btn, pos + 1 < pages.len());

                    let preview: Vec<String> = page
                        .codepoints
                        .iter()
                        .take(36)
                        .filter_map(|&cp| {
                            let ch = char::from_u32(cp)?;
                            if ch.is_control() {
                                Some(format!("U+{cp:04X}"))
                            } else {
                                Some(format!("U+{cp:04X} '{ch}'"))
                            }
                        })
                        .collect();
                    ui.set_text(self.inspector_codepoints, &preview.join("  "));
                }
                return;
            }
        }

        ui.set_text(
            self.inspector_scripts,
            &format!("Unicode Scripts ({}): {}", scripts.len(), scripts_joined),
        );
        ui.set_text(self.inspector_ranges, "ScalarRanges unavailable (font bytes not loaded).");
    }

    fn refresh_maintenance(&mut self, ui: &mut Ui) {
        if let Some(fam) = self.current_family() {
            let primary = fam.primary_face();
            let mono_ok = primary
                .map(|e| e.active && (e.metadata.is_monospace || e.metadata.panose.is_monospace()))
                .unwrap_or(false);
            let msg = if mono_ok {
                format!(
                    "Target: {} ({}) — active monospace, ready for 1-click Spindle handoff.",
                    fam.name,
                    primary.map(|e| e.path.as_str()).unwrap_or("")
                )
            } else {
                format!(
                    "Target: {} — not an active monospace face. Pick a monospace family below (or in Tab 1) before handing off.",
                    fam.name
                )
            };
            ui.set_text(self.spindle_target_label, &msg);
        }
        if let Some(staged) = &self.last_spindle_toml {
            ui.set_text(self.spindle_preview, staged);
        } else {
            ui.set_text(self.spindle_preview, &self.settings_toml);
        }

        let dupes = self.catalog.find_duplicates();
        ui.set_text(self.dupes_summary, &format_duplicates(&dupes, self.catalog.entries.len()));

        if self.last_install_preview.is_none() {
            if let Some(fam) = self.current_family() {
                if let Some(entry) = fam.primary_face() {
                    if let Some(bytes) = self.blobs.get(&entry.path).copied() {
                        let path = entry.path.clone();
                        let _ = self.validate_install_bytes(ui, bytes, &path);
                    }
                }
            }
        }
    }
}

fn format_duplicates(dupes: &[DuplicateGroup], total_faces: usize) -> String {
    if dupes.is_empty() {
        return format!(
            "Clean catalog: 0 duplicate clusters across {total_faces} inspected faces."
        );
    }
    let mut out = format!("Detected {} duplicate cluster(s):", dupes.len());
    for g in dupes {
        let kind = match g.kind {
            DuplicateKind::ExactSha256 => "Exact SHA-256",
            DuplicateKind::VersionShadowed => "Version-Shadowed",
        };
        out.push_str(&format!(
            "\n• [{kind}] {} ({}) — keeper: {} | shadowed: {}",
            g.family,
            g.subfamily,
            g.keeper_path,
            g.shadowed_paths.join(", ")
        ));
        for step in &g.tidy_plan {
            out.push_str(&format!(
                "\n    - {} {}#{} ({})",
                step.action.as_str(),
                step.path,
                step.face_index,
                step.reason
            ));
        }
    }
    out
}
