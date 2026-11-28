// SPDX-License-Identifier: GPL-3.0-only
//! Velvet: Snags (minefield), Mingle (number merging), Twin Flames (matching).
//! Every visual control is a standard Weft widget; no custom widget drawing.
#![no_std]
extern crate alloc;
pub mod model;
use alloc::{format, string::String, vec::Vec};
use model::{Direction, Mingle, Snags, State, TwinFlames};
use weft::theme::{Color, Density, Mode, Palette};
use weft::{ButtonStyle, Command, Id, Layout, Response, TextStyle, Theme, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VelvetTheme {
    Noir,
    Sepia,
    Paper,
}

/// macOS Noir, Sepia, and Paper themes for Velvet with SexOS Rose (`#F43F5E`) accents.
pub fn theme_preset(preset: VelvetTheme, font_size: f32) -> Theme {
    let accent = Color::hex(0xF43F5E);
    let (mode, palette) = match preset {
        VelvetTheme::Noir => (
            Mode::Dark,
            Palette {
                window: Color::hex(0x0D1017),
                surface: Color::hex(0x151924),
                field: Color::hex(0x0F131D),
                control: Color::hex(0x1A202E),
                control_hover: Color::hex(0x232B3D),
                control_pressed: Color::hex(0x2C354A),
                border: Color::hex(0x252C3D),
                text: Color::hex(0xF1F5F9),
                text_dim: Color::hex(0x94A3B8),
                text_disabled: Color::hex(0x64748B),
                accent,
                on_accent: Color::hex(0xFFFFFF),
                selection: accent.with_alpha(0x45),
                focus: accent,
                danger: Color::hex(0xE11D48),
                on_danger: Color::hex(0xFFFFFF),
                shadow: Color::hex(0x000000).with_alpha(0x80),
                scrim: Color::hex(0x05070B).with_alpha(0x90),
            },
        ),
        VelvetTheme::Sepia => (
            Mode::Light,
            Palette {
                window: Color::hex(0xF4ECD8),
                surface: Color::hex(0xFAF4E4),
                field: Color::hex(0xEFE4CA),
                control: Color::hex(0xE8DCC2),
                control_hover: Color::hex(0xDDD0B2),
                control_pressed: Color::hex(0xD0C1A0),
                border: Color::hex(0xC9B997),
                text: Color::hex(0x2B2118),
                text_dim: Color::hex(0x6B5844),
                text_disabled: Color::hex(0x9C8B77),
                accent,
                on_accent: Color::hex(0xFFFFFF),
                selection: accent.with_alpha(0x40),
                focus: accent,
                danger: Color::hex(0xC62828),
                on_danger: Color::hex(0xFFFFFF),
                shadow: Color::hex(0x2B2118).with_alpha(0x30),
                scrim: Color::hex(0x2B2118).with_alpha(0x50),
            },
        ),
        VelvetTheme::Paper => (
            Mode::Light,
            Palette {
                window: Color::hex(0xF8FAFC),
                surface: Color::hex(0xFFFFFF),
                field: Color::hex(0xF1F5F9),
                control: Color::hex(0xE2E8F0),
                control_hover: Color::hex(0xCBD5E1),
                control_pressed: Color::hex(0x94A3B8),
                border: Color::hex(0xCBD5E1),
                text: Color::hex(0x0F172A),
                text_dim: Color::hex(0x475569),
                text_disabled: Color::hex(0x94A3B8),
                accent,
                on_accent: Color::hex(0xFFFFFF),
                selection: accent.with_alpha(0x38),
                focus: accent,
                danger: Color::hex(0xDC2626),
                on_danger: Color::hex(0xFFFFFF),
                shadow: Color::hex(0x0F172A).with_alpha(0x28),
                scrim: Color::hex(0x0F172A).with_alpha(0x48),
            },
        ),
    };
    Theme {
        mode,
        palette,
        scale: 1.0,
        density: Density::Comfortable,
        font_size,
        radius: 10.0,
        accent,
    }
}

pub fn noir_theme() -> Theme {
    theme_preset(VelvetTheme::Noir, 14.0)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
    Snags,
    Mingle,
    TwinFlames,
}
impl Game {
    pub fn name(self) -> &'static str {
        match self {
            Self::Snags => "Snags",
            Self::Mingle => "Mingle",
            Self::TwinFlames => "Twin Flames",
        }
    }
}
pub struct Velvet {
    pub game: Game,
    pub snags: Snags,
    pub mingle: Mingle,
    pub twins: TwinFlames,
    pub cells: Vec<Id>,
    pub choices: [Id; 3],
    pub new_game: Id,
    pub flag_mode: Id,
    pub status: Id,
    pub progress: Id,
    pub theme_btns: [(Id, VelvetTheme); 3],
    pub font_smaller: Id,
    pub font_larger: Id,
    pub active_theme: VelvetTheme,
    pub font_size: f32,
    board: Id,
    instructions: Id,
    arrows: Id,
    directions: [Id; 4],
    seed: u64,
}
impl Velvet {
    pub fn build(ui: &mut Ui, seed: u64) -> Self {
        for (id, label, shortcut) in [
            ("velvet.snags", "Play Snags", "Ctrl+1"),
            ("velvet.mingle", "Play Mingle", "Ctrl+2"),
            ("velvet.twins", "Play Twin Flames", "Ctrl+3"),
            ("velvet.new", "New Game", "Ctrl+N"),
            ("velvet.flag", "Toggle Flags", "Ctrl+F"),
            ("velvet.left", "Slide Left", "Left"),
            ("velvet.right", "Slide Right", "Right"),
            ("velvet.up", "Slide Up", "Up"),
            ("velvet.down", "Slide Down", "Down"),
        ] {
            ui.commands.add(Command::new(id, label).shortcut(shortcut));
        }
        let root = ui.root();
        ui.set_layout(root, Layout::column().gap(4.0).padding(10.0));
        let header = ui.row(root);
        ui.set_layout(header, Layout::row().gap(6.0).fill_width());
        let title = ui.text(header, "Velvet", TextStyle::Title);
        ui.set_layout(title, Layout::default().grow(1.0));
        let theme_btns = [
            (ui.button(header, "Noir"), VelvetTheme::Noir),
            (ui.button(header, "Sepia"), VelvetTheme::Sepia),
            (ui.button(header, "Paper"), VelvetTheme::Paper),
        ];
        for (id, preset) in theme_btns {
            ui.set_button_style(
                id,
                if preset == VelvetTheme::Noir {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Flat
                },
            );
        }
        let font_smaller = ui.button(header, "A-");
        ui.set_button_style(font_smaller, ButtonStyle::Flat);
        let font_larger = ui.button(header, "A+");
        ui.set_button_style(font_larger, ButtonStyle::Flat);

        let toolbar = ui.row(root);
        let choices = [
            ui.button(toolbar, "Snags"),
            ui.button(toolbar, "Mingle"),
            ui.button(toolbar, "Twin Flames"),
        ];
        let new_game = ui.button(toolbar, "New Game");
        ui.set_button_style(new_game, ButtonStyle::Primary);

        let info_card = ui.card(root, Layout::column().gap(3.0).fill_width());
        ui.set_layout(info_card, Layout::column().gap(3.0).padding(8.0).fill_width());
        let instructions = ui.paragraph(info_card, "");
        let status_row = ui.row(info_card);
        ui.set_layout(status_row, Layout::row().gap(8.0).fill_width());
        let status = ui.label(status_row, "");
        ui.set_layout(status, Layout::default().grow(1.0));
        let progress = ui.progress(status_row, Some(0.0));
        ui.set_layout(progress, Layout::default().width(140.0));

        let flag_mode = ui.checkbox(root, "Place flags instead of revealing", false);
        let board_card = ui.card(root, Layout::column().gap(4.0));
        ui.set_layout(board_card, Layout::column().gap(4.0).padding(8.0));
        let board = ui.group(board_card, Layout::grid(6).gap(4.0));
        let arrows = ui.row(root);
        let directions = [
            ui.button(arrows, "Left"),
            ui.button(arrows, "Up"),
            ui.button(arrows, "Down"),
            ui.button(arrows, "Right"),
        ];
        let mut v = Self {
            game: Game::Snags,
            snags: Snags::new(seed),
            mingle: Mingle::new(seed),
            twins: TwinFlames::new(seed),
            cells: Vec::new(),
            choices,
            new_game,
            flag_mode,
            status,
            progress,
            theme_btns,
            font_smaller,
            font_larger,
            active_theme: VelvetTheme::Noir,
            font_size: 14.0,
            board,
            instructions,
            arrows,
            directions,
            seed,
        };
        v.rebuild(ui);
        v
    }
    fn rebuild(&mut self, ui: &mut Ui) {
        ui.clear(self.board);
        self.cells.clear();
        let (cols, n) = if self.game == Game::Snags {
            (6, 36)
        } else {
            (4, 16)
        };
        ui.set_layout(self.board, Layout::grid(cols).gap(4.0));
        for i in 0..n {
            let id = ui.button(self.board, "");
            ui.set_layout(id, Layout::default().width(48.0).height(30.0));
            ui.set_label(id, &format!("Square {}", i + 1));
            self.cells.push(id);
        }
        ui.set_visible(self.flag_mode, self.game == Game::Snags);
        ui.set_visible(self.arrows, self.game == Game::Mingle);
        for (id, g) in [
            (self.choices[0], Game::Snags),
            (self.choices[1], Game::Mingle),
            (self.choices[2], Game::TwinFlames),
        ] {
            ui.set_button_style(
                id,
                if g == self.game {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }
        for id in ["velvet.left", "velvet.right", "velvet.up", "velvet.down"] {
            ui.commands.set_enabled(id, self.game == Game::Mingle);
        }
        let help=match self.game {Game::Snags=>"30 safe squares, 6 snags. First click safe. Ctrl+F flags. Tab/Enter selects.",Game::Mingle=>"Slide equal numbers together to make 2048. Use arrow keys or direction buttons. Each move adds a tile.",Game::TwinFlames=>"Find eight matching pairs. A mismatch stays visible until you choose the next square. Tab then Enter flips a card."};
        ui.set_text(self.instructions, help);
        self.refresh(ui);
    }
    pub fn refresh(&self, ui: &mut Ui) {
        for (i, id) in self.cells.iter().enumerate() {
            let text = match self.game {
                Game::Snags => {
                    if self.snags.state == State::Lost && self.snags.is_mine(i) {
                        String::from("X")
                    } else if self.snags.open[i] {
                        let n = self.snags.adjacent(i);
                        if n == 0 {
                            String::from(".")
                        } else {
                            format!("{n}")
                        }
                    } else if self.snags.flags[i] {
                        String::from("F")
                    } else {
                        String::from("?")
                    }
                }
                Game::Mingle => {
                    if self.mingle.cells[i] == 0 {
                        String::from(".")
                    } else {
                        format!("{}", self.mingle.cells[i])
                    }
                }
                Game::TwinFlames => {
                    if self.twins.visible(i) {
                        format!("{}", (b'A' + self.twins.cards[i]) as char)
                    } else {
                        String::from("?")
                    }
                }
            };
            ui.set_text(*id, &text);
            // Labels contain only public board information; hidden cards/mines
            // never enter accessibility labels, tooltips or serial records.
            ui.set_label(*id, &format!("Square {}: {}", i + 1, text));
        }
        let text = match self.game {
            Game::Snags => format!(
                "Snags — {:?} | Safe: {}/30 | Flags: {}",
                self.snags.state,
                self.snags.open.iter().filter(|v| **v).count(),
                self.snags.flags.iter().filter(|v| **v).count()
            ),
            Game::Mingle => format!(
                "Mingle — {:?} | Score: {} | Moves: {}",
                self.mingle.state, self.mingle.score, self.mingle.moves
            ),
            Game::TwinFlames => format!(
                "Twin Flames — {:?} | Pairs: {}/8 | Turns: {}",
                self.twins.state,
                self.twins.matched.iter().filter(|v| **v).count() / 2,
                self.twins.turns
            ),
        };
        ui.set_text(self.status, &text);
        let pct = match self.game {
            Game::Snags => self.snags.open.iter().filter(|v| **v).count() as f32 / 30.0,
            Game::Mingle => {
                let best = self.mingle.cells.iter().copied().max().unwrap_or(0);
                if best <= 2 { 0.0 } else { (best.ilog2() as f32 - 1.0) / 10.0 }
            }
            Game::TwinFlames => (self.twins.matched.iter().filter(|v| **v).count() / 2) as f32 / 8.0,
        };
        ui.set_value(self.progress, pct.clamp(0.0, 1.0));
    }
    fn apply_theme(&mut self, ui: &mut Ui) {
        ui.set_theme(theme_preset(self.active_theme, self.font_size));
        for (id, preset) in self.theme_btns {
            ui.set_button_style(
                id,
                if preset == self.active_theme {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Flat
                },
            );
        }
    }
    pub fn on(&mut self, ui: &mut Ui, response: &Response) -> bool {
        let mut command = None;
        if let Response::Clicked(id) = response {
            if let Some((_, preset)) = self.theme_btns.iter().find(|(b, _)| b == id) {
                self.active_theme = *preset;
                self.apply_theme(ui);
                return true;
            } else if *id == self.font_smaller {
                self.font_size = (self.font_size - 1.0).max(11.0);
                self.apply_theme(ui);
                return true;
            } else if *id == self.font_larger {
                self.font_size = (self.font_size + 1.0).min(18.0);
                self.apply_theme(ui);
                return true;
            } else if let Some(i) = self.choices.iter().position(|b| b == id) {
                command = Some(["velvet.snags", "velvet.mingle", "velvet.twins"][i]);
            } else if self.game == Game::Mingle && self.directions.contains(id) {
                command = Some(
                    ["velvet.left", "velvet.up", "velvet.down", "velvet.right"]
                        [self.directions.iter().position(|b| b == id).unwrap()],
                );
            } else if *id == self.new_game {
                command = Some("velvet.new");
            } else if let Some(i) = self.cells.iter().position(|b| b == id) {
                match self.game {
                    Game::Snags => {
                        if ui.is_checked(self.flag_mode) {
                            self.snags.flag(i)
                        } else {
                            self.snags.reveal(i)
                        }
                    }
                    Game::Mingle => return false,
                    Game::TwinFlames => self.twins.pick(i),
                }
                self.refresh(ui);
                return true;
            }
        }
        if let Response::Command(id) = response {
            command = Some(id.as_str());
        }
        let Some(command) = command else {
            return false;
        };
        match command {
            "velvet.snags" | "velvet.mingle" | "velvet.twins" => {
                self.game = match command {
                    "velvet.snags" => Game::Snags,
                    "velvet.mingle" => Game::Mingle,
                    _ => Game::TwinFlames,
                };
                self.rebuild(ui);
            }
            "velvet.new" => {
                self.seed = self.seed.wrapping_add(0x9e3779b97f4a7c15);
                match self.game {
                    Game::Snags => {
                        self.snags = Snags::new(self.seed);
                        ui.set_checked(self.flag_mode, false);
                    }
                    Game::Mingle => self.mingle = Mingle::new(self.seed),
                    Game::TwinFlames => self.twins = TwinFlames::new(self.seed),
                };
                self.refresh(ui);
            }
            "velvet.flag" if self.game == Game::Snags => {
                ui.set_checked(self.flag_mode, !ui.is_checked(self.flag_mode));
            }
            "velvet.left" | "velvet.right" | "velvet.up" | "velvet.down"
                if self.game == Game::Mingle =>
            {
                let d = match command {
                    "velvet.left" => Direction::Left,
                    "velvet.right" => Direction::Right,
                    "velvet.up" => Direction::Up,
                    _ => Direction::Down,
                };
                self.mingle.slide(d);
                self.refresh(ui);
            }
            _ => return false,
        }
        true
    }
}
