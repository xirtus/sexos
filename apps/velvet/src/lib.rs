// SPDX-License-Identifier: GPL-3.0-only
//! Velvet: Snags (minefield), Mingle (number merging), Twin Flames (matching).
//! Every visual control is a standard Weft widget; no custom widget drawing.
#![no_std]
extern crate alloc;
pub mod model;
use alloc::{format, string::String, vec::Vec};
use model::{Direction, Mingle, Snags, State, TwinFlames};
use weft::{Command, Id, Layout, Response, TextStyle, Ui};
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
        ui.set_layout(root, Layout::column().gap(4.0).padding(12.0));
        ui.text(root, "Velvet", TextStyle::Title);
        let toolbar = ui.row(root);
        let choices = [
            ui.button(toolbar, "Snags"),
            ui.button(toolbar, "Mingle"),
            ui.button(toolbar, "Twin Flames"),
        ];
        let new_game = ui.button(toolbar, "New Game");
        let instructions = ui.paragraph(root, "");
        let status = ui.label(root, "");
        let flag_mode = ui.checkbox(root, "Place flags instead of revealing", false);
        let board = ui.group(root, Layout::grid(6).gap(4.0));
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
    }
    pub fn on(&mut self, ui: &mut Ui, response: &Response) -> bool {
        let mut command = None;
        if let Response::Clicked(id) = response {
            if let Some(i) = self.choices.iter().position(|b| b == id) {
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
