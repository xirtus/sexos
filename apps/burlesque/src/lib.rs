// SPDX-License-Identifier: GPL-3.0-only
//! Burlesque: Weft Presentations App over `burlesque-core`.

#![cfg_attr(feature = "sexos", no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use burlesque_core::deck::{AspectRatio, Deck, Slide};
use burlesque_core::elements::{Paragraph, SlideElement, TextBox, TextRun};
use burlesque_core::layout::SlideLayout;
use burlesque_core::pdf_export::export_pdf;
use burlesque_core::presenter::{render_presenter_view, PresenterSession};
use burlesque_core::render::render_slide;
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

    // Widget IDs
    pub preview_id: Id,
    pub btn_add_slide: Id,
    pub btn_delete_slide: Id,
    pub btn_prev: Id,
    pub btn_next: Id,
    pub btn_layout_title: Id,
    pub btn_layout_body: Id,
    pub btn_layout_twocol: Id,
    pub btn_presenter: Id,
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
            "Weft Presentation Engine for SexOS",
        );
        slide.set_speaker_notes("Welcome to Burlesque presentations on SexOS.");

        let preview_w = 480;
        let preview_h = 270;
        let mut preview_buf = vec![0u32; (preview_w * preview_h) as usize];

        // Register commands and shortcuts
        for c in [
            Command::new("burlesque.export-pdf", "Export PDF").shortcut("Ctrl+E"),
            Command::new("burlesque.presenter", "Presenter View").shortcut("F5"),
            Command::new("burlesque.add-slide", "Add Slide").shortcut("Ctrl+N"),
            Command::new("burlesque.delete-slide", "Delete Slide"),
            Command::new("burlesque.prev", "Previous Slide").shortcut("Left"),
            Command::new("burlesque.next", "Next Slide").shortcut("Right"),
            Command::new("burlesque.layout-title", "Title Layout"),
            Command::new("burlesque.layout-body", "Title and Body Layout"),
            Command::new("burlesque.layout-twocol", "Two Column Layout"),
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
                    .separator()
                    .command("weft.close"),
                Menu::new("Slide")
                    .command("burlesque.add-slide")
                    .command("burlesque.delete-slide")
                    .separator()
                    .command("burlesque.prev")
                    .command("burlesque.next"),
                Menu::new("Layout")
                    .command("burlesque.layout-title")
                    .command("burlesque.layout-body")
                    .command("burlesque.layout-twocol"),
                Menu::new("View").command("burlesque.presenter"),
            ],
        );

        // Top Toolbar row 1: Slides & Navigation
        let row_nav = ui.row(root);
        let btn_prev = ui.button(row_nav, "Prev");
        let btn_next = ui.button(row_nav, "Next");

        ui.separator(row_nav);

        let btn_add_slide = ui.button(row_nav, "Add Slide");
        let btn_delete_slide = ui.button(row_nav, "Delete Slide");

        ui.separator(row_nav);

        let btn_presenter = ui.button(row_nav, "Presenter View");

        // Top Toolbar row 2: Layouts & Export
        let row_layouts = ui.row(root);
        let btn_layout_title = ui.button(row_layouts, "Title");
        let btn_layout_body = ui.button(row_layouts, "Body");
        let btn_layout_twocol = ui.button(row_layouts, "2-Col");

        ui.separator(row_layouts);

        let btn_export_pdf = ui.button(row_layouts, "Export PDF");

        // Main Workspace: Sidebar on left (slide list + text fields), Preview on right
        let workspace = ui.row(root);

        // Sidebar
        let sidebar = ui.column(workspace);
        ui.label(sidebar, "Slides:");
        let slide_list = ui.list(sidebar, &["1: Burlesque Presentations"]);
        ui.set_layout(slide_list, Layout::default().width(180.0).height(120.0));

        ui.label(sidebar, "Slide Title:");
        let field_title = ui.field(sidebar, "Burlesque Presentations", "Title");
        ui.set_layout(field_title, Layout::default().width(180.0));

        ui.label(sidebar, "Slide Body:");
        let field_body = ui.field(sidebar, "Weft Presentation Engine for SexOS", "Body");
        ui.set_layout(field_body, Layout::default().width(180.0));

        ui.label(sidebar, "Speaker Notes:");
        let field_notes = ui.field(
            sidebar,
            "Welcome to Burlesque presentations on SexOS.",
            "Notes",
        );
        ui.set_layout(field_notes, Layout::default().width(180.0));

        // Preview container
        let preview_col = ui.column(workspace);
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

        let status_label = ui.label(root, "Slide 1 of 1 | Presenter: Off | Aspect: 16:9");

        let controls = vec![
            ("add_slide", btn_add_slide),
            ("delete_slide", btn_delete_slide),
            ("prev", btn_prev),
            ("next", btn_next),
            ("layout_title", btn_layout_title),
            ("layout_body", btn_layout_body),
            ("layout_twocol", btn_layout_twocol),
            ("presenter", btn_presenter),
            ("export_pdf", btn_export_pdf),
        ];

        Self {
            deck,
            current_slide: 0,
            presenter_mode: false,
            presenter_session: None,
            preview_id,
            btn_add_slide,
            btn_delete_slide,
            btn_prev,
            btn_next,
            btn_layout_title,
            btn_layout_body,
            btn_layout_twocol,
            btn_presenter,
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

    pub fn refresh_preview(&mut self, ui: &mut Ui) {
        if self.presenter_mode {
            if let Some(ref mut session) = self.presenter_session {
                render_presenter_view(
                    session,
                    &mut self.preview_buf,
                    self.preview_w,
                    self.preview_h,
                );
            }
        } else if let Some(slide) = self.deck.get_slide(self.current_slide) {
            render_slide(
                slide,
                &self.deck.theme,
                self.deck.aspect_ratio,
                &mut self.preview_buf,
                self.preview_w,
                self.preview_h,
            );
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
        let text = format!(
            "Slide {} of {} | Presenter: {} | Aspect: 16:9",
            self.current_slide + 1,
            self.deck.slide_count(),
            if self.presenter_mode { "ON" } else { "Off" }
        );
        ui.set_text(self.status_label, &text);
    }

    pub fn sync_fields_to_current_slide(&mut self, ui: &mut Ui) {
        if let Some(slide) = self.deck.get_slide(self.current_slide) {
            ui.set_text(self.field_title, &slide.title);
            let body = slide_body_text(slide);
            ui.set_text(self.field_body, &body);
            ui.set_text(self.field_notes, &slide.speaker_notes);
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
            } else if *id == self.btn_presenter {
                command = Some("burlesque.presenter");
            } else if *id == self.btn_export_pdf {
                command = Some("burlesque.export-pdf");
            }
        }

        if let Response::Changed(id) | Response::Submitted(id) = response {
            if *id == self.field_title || *id == self.field_body || *id == self.field_notes {
                let title = ui.text_of(self.field_title).to_string();
                let body = ui.text_of(self.field_body).to_string();
                let notes = ui.text_of(self.field_notes).to_string();
                let aspect = self.deck.aspect_ratio;

                if let Some(slide) = self.deck.get_slide_mut(self.current_slide) {
                    slide.speaker_notes = notes;
                    sync_slide_elements(slide, aspect, &title, &body);
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
                let n = self.deck.slide_count() + 1;
                let aspect = self.deck.aspect_ratio;
                let slide = self.deck.add_slide(SlideLayout::TitleAndBody);
                let title = format!("Slide {}", n);
                sync_slide_elements(slide, aspect, &title, "Content for this slide");
                self.current_slide = self.deck.slide_count() - 1;
                if let Some(ref mut session) = self.presenter_session {
                    session.deck = self.deck.clone();
                    session.goto_slide(self.current_slide);
                }
                self.sync_fields_to_current_slide(ui);
                self.refresh_preview(ui);
                true
            }
            "burlesque.delete-slide" => {
                if self.deck.slide_count() > 1 {
                    self.deck.remove_slide(self.current_slide);
                    if self.current_slide >= self.deck.slide_count() {
                        self.current_slide = self.deck.slide_count().saturating_sub(1);
                    }
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
                if self.current_slide > 0 {
                    self.current_slide -= 1;
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
                if self.current_slide + 1 < self.deck.slide_count() {
                    self.current_slide += 1;
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
            let title_y = height * 0.30;
            let title_h = height * 0.22;
            let mut title_tb = TextBox::new(Rect::new(margin_x, title_y, content_w, title_h));
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
            let title_h = height * 0.14;
            let mut title_tb = TextBox::new(Rect::new(margin_x, margin_y, content_w, title_h));
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
            tb.paragraphs.push(Paragraph::from_text(title, 36.0));
            tb.paragraphs.push(Paragraph::from_text(body, 24.0));
            slide.add_element(SlideElement::TextBox(tb));
        }
    }
}

pub fn slide_body_text(slide: &Slide) -> String {
    let mut out = String::new();
    for (i, elem) in slide.elements.iter().enumerate() {
        if i == 0 && slide.elements.len() > 1 {
            continue;
        }
        if let SlideElement::TextBox(tb) = elem {
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
