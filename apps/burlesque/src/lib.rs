// SPDX-License-Identifier: GPL-3.0-only
//! Burlesque: Weft Presentations App over `burlesque-core`.

#![cfg_attr(feature = "sexos", no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use burlesque_core::animation::{BuildEffect, BuildStep};
use burlesque_core::deck::{AspectRatio, Deck, Slide};
use burlesque_core::elements::{Paragraph, Shape, ShapeKind, SlideElement, StrokeStyle, TextBox, TextRun, Transition};
use burlesque_core::layout::SlideLayout;
use burlesque_core::pdf_export::export_pdf;
use burlesque_core::presenter::{render_presenter_view, PresenterSession};
use burlesque_core::rehearse::{render_rehearsal_hud, RehearsalSession, VideoTimeline};
use burlesque_core::render::{render_magic_move_frame, render_slide, render_slide_at_build, render_transition_frame};
use burlesque_core::theme::Theme;
use sex_canvas::Rect;

pub mod ui_backend;
pub use ui_backend as weft;

use weft::input::Event;
use weft::menu::{Command, Menu};
use weft::widget::{Id, Layout, Rows};
use weft::{Response, Ui};

pub struct Burlesque {
    pub deck: Deck,
    pub current_slide: usize,
    pub presenter_mode: bool,
    pub presenter_session: Option<PresenterSession>,
    pub rehearsal_session: Option<RehearsalSession>,
    pub active_build_index: usize,
    pub magic_move_preview: Option<f32>,

    // Widget IDs
    pub preview_id: Id,
    pub btn_add_slide: Id,
    pub btn_delete_slide: Id,
    pub btn_prev: Id,
    pub btn_next: Id,
    pub btn_layout_title: Id,
    pub btn_layout_body: Id,
    pub btn_layout_twocol: Id,
    pub btn_theme: Id,
    pub btn_transition: Id,
    pub btn_magic_move: Id,
    pub btn_add_build: Id,
    pub btn_step_build: Id,
    pub btn_rehearse: Id,
    pub btn_export_webm: Id,
    pub btn_play_transition: Id,
    pub btn_presenter: Id,
    pub btn_save_pptx: Id,
    pub btn_import_pptx: Id,
    pub btn_save_odp: Id,
    pub btn_import_odp: Id,
    pub btn_export_pdf: Id,

    pub field_title: Id,
    pub field_body: Id,
    pub field_notes: Id,

    pub slide_list: Id,
    pub status_label: Id,

    pub controls: Vec<(&'static str, Id)>,

    pub preview_w: u32,
    pub preview_h: u32,
    pub preview_buf: Vec<u32>,
}

impl Burlesque {
    pub fn build(ui: &mut Ui) -> Self {
        let mut deck = Deck::new(AspectRatio::Ratio16x9);

        // Slide 1: Title Slide
        let slide = deck.add_slide(SlideLayout::TitleSlide);
        sync_slide_elements(
            slide,
            AspectRatio::Ratio16x9,
            "Burlesque Presentations",
            "Keynote-Grade Presentation Engine for SexOS",
        );
        slide.set_speaker_notes("Welcome to Burlesque presentations on SexOS. Keynote-class themes, transitions, presenter view, and full PPTX/ODP/PDF workflows.");

        let preview_w = 640;
        let preview_h = 360;
        let mut preview_buf = vec![0u32; (preview_w * preview_h) as usize];

        // Register commands and shortcuts
        for c in [
            Command::new("burlesque.export-pdf", "Export PDF").shortcut("Ctrl+E"),
            Command::new("burlesque.export-webm", "Export WebM Video"),
            Command::new("burlesque.save-pptx", "Save PPTX"),
            Command::new("burlesque.import-pptx", "Import PPTX"),
            Command::new("burlesque.save-odp", "Save ODP"),
            Command::new("burlesque.import-odp", "Import ODP"),
            Command::new("burlesque.presenter", "Presenter View").shortcut("F5"),
            Command::new("burlesque.add-slide", "Add Slide").shortcut("Ctrl+N"),
            Command::new("burlesque.delete-slide", "Delete Slide"),
            Command::new("burlesque.prev", "Previous Slide").shortcut("Left"),
            Command::new("burlesque.next", "Next Slide").shortcut("Right"),
            Command::new("burlesque.add-build", "Add Build Step"),
            Command::new("burlesque.step-build", "Step Build Animation"),
            Command::new("burlesque.layout-title", "Title Layout"),
            Command::new("burlesque.layout-body", "Title and Body Layout"),
            Command::new("burlesque.layout-twocol", "Two Column Layout"),
            Command::new("burlesque.cycle-theme", "Cycle Theme").shortcut("Ctrl+T"),
            Command::new("burlesque.theme-velvet", "Burlesque Velvet"),
            Command::new("burlesque.theme-editorial", "Editorial Light"),
            Command::new("burlesque.theme-midnight", "Midnight Slate"),
            Command::new("burlesque.theme-champagne", "Champagne Ivory"),
            Command::new("burlesque.theme-cyber", "Cyber Prism"),
            Command::new("burlesque.theme-swiss", "Swiss Modern"),
            Command::new("burlesque.cycle-transition", "Cycle Transition"),
            Command::new("burlesque.magic-move", "Magic Move Transition"),
            Command::new("burlesque.transition-none", "No Transition"),
            Command::new("burlesque.transition-dissolve", "Dissolve Transition"),
            Command::new("burlesque.transition-push", "Push Transition"),
            Command::new("burlesque.transition-wipe", "Wipe Transition"),
            Command::new("burlesque.transition-zoom", "Zoom Transition"),
            Command::new("burlesque.play-transition", "Play Transition").shortcut("Space"),
            Command::new("burlesque.rehearse", "Rehearse Timings"),
        ] {
            ui.commands.add(c);
        }

        let root = ui.root();
        ui.set_layout(root, Layout::column());

        // Menu Bar
        ui.menubar(
            root,
            vec![
                Menu::new("File")
                    .command("burlesque.export-pdf")
                    .command("burlesque.export-webm")
                    .command("burlesque.save-pptx")
                    .command("burlesque.import-pptx")
                    .command("burlesque.save-odp")
                    .command("burlesque.import-odp")
                    .separator()
                    .command("weft.close"),
                Menu::new("Slide")
                    .command("burlesque.add-slide")
                    .command("burlesque.delete-slide")
                    .separator()
                    .command("burlesque.add-build")
                    .command("burlesque.step-build")
                    .separator()
                    .command("burlesque.prev")
                    .command("burlesque.next"),
                Menu::new("Layout")
                    .command("burlesque.layout-title")
                    .command("burlesque.layout-body")
                    .command("burlesque.layout-twocol"),
                Menu::new("Theme")
                    .command("burlesque.cycle-theme")
                    .separator()
                    .command("burlesque.theme-velvet")
                    .command("burlesque.theme-editorial")
                    .command("burlesque.theme-midnight")
                    .command("burlesque.theme-champagne")
                    .command("burlesque.theme-cyber")
                    .command("burlesque.theme-swiss"),
                Menu::new("Transition")
                    .command("burlesque.cycle-transition")
                    .separator()
                    .command("burlesque.magic-move")
                    .command("burlesque.transition-none")
                    .command("burlesque.transition-dissolve")
                    .command("burlesque.transition-push")
                    .command("burlesque.transition-wipe")
                    .command("burlesque.transition-zoom")
                    .separator()
                    .command("burlesque.play-transition"),
                Menu::new("View")
                    .command("burlesque.presenter")
                    .command("burlesque.rehearse"),
            ],
        );

        // Top Toolbar row 1: Slides & Navigation & Presentation modes
        let row_nav = ui.row(root);
        let btn_prev = ui.button(row_nav, "Prev");
        let btn_next = ui.button(row_nav, "Next");

        ui.separator(row_nav);

        let btn_add_slide = ui.button(row_nav, "Add Slide");
        let btn_delete_slide = ui.button(row_nav, "Delete Slide");

        ui.separator(row_nav);

        let btn_presenter = ui.button(row_nav, "Presenter View");
        let btn_rehearse = ui.button(row_nav, "Rehearse");

        ui.separator(row_nav);

        let btn_theme = ui.button(row_nav, "Theme");
        let btn_transition = ui.button(row_nav, "Transition");
        let btn_magic_move = ui.button(row_nav, "Magic Move");
        let btn_play_transition = ui.button(row_nav, "Play");

        // Top Toolbar row 2: Layouts & Format Import/Export
        let row_actions = ui.row(root);
        let btn_layout_title = ui.button(row_actions, "Title");
        let btn_layout_body = ui.button(row_actions, "Body");
        let btn_layout_twocol = ui.button(row_actions, "2-Col");

        ui.separator(row_actions);

        let btn_add_build = ui.button(row_actions, "Add Build");
        let btn_step_build = ui.button(row_actions, "Step Build");

        ui.separator(row_actions);

        let btn_save_pptx = ui.button(row_actions, "Save PPTX");
        let btn_import_pptx = ui.button(row_actions, "Open PPTX");
        let btn_save_odp = ui.button(row_actions, "Save ODP");
        let btn_import_odp = ui.button(row_actions, "Open ODP");

        ui.separator(row_actions);

        let btn_export_pdf = ui.button(row_actions, "Export PDF");
        let btn_export_webm = ui.button(row_actions, "Export WebM");

        // Main Workspace: Sidebar on left (slide list + text fields), Preview on right
        let workspace = ui.row(root);
        ui.set_layout(workspace, Layout::row().height(384.0));

        // Sidebar
        let sidebar = ui.column(workspace);
        ui.set_layout(sidebar, Layout::column().width(240.0).height(372.0));
        ui.label(sidebar, "Slides:");
        let slide_list = ui.list(sidebar, &["1: Burlesque Presentations"]);
        ui.set_layout(slide_list, Layout::default().width(220.0).height(110.0));

        ui.label(sidebar, "Slide Title:");
        let field_title = ui.field(sidebar, "Burlesque Presentations", "Title");
        ui.set_layout(field_title, Layout::default().width(220.0));

        ui.label(sidebar, "Slide Body:");
        let field_body = ui.field(sidebar, "Keynote-Grade Presentation Engine for SexOS", "Body");
        ui.set_layout(field_body, Layout::default().width(220.0));

        ui.label(sidebar, "Speaker Notes:");
        let field_notes = ui.field(
            sidebar,
            "Welcome to Burlesque presentations on SexOS. Keynote-class themes, transitions, presenter view, and full PPTX/ODP/PDF workflows.",
            "Notes",
        );
        ui.set_layout(field_notes, Layout::default().width(220.0));

        // Preview container
        let preview_col = ui.column(workspace);
        ui.set_layout(preview_col, Layout::column().width(656.0).height(372.0));
        let first_slide = deck.get_slide(0).expect("first slide");
        render_slide(
            first_slide,
            &deck.theme,
            deck.aspect_ratio,
            &mut preview_buf,
            preview_w,
            preview_h,
        );

        let preview_id = ui.image(
            preview_col,
            preview_w as usize,
            preview_h as usize,
            preview_buf.clone(),
        );
        ui.set_layout(
            preview_id,
            Layout::default().width(preview_w as f32).height(preview_h as f32),
        );

        let status_label = ui.label(root, "Slide 1 of 1 | Presenter: Off | Theme: Burlesque Velvet | Transition: None");

        let controls = vec![
            ("add_slide", btn_add_slide),
            ("delete_slide", btn_delete_slide),
            ("prev", btn_prev),
            ("next", btn_next),
            ("layout_title", btn_layout_title),
            ("layout_body", btn_layout_body),
            ("layout_twocol", btn_layout_twocol),
            ("theme", btn_theme),
            ("transition", btn_transition),
            ("magic_move", btn_magic_move),
            ("add_build", btn_add_build),
            ("step_build", btn_step_build),
            ("rehearse", btn_rehearse),
            ("play_transition", btn_play_transition),
            ("presenter", btn_presenter),
            ("save_pptx", btn_save_pptx),
            ("import_pptx", btn_import_pptx),
            ("save_odp", btn_save_odp),
            ("import_odp", btn_import_odp),
            ("export_pdf", btn_export_pdf),
            ("export_webm", btn_export_webm),
        ];

        Self {
            deck,
            current_slide: 0,
            presenter_mode: false,
            presenter_session: None,
            rehearsal_session: None,
            active_build_index: 0,
            magic_move_preview: None,
            preview_id,
            btn_add_slide,
            btn_delete_slide,
            btn_prev,
            btn_next,
            btn_layout_title,
            btn_layout_body,
            btn_layout_twocol,
            btn_theme,
            btn_transition,
            btn_magic_move,
            btn_add_build,
            btn_step_build,
            btn_rehearse,
            btn_export_webm,
            btn_play_transition,
            btn_presenter,
            btn_save_pptx,
            btn_import_pptx,
            btn_save_odp,
            btn_import_odp,
            btn_export_pdf,
            field_title,
            field_body,
            field_notes,
            slide_list,
            status_label,
            controls,
            preview_w,
            preview_h,
            preview_buf,
        }
    }

    pub fn theme_slug(&self) -> &'static str {
        match self.deck.theme.name.as_str() {
            "Burlesque Velvet" => "burlesque_velvet",
            "Editorial Light" => "editorial_light",
            "Midnight Slate" => "midnight_slate",
            "Champagne Ivory" => "champagne_ivory",
            "Cyber Prism" => "cyber_prism",
            "Swiss Modern" => "swiss_modern",
            _ => "custom",
        }
    }

    pub fn current_transition(&self) -> Transition {
        self.deck.get_slide(self.current_slide).map(|s| s.transition).unwrap_or(Transition::None)
    }

    pub fn cycle_theme(&mut self, ui: &mut Ui) -> &'static str {
        let all = Theme::all_themes();
        let current_name = &self.deck.theme.name;
        let next_theme = match all.iter().position(|t| &t.name == current_name) {
            Some(idx) => all[(idx + 1) % all.len()].clone(),
            None => all[0].clone(),
        };
        self.set_theme(next_theme, ui);
        self.theme_slug()
    }

    pub fn set_theme(&mut self, theme: Theme, ui: &mut Ui) {
        self.deck.set_theme(theme);
        if let Some(ref mut session) = self.presenter_session {
            session.deck.set_theme(self.deck.theme.clone());
        }
        self.refresh_preview(ui);
    }

    pub fn cycle_transition(&mut self) -> Transition {
        let current = self.current_transition();
        let next = match current {
            Transition::None => Transition::Dissolve,
            Transition::Dissolve => Transition::PushLeft,
            Transition::PushLeft => Transition::WipeRight,
            Transition::WipeRight => Transition::ZoomIn,
            Transition::ZoomIn => Transition::MagicMove,
            Transition::MagicMove => Transition::None,
            _ => Transition::None,
        };
        self.set_transition(next);
        next
    }

    pub fn set_transition(&mut self, transition: Transition) {
        if let Some(slide) = self.deck.get_slide_mut(self.current_slide) {
            slide.transition = transition;
        }
        if let Some(ref mut session) = self.presenter_session {
            if let Some(slide) = session.deck.get_slide_mut(self.current_slide) {
                slide.transition = transition;
            }
        }
    }

    pub fn play_transition(&mut self, ui: &mut Ui) -> (Transition, usize) {
        let mut transition = self.current_transition();
        if transition == Transition::None {
            transition = Transition::Dissolve;
        }
        let frames = 8usize;
        let w = self.preview_w;
        let h = self.preview_h;
        let len = (w * h) as usize;

        let next_idx = if self.current_slide + 1 < self.deck.slide_count() {
            self.current_slide + 1
        } else if self.current_slide > 0 {
            self.current_slide - 1
        } else {
            self.current_slide
        };

        if transition == Transition::MagicMove {
            let from_slide = self.deck.get_slide(self.current_slide);
            let to_slide = self.deck.get_slide(next_idx);
            if let (Some(sf), Some(st)) = (from_slide, to_slide) {
                for f in 1..=frames {
                    let t = f as f32 / frames as f32;
                    render_magic_move_frame(
                        sf,
                        st,
                        &self.deck.theme,
                        self.deck.aspect_ratio,
                        t,
                        &mut self.preview_buf,
                        w,
                        h,
                    );
                }
                render_magic_move_frame(
                    sf,
                    st,
                    &self.deck.theme,
                    self.deck.aspect_ratio,
                    0.5,
                    &mut self.preview_buf,
                    w,
                    h,
                );
            }
            self.magic_move_preview = Some(0.5);
        } else {
            self.magic_move_preview = None;
            let mut from_buf = vec![0u32; len];
            let mut to_buf = vec![0u32; len];

            if let Some(slide) = self.deck.get_slide(self.current_slide) {
                render_slide(slide, &self.deck.theme, self.deck.aspect_ratio, &mut from_buf, w, h);
            }
            if next_idx != self.current_slide {
                if let Some(next_slide) = self.deck.get_slide(next_idx) {
                    render_slide(next_slide, &self.deck.theme, self.deck.aspect_ratio, &mut to_buf, w, h);
                }
            } else {
                let mut alt_theme = self.deck.theme.clone();
                alt_theme.palette.background = self.deck.theme.palette.primary_accent;
                if let Some(slide) = self.deck.get_slide(self.current_slide) {
                    render_slide(slide, &alt_theme, self.deck.aspect_ratio, &mut to_buf, w, h);
                }
            }

            for f in 1..=frames {
                let t = f as f32 / frames as f32;
                render_transition_frame(&from_buf, &to_buf, &mut self.preview_buf, w, h, transition, t);
            }
        }
        ui.set_image(self.preview_id, w as usize, h as usize, self.preview_buf.clone());
        self.update_status(ui);
        (transition, frames)
    }

    pub fn refresh_preview(&mut self, ui: &mut Ui) {
        if let Some(ref session) = self.rehearsal_session {
            render_rehearsal_hud(
                session,
                &mut self.preview_buf,
                self.preview_w,
                self.preview_h,
            );
        } else if self.presenter_mode {
            if let Some(ref mut session) = self.presenter_session {
                render_presenter_view(
                    session,
                    &mut self.preview_buf,
                    self.preview_w,
                    self.preview_h,
                );
            }
        } else if let Some(frac) = self.magic_move_preview {
            let next_idx = if self.current_slide + 1 < self.deck.slide_count() {
                self.current_slide + 1
            } else if self.current_slide > 0 {
                self.current_slide - 1
            } else {
                self.current_slide
            };
            if let (Some(sf), Some(st)) = (
                self.deck.get_slide(self.current_slide),
                self.deck.get_slide(next_idx),
            ) {
                render_magic_move_frame(
                    sf,
                    st,
                    &self.deck.theme,
                    self.deck.aspect_ratio,
                    frac,
                    &mut self.preview_buf,
                    self.preview_w,
                    self.preview_h,
                );
            }
        } else if let Some(slide) = self.deck.get_slide(self.current_slide) {
            if !slide.builds.is_empty() && self.active_build_index > 0 {
                render_slide_at_build(
                    slide,
                    &self.deck.theme,
                    self.deck.aspect_ratio,
                    self.active_build_index,
                    None,
                    &mut self.preview_buf,
                    self.preview_w,
                    self.preview_h,
                );
            } else {
                render_slide(
                    slide,
                    &self.deck.theme,
                    self.deck.aspect_ratio,
                    &mut self.preview_buf,
                    self.preview_w,
                    self.preview_h,
                );
            }
        }

        ui.set_image(
            self.preview_id,
            self.preview_w as usize,
            self.preview_h as usize,
            self.preview_buf.clone(),
        );

        self.update_status(ui);
    }

    pub fn update_status(&self, ui: &mut Ui) {
        let builds_info = if let Some(s) = self.deck.get_slide(self.current_slide) {
            if s.builds.is_empty() {
                String::new()
            } else {
                format!(" | Builds: {}/{}", self.active_build_index, s.builds.len())
            }
        } else {
            String::new()
        };
        let morph_info = if self.magic_move_preview.is_some() {
            " | MORPH 50%"
        } else {
            ""
        };
        let text = format!(
            "Slide {} of {} | Presenter: {} | Theme: {} | Transition: {}{}{}{}",
            self.current_slide + 1,
            self.deck.slide_count(),
            if self.presenter_mode { "ON" } else { "Off" },
            self.deck.theme.name,
            self.current_transition().name(),
            morph_info,
            builds_info,
            if self.rehearsal_session.is_some() { " | REHEARSAL" } else { "" },
        );
        ui.set_text(self.status_label, &text);
    }

    pub fn sync_fields_to_current_slide(&mut self, ui: &mut Ui) {
        if let Some(slide) = self.deck.get_slide(self.current_slide) {
            ui.set_text(self.field_title, &slide.title);
            let body = slide_body_text(slide);
            ui.set_text(self.field_body, &body);
            ui.set_text(self.field_notes, slide.notes());
        }
        self.refresh_slide_list(ui);
    }

    pub fn refresh_slide_list(&mut self, ui: &mut Ui) {
        let names: Vec<String> = self
            .deck
            .slides
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let t = if s.title.is_empty() {
                    "Untitled Slide"
                } else {
                    &s.title
                };
                format!("{}: {}", i + 1, t)
            })
            .collect();
        let rows = Rows(names.into_iter().map(|s| vec![s]).collect());
        ui.set_rows(self.slide_list, rows);
    }

    pub fn toggle_presenter_mode(&mut self, ui: &mut Ui) {
        self.magic_move_preview = None;
        self.presenter_mode = !self.presenter_mode;
        if self.presenter_mode {
            let mut session = PresenterSession::new(self.deck.clone());
            session.goto_slide(self.current_slide);
            session.start_timer();
            self.presenter_session = Some(session);
        } else {
            self.presenter_session = None;
        }
        self.refresh_preview(ui);
    }

    pub fn on_response(&mut self, ui: &mut Ui, response: &Response) -> bool {
        let mut command = None;

        if let Response::Clicked(id) = response {
            if *id == self.btn_add_slide {
                command = Some("burlesque.add-slide");
            } else if *id == self.btn_delete_slide {
                command = Some("burlesque.delete-slide");
            } else if *id == self.btn_prev {
                command = Some("burlesque.prev");
            } else if *id == self.btn_next {
                command = Some("burlesque.next");
            } else if *id == self.btn_layout_title {
                command = Some("burlesque.layout-title");
            } else if *id == self.btn_layout_body {
                command = Some("burlesque.layout-body");
            } else if *id == self.btn_layout_twocol {
                command = Some("burlesque.layout-twocol");
            } else if *id == self.btn_theme {
                command = Some("burlesque.cycle-theme");
            } else if *id == self.btn_transition {
                command = Some("burlesque.cycle-transition");
            } else if *id == self.btn_play_transition {
                command = Some("burlesque.play-transition");
            } else if *id == self.btn_presenter {
                command = Some("burlesque.presenter");
            } else if *id == self.btn_save_pptx {
                command = Some("burlesque.save-pptx");
            } else if *id == self.btn_import_pptx {
                command = Some("burlesque.import-pptx");
            } else if *id == self.btn_save_odp {
                command = Some("burlesque.save-odp");
            } else if *id == self.btn_import_odp {
                command = Some("burlesque.import-odp");
            } else if *id == self.btn_export_pdf {
                command = Some("burlesque.export-pdf");
            } else if *id == self.btn_export_webm {
                command = Some("burlesque.export-webm");
            } else if *id == self.btn_magic_move {
                command = Some("burlesque.magic-move");
            } else if *id == self.btn_add_build {
                command = Some("burlesque.add-build");
            } else if *id == self.btn_step_build {
                command = Some("burlesque.step-build");
            } else if *id == self.btn_rehearse {
                command = Some("burlesque.rehearse");
            }
        }

        if let Response::Changed(id) | Response::Submitted(id) = response {
            if *id == self.field_title || *id == self.field_body || *id == self.field_notes {
                let title = ui.text_of(self.field_title).to_string();
                let body = ui.text_of(self.field_body).to_string();
                let notes = ui.text_of(self.field_notes).to_string();
                let aspect = self.deck.aspect_ratio;

                if let Some(slide) = self.deck.get_slide_mut(self.current_slide) {
                    slide.set_speaker_notes(notes);
                    sync_slide_elements(slide, aspect, &title, &body);
                }
                if let Some(ref mut session) = self.presenter_session {
                    session.deck = self.deck.clone();
                }
                self.refresh_slide_list(ui);
                self.refresh_preview(ui);
                return true;
            }
        }

        if let Response::Command(cmd) = response {
            command = Some(cmd.as_str());
        }

        let Some(cmd) = command else {
            return false;
        };

        match cmd {
            "burlesque.add-slide" => {
                self.magic_move_preview = None;
                let n = self.deck.slide_count() + 1;
                let aspect = self.deck.aspect_ratio;
                let slide = self.deck.add_slide(SlideLayout::TitleAndBody);
                let title = format!("Slide {}", n);
                sync_slide_elements(slide, aspect, &title, "Content for this slide");
                slide.set_speaker_notes(format!("Notes for slide {}", n));
                self.current_slide = self.deck.slide_count() - 1;
                self.active_build_index = 0;
                if let Some(ref mut session) = self.presenter_session {
                    session.deck = self.deck.clone();
                    session.goto_slide(self.current_slide);
                }
                self.sync_fields_to_current_slide(ui);
                self.refresh_preview(ui);
                true
            }
            "burlesque.delete-slide" => {
                self.magic_move_preview = None;
                if self.deck.slide_count() > 1 {
                    self.deck.remove_slide(self.current_slide);
                    if self.current_slide >= self.deck.slide_count() {
                        self.current_slide = self.deck.slide_count().saturating_sub(1);
                    }
                    self.active_build_index = 0;
                    if let Some(ref mut session) = self.presenter_session {
                        session.deck = self.deck.clone();
                        session.goto_slide(self.current_slide);
                    }
                    self.sync_fields_to_current_slide(ui);
                    self.refresh_preview(ui);
                    true
                } else {
                    false
                }
            }
            "burlesque.prev" => {
                self.magic_move_preview = None;
                if self.current_slide > 0 {
                    self.current_slide -= 1;
                    self.active_build_index = 0;
                    if let Some(ref mut session) = self.presenter_session {
                        session.prev_slide();
                    }
                    self.sync_fields_to_current_slide(ui);
                    self.refresh_preview(ui);
                    true
                } else {
                    false
                }
            }
            "burlesque.next" => {
                self.magic_move_preview = None;
                if let Some(ref mut session) = self.rehearsal_session {
                    let _ = session.record_advance();
                    self.current_slide = session.current_slide;
                    self.active_build_index = session.current_build;
                    self.sync_fields_to_current_slide(ui);
                    self.refresh_preview(ui);
                    return true;
                }
                if self.current_slide + 1 < self.deck.slide_count() {
                    self.current_slide += 1;
                    self.active_build_index = 0;
                    if let Some(ref mut session) = self.presenter_session {
                        session.next_slide();
                    }
                    self.sync_fields_to_current_slide(ui);
                    self.refresh_preview(ui);
                    true
                } else {
                    false
                }
            }
            "burlesque.layout-title" => {
                self.magic_move_preview = None;
                let aspect = self.deck.aspect_ratio;
                if let Some(slide) = self.deck.get_slide_mut(self.current_slide) {
                    slide.layout = SlideLayout::TitleSlide;
                    let title = ui.text_of(self.field_title).to_string();
                    let body = ui.text_of(self.field_body).to_string();
                    sync_slide_elements(slide, aspect, &title, &body);
                    self.refresh_preview(ui);
                    true
                } else {
                    false
                }
            }
            "burlesque.layout-body" => {
                self.magic_move_preview = None;
                let aspect = self.deck.aspect_ratio;
                if let Some(slide) = self.deck.get_slide_mut(self.current_slide) {
                    slide.layout = SlideLayout::TitleAndBody;
                    let title = ui.text_of(self.field_title).to_string();
                    let body = ui.text_of(self.field_body).to_string();
                    sync_slide_elements(slide, aspect, &title, &body);
                    self.refresh_preview(ui);
                    true
                } else {
                    false
                }
            }
            "burlesque.layout-twocol" => {
                self.magic_move_preview = None;
                let aspect = self.deck.aspect_ratio;
                if let Some(slide) = self.deck.get_slide_mut(self.current_slide) {
                    slide.layout = SlideLayout::TwoColumn;
                    let title = ui.text_of(self.field_title).to_string();
                    let body = ui.text_of(self.field_body).to_string();
                    sync_slide_elements(slide, aspect, &title, &body);
                    self.refresh_preview(ui);
                    true
                } else {
                    false
                }
            }
            "burlesque.cycle-theme" => {
                self.cycle_theme(ui);
                true
            }
            "burlesque.theme-velvet" => {
                self.set_theme(Theme::burlesque_velvet(), ui);
                true
            }
            "burlesque.theme-editorial" => {
                self.set_theme(Theme::editorial_light(), ui);
                true
            }
            "burlesque.theme-midnight" => {
                self.set_theme(Theme::midnight_slate(), ui);
                true
            }
            "burlesque.theme-champagne" => {
                self.set_theme(Theme::champagne_ivory(), ui);
                true
            }
            "burlesque.theme-cyber" => {
                self.set_theme(Theme::cyber_prism(), ui);
                true
            }
            "burlesque.theme-swiss" => {
                self.set_theme(Theme::swiss_modern(), ui);
                true
            }
            "burlesque.cycle-transition" => {
                let next = self.cycle_transition();
                self.magic_move_preview = (next == Transition::MagicMove).then_some(0.5);
                self.refresh_preview(ui);
                true
            }
            "burlesque.transition-none" => {
                self.magic_move_preview = None;
                self.set_transition(Transition::None);
                self.refresh_preview(ui);
                true
            }
            "burlesque.transition-dissolve" => {
                self.magic_move_preview = None;
                self.set_transition(Transition::Dissolve);
                self.refresh_preview(ui);
                true
            }
            "burlesque.transition-push" => {
                self.magic_move_preview = None;
                self.set_transition(Transition::PushLeft);
                self.refresh_preview(ui);
                true
            }
            "burlesque.transition-wipe" => {
                self.magic_move_preview = None;
                self.set_transition(Transition::WipeRight);
                self.refresh_preview(ui);
                true
            }
            "burlesque.transition-zoom" => {
                self.magic_move_preview = None;
                self.set_transition(Transition::ZoomIn);
                self.refresh_preview(ui);
                true
            }
            "burlesque.magic-move" => {
                self.set_transition(Transition::MagicMove);
                self.magic_move_preview = Some(0.5);
                self.refresh_preview(ui);
                true
            }
            "burlesque.add-build" => {
                self.magic_move_preview = None;
                if let Some(slide) = self.deck.get_slide_mut(self.current_slide) {
                    let elem_idx = slide.elements.len().saturating_sub(1);
                    slide.add_build(BuildStep::new(elem_idx, BuildEffect::DissolveIn).with_duration(400));
                    self.active_build_index = slide.builds.len();
                    self.refresh_preview(ui);
                    true
                } else {
                    false
                }
            }
            "burlesque.step-build" => {
                self.magic_move_preview = None;
                if let Some(slide) = self.deck.get_slide(self.current_slide) {
                    let total = slide.builds.len();
                    if total > 0 {
                        self.active_build_index = (self.active_build_index + 1) % (total + 1);
                        self.refresh_preview(ui);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            "burlesque.rehearse" => {
                self.magic_move_preview = None;
                if self.rehearsal_session.is_some() {
                    let session = self.rehearsal_session.take().unwrap();
                    session.apply_to_deck(&mut self.deck);
                } else {
                    let mut session = RehearsalSession::new(self.deck.clone());
                    session.current_slide = self.current_slide;
                    self.rehearsal_session = Some(session);
                }
                self.refresh_preview(ui);
                true
            }
            "burlesque.play-transition" => {
                self.play_transition(ui);
                true
            }
            "burlesque.presenter" => {
                self.toggle_presenter_mode(ui);
                true
            }
            _ => false,
        }
    }

    pub fn export_pdf(&self) -> Result<Vec<u8>, burlesque_core::Error> {
        export_pdf(&self.deck)
    }

    pub fn export_webm(&self, width: u32, height: u32, fps: u32) -> Result<(Vec<u8>, usize), WebmExportError> {
        export_deck_webm(&self.deck, width, height, fps)
    }

    pub fn export_pptx(&self) -> Result<Vec<u8>, burlesque_core::Error> {
        self.deck.export_pptx()
    }

    pub fn import_pptx(&mut self, bytes: &[u8], ui: &mut Ui) -> Result<(usize, usize), burlesque_core::Error> {
        let deck = Deck::open_pptx(bytes)?;
        let slides = deck.slide_count();
        let notes = deck.slides.iter().filter(|s| !s.notes().is_empty()).count();
        self.deck = deck;
        self.current_slide = 0;
        self.sync_fields_to_current_slide(ui);
        self.refresh_preview(ui);
        Ok((slides, notes))
    }

    pub fn export_odp(&self) -> Result<Vec<u8>, burlesque_core::Error> {
        self.deck.export_odp()
    }

    pub fn import_odp(&mut self, bytes: &[u8], ui: &mut Ui) -> Result<(usize, usize), burlesque_core::Error> {
        let deck = Deck::open_odp(bytes)?;
        let slides = deck.slide_count();
        let notes = deck.slides.iter().filter(|s| !s.notes().is_empty()).count();
        self.deck = deck;
        self.current_slide = 0;
        self.sync_fields_to_current_slide(ui);
        self.refresh_preview(ui);
        Ok((slides, notes))
    }

    pub fn step(&mut self, ui: &mut Ui, event: Event) -> Vec<Response> {
        let responses = ui.handle(event);
        let mut changed = false;
        for r in &responses {
            if self.on_response(ui, r) {
                changed = true;
            }
        }
        if changed {
            self.refresh_preview(ui);
        }
        responses
    }
}

pub fn sync_slide_elements(
    slide: &mut Slide,
    aspect_ratio: AspectRatio,
    title: &str,
    body: &str,
) {
    slide.title = title.into();
    slide.elements.clear();

    let width = aspect_ratio.width();
    let height = aspect_ratio.height();
    let margin_x = width * 0.08;
    let margin_y = height * 0.08;
    let content_w = width - margin_x * 2.0;

    match slide.layout {
        SlideLayout::TitleSlide => {
            let hero_shape = SlideElement::Shape(Shape {
                bounds: Rect::new(margin_x, height * 0.18, width * 0.12, height * 0.07),
                kind: ShapeKind::RoundedRect { radius: 10.0 },
                fill: Some(sex_canvas::Color::rgb(0x43, 0x38, 0xCA)),
                stroke: Some(StrokeStyle { color: sex_canvas::Color::WHITE, width: 2.0 }),
                magic_key: Some("hero-stat".into()),
            });
            slide.add_element(hero_shape);

            let title_y = height * 0.30;
            let title_h = height * 0.22;
            let mut title_tb = TextBox::new(Rect::new(margin_x, title_y, content_w, title_h));
            title_tb.magic_key = Some("hero-title".into());
            let mut p = Paragraph::new();
            p.push_run(TextRun::plain(title, 48.0).bold());
            title_tb.paragraphs.push(p);
            slide.add_element(SlideElement::TextBox(title_tb));

            if !body.is_empty() {
                let sub_y = height * 0.55;
                let sub_h = height * 0.18;
                let mut sub_tb = TextBox::new(Rect::new(margin_x, sub_y, content_w, sub_h));
                let mut p2 = Paragraph::new();
                p2.push_run(TextRun::plain(body, 28.0));
                sub_tb.paragraphs.push(p2);
                slide.add_element(SlideElement::TextBox(sub_tb));
            }
        }
        SlideLayout::TitleAndBody => {
            let hero_shape = SlideElement::Shape(Shape {
                bounds: Rect::new(width - margin_x - width * 0.14, margin_y, width * 0.14, height * 0.08),
                kind: ShapeKind::RoundedRect { radius: 14.0 },
                fill: Some(sex_canvas::Color::rgb(0xDC, 0x26, 0x26)),
                stroke: Some(StrokeStyle { color: sex_canvas::Color::WHITE, width: 2.0 }),
                magic_key: Some("hero-stat".into()),
            });
            slide.add_element(hero_shape);

            let title_h = height * 0.14;
            let mut title_tb = TextBox::new(Rect::new(margin_x, margin_y, content_w - width * 0.16, title_h));
            title_tb.magic_key = Some("hero-title".into());
            let mut p = Paragraph::new();
            p.push_run(TextRun::plain(title, 44.0).bold());
            title_tb.paragraphs.push(p);
            slide.add_element(SlideElement::TextBox(title_tb));

            let body_y = margin_y + title_h + height * 0.04;
            let body_h = height - body_y - margin_y;
            let mut body_tb = TextBox::new(Rect::new(margin_x, body_y, content_w, body_h));
            for line in body.lines() {
                let trimmed = line.trim();
                if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
                    body_tb.paragraphs.push(Paragraph::bullet(item, 24.0, 1));
                } else if !trimmed.is_empty() {
                    body_tb.paragraphs.push(Paragraph::from_text(trimmed, 24.0));
                }
            }
            slide.add_element(SlideElement::TextBox(body_tb));
        }
        SlideLayout::TwoColumn => {
            let title_h = height * 0.14;
            let mut title_tb = TextBox::new(Rect::new(margin_x, margin_y, content_w, title_h));
            title_tb.magic_key = Some("hero-title".into());
            let mut p = Paragraph::new();
            p.push_run(TextRun::plain(title, 44.0).bold());
            title_tb.paragraphs.push(p);
            slide.add_element(SlideElement::TextBox(title_tb));

            let body_y = margin_y + title_h + height * 0.04;
            let body_h = height - body_y - margin_y;
            let gap = width * 0.04;
            let col_w = (content_w - gap) / 2.0;

            let mut col1_tb = TextBox::new(Rect::new(margin_x, body_y, col_w, body_h));
            let mut col2_tb = TextBox::new(Rect::new(margin_x + col_w + gap, body_y, col_w, body_h));

            let mut col1 = true;
            for line in body.lines() {
                let trimmed = line.trim();
                if trimmed == "---" {
                    col1 = false;
                    continue;
                }
                let target = if col1 { &mut col1_tb } else { &mut col2_tb };
                if let Some(item) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
                    target.paragraphs.push(Paragraph::bullet(item, 22.0, 1));
                } else if !trimmed.is_empty() {
                    target.paragraphs.push(Paragraph::from_text(trimmed, 22.0));
                }
            }
            slide.add_element(SlideElement::TextBox(col1_tb));
            slide.add_element(SlideElement::TextBox(col2_tb));
        }
        _ => {
            let mut tb = TextBox::new(Rect::new(margin_x, margin_y, content_w, height - margin_y * 2.0));
            tb.magic_key = Some("hero-title".into());
            tb.paragraphs.push(Paragraph::from_text(title, 36.0));
            tb.paragraphs.push(Paragraph::from_text(body, 24.0));
            slide.add_element(SlideElement::TextBox(tb));
        }
    }
}

pub fn slide_body_text(slide: &Slide) -> String {
    let mut out = String::new();
    let textboxes = slide
        .elements
        .iter()
        .filter(|e| matches!(e, SlideElement::TextBox(_)))
        .count();
    let mut seen_tb = 0usize;
    for elem in &slide.elements {
        if let SlideElement::TextBox(tb) = elem {
            seen_tb += 1;
            if tb.magic_key.as_deref() == Some("hero-title") || (seen_tb == 1 && textboxes > 1) {
                continue;
            }
            let t = tb.plain_text();
            if !t.is_empty() {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&t);
            }
        }
    }
    out
}

/// Error returned when exporting a rehearsed Burlesque presentation via Splicy's encoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebmExportError {
    InvalidTimeline,
    /// Splicy's `sex-splice` timeline rendered all frames cleanly, and `sex-media`'s
    /// FFmpeg VP9 export profile (`FFMPEG_RECORDING=2`) was not compiled into this binary.
    EncoderUnavailable {
        frames: usize,
        frame_hash: u64,
    },
    CodecError(i32),
}

struct DeckPictureSource<'a> {
    deck: &'a Deck,
    video_tl: &'a VideoTimeline,
    w: u32,
    h: u32,
    fps: u32,
}

impl<'a> sex_splice::export::Pictures for DeckPictureSource<'a> {
    type Error = ();

    fn picture(
        &mut self,
        _media: u32,
        source: sex_splice::Micros,
        out: &mut [u32],
    ) -> Result<(), Self::Error> {
        let idx = ((source.max(0) as u64 * self.fps as u64) / sex_splice::SECOND as u64) as usize;
        let clamped = idx.min(self.video_tl.total_frames().saturating_sub(1));
        self.video_tl.render_frame(self.deck, clamped, out, self.w, self.h);
        Ok(())
    }
}

struct DeckFrameSink<F> {
    next_frame: u64,
    hash: sex_splice::export::FrameHash,
    on_frame: F,
}

impl<F> sex_splice::export::Frames for DeckFrameSink<F>
where
    F: FnMut(u64, &[u32]) -> Result<(), WebmExportError>,
{
    type Error = WebmExportError;

    fn frame(&mut self, pixels: &[u32], _at: sex_splice::Micros) -> Result<(), Self::Error> {
        self.hash.add(pixels);
        let idx = self.next_frame;
        self.next_frame += 1;
        (self.on_frame)(idx, pixels)
    }
}

/// Build a Splicy [`sex_splice::Timeline`] for `deck`'s rehearsed timings, intra-slide
/// build steps, and Magic Move transitions, then walk every frame through
/// [`sex_splice::export::export_pictures`].
pub fn render_splicy_frames<F>(
    deck: &Deck,
    width: u32,
    height: u32,
    fps: u32,
    on_frame: F,
) -> Result<(sex_splice::Timeline, usize, u64), WebmExportError>
where
    F: FnMut(u64, &[u32]) -> Result<(), WebmExportError>,
{
    let w = (width & !1).max(16);
    let h = (height & !1).max(16);
    let fps = fps.clamp(5, 30);
    let video_tl = VideoTimeline::from_deck(deck, fps);
    let total_frames = video_tl.total_frames();
    if total_frames == 0 {
        return Err(WebmExportError::InvalidTimeline);
    }

    let mut splice_tl = sex_splice::Timeline::new(fps, 1);
    let end_micros = sex_splice::export::frame_time(&splice_tl, total_frames as u64);
    let media_id = splice_tl.add_media(
        sex_splice::MediaKind::Video,
        "burlesque://rehearsed-deck",
        end_micros,
    );
    let track_id = splice_tl.add_track(sex_splice::TrackKind::Video, "Slides & Magic Move");
    splice_tl
        .apply(&sex_splice::Op::AddClip {
            track: track_id,
            media: media_id,
            at: 0,
            source: sex_splice::Range::new(0, end_micros),
        })
        .map_err(|_| WebmExportError::InvalidTimeline)?;

    let mut last_marked_slide: Option<usize> = None;
    for (frame_idx, plan_frame) in video_tl.frames.iter().enumerate() {
        if let burlesque_core::rehearse::TimelineFrame::SlideHold { slide_idx, .. } = plan_frame {
            if last_marked_slide != Some(*slide_idx) {
                last_marked_slide = Some(*slide_idx);
                let at = sex_splice::export::frame_time(&splice_tl, frame_idx as u64);
                let label = deck
                    .get_slide(*slide_idx)
                    .map(|s| format!("Slide {}: {}", slide_idx + 1, s.title))
                    .unwrap_or_else(|| format!("Slide {}", slide_idx + 1));
                let _ = splice_tl.apply(&sex_splice::Op::AddMarker { at, label });
            }
        }
    }

    let mut src = DeckPictureSource {
        deck,
        video_tl: &video_tl,
        w,
        h,
        fps,
    };
    let mut sink = DeckFrameSink {
        next_frame: 0,
        hash: sex_splice::export::FrameHash::default(),
        on_frame,
    };

    let written = sex_splice::export::export_pictures(
        &splice_tl,
        None,
        (w * h) as usize,
        &mut src,
        &mut sink,
    )
    .map_err(|e| match e {
        sex_splice::export::ExportError::Output(err) => err,
        _ => WebmExportError::InvalidTimeline,
    })?;

    Ok((splice_tl, written as usize, sink.hash.0))
}

/// Export `deck` (with rehearsed timings, intra-slide build steps, and Magic Move)
/// strictly through Splicy's timeline export driver (`sex-splice`) and Splicy's VP9 WebM
/// encoder (`sex-media::export::Exporter`). Never invents a custom WebM muxer.
pub fn export_deck_webm(
    deck: &Deck,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<(Vec<u8>, usize), WebmExportError> {
    let w = (width & !1).max(16);
    let h = (height & !1).max(16);
    let fps = fps.clamp(5, 30);

    #[cfg(feature = "sexos")]
    {
        use sex_media::export::{Config, Exporter, Format};
        use sex_media::recording::Error as MediaError;

        let config = Config {
            format: Format::WebmVp9Opus,
            width: w,
            height: h,
            fps_num: fps,
            fps_den: 1,
            sound: false,
            quality: 32,
        };

        match Exporter::new(config) {
            Ok(mut exp) => {
                let mut out = Vec::new();
                const MAX_VIDEO_BYTES: usize = 32 * 1024 * 1024;
                let (_tl, written, _hash) = render_splicy_frames(deck, w, h, fps, |idx, pixels| {
                    exp.picture(pixels, idx)
                        .map_err(|_| WebmExportError::CodecError(-1))?;
                    drain_exporter(&mut exp, &mut out, MAX_VIDEO_BYTES);
                    Ok(())
                })?;
                exp.finish().map_err(|_| WebmExportError::CodecError(-2))?;
                drain_exporter(&mut exp, &mut out, MAX_VIDEO_BYTES);
                if out.is_empty() {
                    return Err(WebmExportError::CodecError(-3));
                }
                return Ok((out, written));
            }
            Err(MediaError::Unavailable) => {
                let (_tl, written, hash) = render_splicy_frames(deck, w, h, fps, |_, _| Ok(()))?;
                return Err(WebmExportError::EncoderUnavailable {
                    frames: written,
                    frame_hash: hash,
                });
            }
            Err(MediaError::Codec(code)) => return Err(WebmExportError::CodecError(code)),
            Err(_) => return Err(WebmExportError::CodecError(-4)),
        }
    }

    #[cfg(not(feature = "sexos"))]
    {
        let (_tl, written, hash) = render_splicy_frames(deck, w, h, fps, |_, _| Ok(()))?;
        Err(WebmExportError::EncoderUnavailable {
            frames: written,
            frame_hash: hash,
        })
    }
}

#[cfg(feature = "sexos")]
fn drain_exporter(exp: &mut sex_media::export::Exporter, out: &mut Vec<u8>, max_bytes: usize) {
    use sex_media::recording::Drain;
    loop {
        match exp.output() {
            Ok(Drain::Write(w)) => {
                let token = w.token;
                let bytes = w.bytes;
                let len = bytes.len();
                if let Some(end) = usize::try_from(w.offset)
                    .ok()
                    .and_then(|off| off.checked_add(len))
                {
                    if end <= max_bytes {
                        if end > out.len() {
                            if out.try_reserve(end - out.len()).is_ok() {
                                out.resize(end, 0);
                            } else {
                                let _ = exp.acknowledge(token, len);
                                continue;
                            }
                        }
                        let off = end - len;
                        out[off..end].copy_from_slice(bytes);
                    }
                }
                let _ = exp.acknowledge(token, len);
            }
            _ => break,
        }
    }
}

