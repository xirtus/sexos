// SPDX-License-Identifier: GPL-3.0-only
//! Abacus, the SexOS calculator, built on Weft: a keypad, an expression line you can
//! type into, a history to reuse answers from, menus, the command palette and
//! keyboard shortcuts. The window code lives here so tests drive it on the host.
#![no_std]
extern crate alloc;

pub mod calc;
pub mod programmer;
pub mod units;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Scientific,
    Programmer,
    Convert,
}

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use calc::Angle;
use weft::menu::{Command, Menu};
use weft::widget::{Column, Id, Layout, Rows, TextStyle};
use weft::{Align, ButtonStyle, Response, Ui};

/// The keypad, row by row: what each key shows.
pub const KEYS: [[&str; 5]; 6] = [
    ["C", "⌫", "(", ")", "%"],
    ["sin", "cos", "tan", "√", "^"],
    ["7", "8", "9", "÷", "π"],
    ["4", "5", "6", "×", "ln"],
    ["1", "2", "3", "−", "log"],
    ["0", ".", "ans", "+", "="],
];

pub struct Abacus {
    pub mode: Mode,
    pub word_bits: u8,
    pub radix: u32,
    pub integer_ans: u64,
    pub modes: [(Id, Mode); 3],
    pub extra_keys: Vec<(Id, &'static str)>,
    pub help: Id,
    history_context: Vec<(Mode, u8, u32)>,
    pub result: Id,
    pub expr: Id,
    pub history: Id,
    pub keys: Vec<(Id, &'static str)>,
    pub ans: f64,
    pub angle: Angle,
    pub past: Vec<(String, String)>,
}

/// Something worth a line in the serial record.
pub enum Note {
    Result { expr: String, value: String },
    Error(String),
}

impl Abacus {
    pub fn build(ui: &mut Ui) -> Abacus {
        for c in [
            Command::new("abacus.scientific", "Scientific Mode").checked(true),
            Command::new("abacus.programmer", "Programmer Mode").checked(false),
            Command::new("abacus.convert", "Unit Conversion").checked(false),
            Command::new("abacus.bits8", "8-bit Words"),
            Command::new("abacus.bits16", "16-bit Words"),
            Command::new("abacus.bits32", "32-bit Words"),
            Command::new("abacus.bits64", "64-bit Words").checked(true),
            Command::new("abacus.hex", "Hexadecimal Output"),
            Command::new("abacus.decimal", "Decimal Output").checked(true),
            Command::new("abacus.binary", "Binary Output"),
            Command::new("abacus.octal", "Octal Output"),
            Command::new("abacus.copy", "Copy Result")
                .shortcut("Ctrl+Shift+C")
                .icon("copy"),
            Command::new("abacus.clear", "Clear")
                .shortcut("Escape")
                .icon("close")
                .keywords("reset all clear"),
            Command::new("abacus.recall-last", "Recall Last Calculation").shortcut("Alt+Up"),
            Command::new("abacus.clear-history", "Clear History").icon("trash"),
            Command::new("abacus.degrees", "Degrees")
                .checked(true)
                .keywords("angle"),
            Command::new("abacus.radians", "Radians")
                .checked(false)
                .keywords("angle"),
            Command::new("abacus.about", "About Abacus").icon("info"),
        ] {
            ui.commands.add(c);
        }
        let root = ui.root();
        ui.set_layout(root, Layout::column());
        ui.menubar(
            root,
            vec![
                Menu::new("Edit")
                    .command("abacus.copy")
                    .command("weft.paste")
                    .separator()
                    .command("abacus.clear")
                    .command("abacus.clear-history")
                    .command("abacus.recall-last"),
                Menu::new("Mode")
                    .command("abacus.scientific")
                    .command("abacus.programmer")
                    .command("abacus.convert"),
                Menu::new("Words")
                    .command("abacus.bits8")
                    .command("abacus.bits16")
                    .command("abacus.bits32")
                    .command("abacus.bits64")
                    .separator()
                    .command("abacus.decimal")
                    .command("abacus.hex")
                    .command("abacus.binary")
                    .command("abacus.octal"),
                Menu::new("View")
                    .command("abacus.degrees")
                    .command("abacus.radians")
                    .separator()
                    .command("weft.palette"),
                Menu::new("Help").command("abacus.about"),
            ],
        );
        let body = ui.group(root, Layout::column().gap(10.0).padding(12.0).fill());
        let mode_row = ui.row(body);
        let modes = [
            (ui.button(mode_row, "Scientific"), Mode::Scientific),
            (ui.button(mode_row, "Programmer"), Mode::Programmer),
            (ui.button(mode_row, "Convert"), Mode::Convert),
        ];
        let help = ui.paragraph(
            body,
            "Scientific: functions, factorial, degrees/radians and ans.",
        );
        let result = ui.paragraph(body, "0");
        ui.set_align(result, Align::End);
        ui.set_layout(result, Layout::default().fill_width().min_height(48.0));
        ui.set_label(result, "Result");
        let expr = ui.field(body, "", "Type a sum, like 12 + 30");
        ui.set_layout(expr, Layout::default().fill_width());
        ui.set_label(expr, "Expression");
        ui.set_max_length(expr, Some(512));
        let extra_pad = ui.group(body, Layout::grid(7).gap(4.0));
        let mut extra_keys = Vec::new();
        for k in [
            "0x", "A", "B", "C", "D", "E", "F", "0b", "&", "|", "^", "~", "<<", ">>",
        ] {
            let id = ui.button(extra_pad, k);
            ui.set_visible(id, false);
            extra_keys.push((id, k));
        }
        let split = ui.split(body, false, 0.62);
        let pad = ui.group(split, Layout::grid(5).gap(6.0));
        let mut keys = Vec::new();
        for row in KEYS {
            for k in row {
                let b = ui.button(pad, k);
                ui.set_layout(b, Layout::default().fill_width().min_height(40.0));
                match k {
                    "=" => ui.set_button_style(b, ButtonStyle::Primary),
                    "C" => ui.set_button_style(b, ButtonStyle::Danger),
                    "⌫" => ui.set_label(b, "Backspace"),
                    _ => {}
                }
                keys.push((b, k));
            }
        }
        let side = ui.column(split);
        ui.text(side, "History", TextStyle::Caption);
        let history = ui.list_with(
            side,
            alloc::boxed::Box::new(Rows::default()),
            vec![
                Column::new("Sum", 150.0),
                Column::new("Answer", 100.0).right(),
            ],
        );
        ui.set_tooltip(history, "Double-click or Enter to reuse an answer");
        ui.focus(Some(expr));
        let mut calc = Abacus {
            result,
            expr,
            history,
            keys,
            ans: 0.0,
            angle: Angle::Degrees,
            past: Vec::new(),
            mode: Mode::Scientific,
            word_bits: 64,
            radix: 10,
            integer_ans: 0,
            modes,
            extra_keys,
            help,
            history_context: Vec::new(),
        };
        calc.set_mode(ui, Mode::Scientific);
        calc
    }

    fn put(&mut self, ui: &mut Ui, text: &str) {
        let mut t = String::from(ui.text_of(self.expr));
        t.push_str(text);
        ui.set_text(self.expr, &t);
        ui.focus(Some(self.expr));
    }

    /// Works out the expression line.
    pub fn equals(&mut self, ui: &mut Ui) -> Note {
        // Typing goes on in the expression line whatever was clicked.
        ui.focus(Some(self.expr));
        let expr = String::from(ui.text_of(self.expr).trim());
        if expr.is_empty() {
            return Note::Error("empty".into());
        }
        let evaluated: Result<String, String> = match self.mode {
            Mode::Scientific => calc::eval(&expr, self.ans, self.angle)
                .map(|v| {
                    self.ans = v;
                    calc::format(v)
                })
                .map_err(|e| match e {
                    calc::Error::DivideByZero => "Can't divide by zero".into(),
                    calc::Error::Domain => "Out of range".into(),
                    calc::Error::Unknown(n) => format!("Unknown: {n}"),
                    calc::Error::Syntax(_) => "Check the sum".into(),
                }),
            Mode::Programmer => programmer::eval(&expr, self.integer_ans, self.word_bits)
                .map(|v| {
                    self.integer_ans = v;
                    programmer::format(v, self.radix)
                })
                .map_err(|e| format!("Programmer: {e:?}")),
            Mode::Convert => units::eval(&expr).map_err(|e| format!("Units: {e:?}")),
        };
        match evaluated {
            Ok(value) => {
                ui.set_text(self.result, &value);
                ui.set_color(self.result, None);
                self.past.insert(0, (expr.clone(), value.clone()));
                self.past.truncate(500);
                self.history_context
                    .insert(0, (self.mode, self.word_bits, self.radix));
                self.history_context.truncate(500);
                ui.set_rows(
                    self.history,
                    Rows(
                        self.past
                            .iter()
                            .map(|(e, v)| vec![e.clone(), format!("= {v}")])
                            .collect(),
                    ),
                );
                ui.set_text(self.expr, "");
                Note::Result { expr, value }
            }
            Err(msg) => {
                ui.set_text(self.result, &msg);
                let danger = ui.theme().palette.danger;
                ui.set_color(self.result, Some(danger));
                Note::Error(msg)
            }
        }
    }

    pub fn set_mode(&mut self, ui: &mut Ui, mode: Mode) {
        self.mode = mode;
        for (command, m) in [
            ("abacus.scientific", Mode::Scientific),
            ("abacus.programmer", Mode::Programmer),
            ("abacus.convert", Mode::Convert),
        ] {
            ui.commands.set_checked(command, m == mode);
        }
        for bits in [8, 16, 32, 64] {
            ui.commands
                .set_checked(&format!("abacus.bits{bits}"), bits == self.word_bits);
        }
        for (id, radix) in [
            ("abacus.hex", 16),
            ("abacus.decimal", 10),
            ("abacus.binary", 2),
            ("abacus.octal", 8),
        ] {
            ui.commands.set_checked(id, self.radix == radix);
        }
        for (id, m) in self.modes {
            ui.set_button_style(
                id,
                if m == mode {
                    ButtonStyle::Primary
                } else {
                    ButtonStyle::Normal
                },
            );
        }
        for (id, key) in &self.keys {
            ui.set_enabled(
                *id,
                mode != Mode::Programmer
                    || !matches!(*key, "sin" | "cos" | "tan" | "√" | "π" | "ln" | "log" | "."),
            );
        }
        for (id, _) in &self.extra_keys {
            ui.set_visible(*id, mode == Mode::Programmer);
        }
        ui.set_text(self.help,&match mode{Mode::Scientific=>"Scientific: functions, factorial, degrees/radians and ans.".into(),Mode::Programmer=>format!("{}-bit unsigned wrapping arithmetic. Literals 0x/0b/0o; ^ means XOR. Output radix {} (Words menu).",self.word_bits,self.radix),Mode::Convert=>"Type: 32 F to C. Units: m/km/cm/mm/in/ft/mi; kg/g/lb/oz; s/ms/min/h/day; K/C/F; B/kB/MB/GB/KiB/MiB/GiB.".into()});
        ui.focus(Some(self.expr));
    }

    fn recall(&mut self, ui: &mut Ui, row: usize) {
        if let Some((expr, value)) = self.past.get(row).cloned() {
            let (mode, bits, radix) =
                self.history_context
                    .get(row)
                    .copied()
                    .unwrap_or((Mode::Scientific, 64, 10));
            self.word_bits = bits;
            self.radix = radix;
            self.set_mode(ui, mode);
            ui.set_text(self.expr, "");
            self.put(ui, if mode == Mode::Convert { &expr } else { &value });
        }
    }

    /// Acts on one response; returns a note for the record, if any.
    pub fn on(&mut self, ui: &mut Ui, r: &Response) -> Option<Note> {
        match r {
            Response::Submitted(id) if *id == self.expr => return Some(self.equals(ui)),
            Response::Clicked(id) => {
                if let Some((_, mode)) = self.modes.iter().find(|(b, _)| b == id) {
                    let mode = *mode;
                    self.set_mode(ui, mode);
                    return None;
                }
                if let Some((_, key)) = self.extra_keys.iter().find(|(b, _)| b == id) {
                    let key = *key;
                    self.put(ui, key);
                    return None;
                }
                let k = self.keys.iter().find(|(b, _)| b == id)?.1;
                match k {
                    "=" => return Some(self.equals(ui)),
                    "C" => {
                        ui.set_text(self.expr, "");
                        ui.set_text(self.result, "0");
                        ui.set_color(self.result, None);
                        ui.focus(Some(self.expr));
                    }
                    "⌫" => {
                        let mut t = String::from(ui.text_of(self.expr));
                        t.pop();
                        ui.set_text(self.expr, &t);
                        ui.focus(Some(self.expr));
                    }
                    "sin" | "cos" | "tan" | "ln" | "log" => {
                        let s = format!("{k}(");
                        self.put(ui, &s);
                    }
                    "√" => self.put(ui, "√("),
                    "ans" => self.put(ui, "ans"),
                    "−" => self.put(ui, "−"),
                    k => self.put(ui, k),
                }
            }
            Response::Activated(id, row) if *id == self.history => {
                self.recall(ui, *row);
            }
            Response::Command(c) => match c.as_str() {
                "abacus.recall-last" => self.recall(ui, 0),
                "abacus.scientific" => self.set_mode(ui, Mode::Scientific),
                "abacus.programmer" => self.set_mode(ui, Mode::Programmer),
                "abacus.convert" => self.set_mode(ui, Mode::Convert),
                "abacus.bits8" | "abacus.bits16" | "abacus.bits32" | "abacus.bits64" => {
                    self.word_bits = c.trim_start_matches("abacus.bits").parse().unwrap_or(64);
                    self.integer_ans &= programmer::mask(self.word_bits).unwrap();
                    for bits in [8, 16, 32, 64] {
                        ui.commands
                            .set_checked(&format!("abacus.bits{bits}"), bits == self.word_bits);
                    }
                    self.set_mode(ui, Mode::Programmer);
                }
                "abacus.hex" | "abacus.decimal" | "abacus.binary" | "abacus.octal" => {
                    self.radix = match c.as_str() {
                        "abacus.hex" => 16,
                        "abacus.binary" => 2,
                        "abacus.octal" => 8,
                        _ => 10,
                    };
                    for (id, radix) in [
                        ("abacus.hex", 16),
                        ("abacus.decimal", 10),
                        ("abacus.binary", 2),
                        ("abacus.octal", 8),
                    ] {
                        ui.commands.set_checked(id, self.radix == radix);
                    }
                    self.set_mode(ui, Mode::Programmer);
                }
                "abacus.clear" => {
                    ui.set_text(self.expr, "");
                    ui.set_text(self.result, "0");
                    ui.set_color(self.result, None);
                }
                "abacus.clear-history" => {
                    self.past.clear();
                    self.history_context.clear();
                    ui.set_rows(self.history, Rows::default());
                }
                "abacus.degrees" | "abacus.radians" => {
                    self.angle = if c == "abacus.degrees" {
                        Angle::Degrees
                    } else {
                        Angle::Radians
                    };
                    ui.commands
                        .set_checked("abacus.degrees", self.angle == Angle::Degrees);
                    ui.commands
                        .set_checked("abacus.radians", self.angle == Angle::Radians);
                    ui.toast(
                        if self.angle == Angle::Degrees {
                            "Angles in degrees"
                        } else {
                            "Angles in radians"
                        },
                        None,
                        2000,
                    );
                }
                "abacus.about" => {
                    ui.message(
                        "Abacus",
                        "The SexOS calculator, built on Weft.",
                        &["OK"],
                        0,
                        0,
                    );
                }
                _ => {}
            },
            _ => {}
        }
        None
    }

    /// Text for Copy Result.
    pub fn result_text<'a>(&self, ui: &'a Ui) -> &'a str {
        ui.text_of(self.result)
    }
}
