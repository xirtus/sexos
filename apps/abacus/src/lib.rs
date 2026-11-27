// SPDX-License-Identifier: GPL-3.0-only
//! Abacus, the SexOS calculator, built on Weft: a keypad, an expression line you can
//! type into, scientific, programmer and unit-conversion modes, a memory (MC MR M+
//! M−), a history to reuse answers from (Up and Down pick one, Enter puts it in),
//! degrees or radians (Ctrl+D switches), menus, the command palette and keyboard
//! shortcuts. After an answer, a sum that starts with an operator carries on from
//! it. The window code lives here so tests drive it on the host.
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
use weft::theme::{Color, Density, Palette, Theme};
use weft::widget::{Column, Id, Layout, Rows, TextStyle};
use weft::{Align, ButtonStyle, Response, Ui};

/// macOS Noir palette for Abacus: deep obsidian canvas (`#0D1017`), elevated
/// card surfaces (`#151924`), hairline borders (`#252C3D`), and SexOS Rose (`#F43F5E`).
pub fn noir_theme() -> Theme {
    let accent = Color::hex(0xF43F5E);
    let palette = Palette {
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
    };
    Theme {
        mode: weft::theme::Mode::Dark,
        palette,
        scale: 1.0,
        density: Density::Comfortable,
        font_size: 14.0,
        radius: 10.0,
        accent,
    }
}

/// The keypad, row by row: what each key shows.
pub const KEYS: [[&str; 5]; 7] = [
    ["MC", "MR", "M+", "M−", "±"],
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
    pub memory: f64,
    /// The small line above the result that shows the memory.
    pub mem_line: Id,
    /// Small expression-history line above the large result readout.
    pub prev_expr: Id,
    /// Live Hex/Dec/Bin readout in Programmer mode.
    pub base_line: Id,
    /// Quick DEG/RAD pill button.
    pub angle_pill: Id,
    /// Quick Copy button on the display card.
    pub copy_btn: Id,
    /// The past answer picked with Up and Down, 0 the newest.
    pub picked: Option<usize>,
    /// The result on screen was just worked out: a sum that starts with an operator
    /// carries on from it.
    pub fresh: bool,
}

/// Functions and constants the Functions menu puts in the expression line.
pub const INSERTS: [(&str, &str, &str); 9] = [
    ("abacus.fn.asin", "Arcsine", "asin("),
    ("abacus.fn.acos", "Arccosine", "acos("),
    ("abacus.fn.atan", "Arctangent", "atan("),
    ("abacus.fn.exp", "Exponential (eˣ)", "exp("),
    ("abacus.fn.abs", "Absolute Value", "abs("),
    ("abacus.fn.fact", "Factorial (n!)", "!"),
    ("abacus.fn.mod", "Remainder (mod)", " mod "),
    ("abacus.fn.e", "Euler's Number (e)", "e"),
    ("abacus.fn.mem", "Memory Value", "mem"),
];

/// Something worth a line in the serial record.
pub enum Note {
    Result { expr: String, value: String },
    Error(String),
    /// The memory changed to this value.
    Memory(String),
    /// Past answer `index` (0 the newest) went into the expression line.
    Recall { index: usize, value: String },
    /// Angles are now in degrees (`true`) or radians.
    Angle(bool),
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
            Command::new("abacus.recall-last", "Recall Last Calculation").shortcut("Ctrl+Up"),
            Command::new("abacus.clear-history", "Clear History").icon("trash"),
            Command::new("abacus.degrees", "Degrees")
                .checked(true)
                .keywords("angle"),
            Command::new("abacus.radians", "Radians")
                .checked(false)
                .keywords("angle"),
            Command::new("abacus.angle", "Switch Degrees and Radians").shortcut("Ctrl+D").keywords("angle deg rad"),
            Command::new("abacus.square", "Square (x²)").keywords("power"),
            Command::new("abacus.recip", "Reciprocal (1/x)").keywords("inverse"),
            Command::new("abacus.negate", "Change Sign (±)").keywords("negative minus"),
            Command::new("abacus.mem-clear", "Memory Clear").keywords("mc"),
            Command::new("abacus.mem-recall", "Memory Recall").keywords("mr"),
            Command::new("abacus.mem-add", "Memory Add").keywords("m+"),
            Command::new("abacus.mem-sub", "Memory Subtract").keywords("m-"),
            Command::new("abacus.older", "Older Answer").shortcut("Up").hidden(),
            Command::new("abacus.newer", "Newer Answer").shortcut("Down").hidden(),
            Command::new("abacus.about", "About Abacus").icon("info"),
        ] {
            ui.commands.add(c);
        }
        for (id, label, _) in INSERTS { ui.commands.add(Command::new(id, label).keywords("function insert")); }
        let mut functions = Menu::new("Functions").command("abacus.square").command("abacus.recip").command("abacus.negate").separator();
        for (id, _, _) in INSERTS { functions = functions.command(id); }
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
                functions,
                Menu::new("Memory").command("abacus.mem-clear").command("abacus.mem-recall").command("abacus.mem-add").command("abacus.mem-sub"),
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
                    .command("abacus.angle")
                    .separator()
                    .command("weft.palette"),
                Menu::new("Help").command("abacus.about"),
            ],
        );
        let scroll = ui.scroll(root);
        ui.set_layout(scroll, Layout::column().fill());
        let body = ui.group(
            scroll,
            Layout::column()
                .gap(8.0)
                .padding(10.0)
                .fill_width()
                .shrink(0.0),
        );
        let mode_row = ui.row(body);
        let modes = [
            (ui.button(mode_row, "Scientific"), Mode::Scientific),
            (ui.button(mode_row, "Hex/Bin"), Mode::Programmer),
            (ui.button(mode_row, "Convert"), Mode::Convert),
        ];
        let angle_pill = ui.button(mode_row, "DEG");
        ui.set_button_style(angle_pill, ButtonStyle::Flat);
        ui.set_tooltip(angle_pill, "Toggle Degrees / Radians (Ctrl+D)");
        let copy_btn = ui.button(mode_row, "Copy");
        ui.set_button_style(copy_btn, ButtonStyle::Flat);
        ui.set_tooltip(copy_btn, "Copy result to clipboard (Ctrl+Shift+C)");

        let display_card = ui.card(body, Layout::column().gap(3.0).fill_width());
        ui.set_layout(display_card, Layout::column().gap(3.0).padding(8.0).fill_width());
        let status_row = ui.row(display_card);
        ui.set_layout(status_row, Layout::row().gap(8.0).fill_width());
        let prev_expr = ui.text(status_row, "", TextStyle::Caption);
        ui.set_layout(prev_expr, Layout::default().grow(1.0));
        ui.set_label(prev_expr, "Previous Expression");
        let mem_line = ui.text(status_row, "", TextStyle::Caption);
        ui.set_align(mem_line, Align::End);
        ui.set_label(mem_line, "Memory");

        let result = ui.text(display_card, "0", TextStyle::Display);
        ui.set_align(result, Align::End);
        ui.set_layout(result, Layout::default().fill_width().min_height(38.0));
        ui.set_label(result, "Result");

        let base_line = ui.text(display_card, "", TextStyle::Mono);
        ui.set_align(base_line, Align::End);
        ui.set_layout(base_line, Layout::default().fill_width());
        ui.set_color(base_line, Some(ui.theme().palette.text_dim));
        ui.set_visible(base_line, false);

        let expr = ui.field(display_card, "", "Type an expression, like 12 + 30");
        ui.set_layout(expr, Layout::default().fill_width());
        ui.set_label(expr, "Expression");
        ui.set_max_length(expr, Some(512));

        let help = ui.text(
            display_card,
            "Scientific: functions, factorial, degrees/radians and ans.",
            TextStyle::Caption,
        );
        ui.set_layout(help, Layout::default().fill_width());

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
        let pad_card = ui.card(split, Layout::column().gap(4.0).fill_width());
        ui.set_layout(pad_card, Layout::column().gap(4.0).padding(6.0).fill_width());
        let pad = ui.group(pad_card, Layout::grid(5).gap(5.0).fill_width());
        let mut keys = Vec::new();
        for row in KEYS {
            for k in row {
                let b = ui.button(pad, k);
                ui.set_layout(b, Layout::default().fill_width().min_height(34.0));
                match k {
                    "=" => ui.set_button_style(b, ButtonStyle::Primary),
                    "C" => ui.set_button_style(b, ButtonStyle::Danger),
                    "+" | "−" | "×" | "÷" => ui.set_button_style(b, ButtonStyle::Normal),
                    "⌫" => ui.set_label(b, "Backspace"),
                    "MC" => ui.set_label(b, "Memory clear"),
                    "MR" => ui.set_label(b, "Memory recall"),
                    "M+" => ui.set_label(b, "Memory add"),
                    "M−" => ui.set_label(b, "Memory subtract"),
                    "±" => ui.set_label(b, "Change sign"),
                    _ => {}
                }
                keys.push((b, k));
            }
        }
        let side = ui.card(split, Layout::column().gap(6.0));
        ui.text(side, "History", TextStyle::Heading);
        let history = ui.list_with(
            side,
            alloc::boxed::Box::new(Rows::default()),
            vec![
                Column::new("Sum", 150.0),
                Column::new("Answer", 100.0).right(),
            ],
        );
        ui.set_tooltip(history, "Up and Down pick an answer, Enter puts it in");
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
            memory: 0.0,
            mem_line,
            prev_expr,
            base_line,
            angle_pill,
            copy_btn,
            picked: None,
            fresh: false,
        };
        calc.set_mode(ui, Mode::Scientific);
        calc
    }

    fn put(&mut self, ui: &mut Ui, text: &str) {
        let mut t = String::from(ui.text_of(self.expr));
        t.push_str(text);
        ui.set_text(self.expr, &t);
        ui.focus(Some(self.expr));
        self.unpick(ui);
    }

    fn unpick(&mut self, ui: &mut Ui) {
        if self.picked.take().is_some() { ui.select_row(self.history, None); }
    }

    /// The expression line in brackets with something around it, or `ans` when empty.
    fn wrap(&mut self, ui: &mut Ui, before: &str, after: &str) {
        let t = String::from(ui.text_of(self.expr).trim());
        let inner = if t.is_empty() { String::from("ans") } else { t };
        ui.set_text(self.expr, &format!("{before}({inner}){after}"));
        ui.focus(Some(self.expr));
        self.fresh = false;
        self.unpick(ui);
    }

    /// What the expression line works out to, or the last answer when it is empty.
    fn value_now(&self, ui: &Ui) -> Option<f64> {
        let t = ui.text_of(self.expr).trim();
        if t.is_empty() { return Some(self.ans); }
        calc::eval_with(t, self.ans, self.memory, self.angle).ok()
    }

    fn show_memory(&mut self, ui: &mut Ui) -> Note {
        let v = calc::format(self.memory);
        ui.set_text(self.mem_line, &if self.memory == 0.0 { String::new() } else { format!("M = {v}") });
        Note::Memory(v)
    }

    /// Clears the expression line and the result (the **C** key).
    pub fn clear(&mut self, ui: &mut Ui) {
        ui.set_text(self.expr, "");
        ui.set_text(self.result, "0");
        ui.set_text(self.prev_expr, "");
        ui.set_color(self.result, None);
        ui.focus(Some(self.expr));
        self.fresh = false;
        self.unpick(ui);
    }

    /// Up (`older`) or Down: picks a past answer in the history.
    fn pick(&mut self, ui: &mut Ui, older: bool) {
        let n = self.past.len();
        if n == 0 { return; }
        self.picked = match (self.picked, older) {
            (None, true) => Some(0),
            (None, false) => None,
            (Some(i), true) => Some((i + 1).min(n - 1)),
            (Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
        };
        ui.select_row(self.history, self.picked);
    }

    fn set_angle(&mut self, ui: &mut Ui, degrees: bool) {
        self.angle = if degrees { Angle::Degrees } else { Angle::Radians };
        ui.commands.set_checked("abacus.degrees", degrees);
        ui.commands.set_checked("abacus.radians", !degrees);
        ui.set_text(self.angle_pill, if degrees { "DEG" } else { "RAD" });
        ui.toast(if degrees { "Angles in degrees" } else { "Angles in radians" }, None, 2000);
    }

    /// MC, MR, M+ and M−.
    fn memory_key(&mut self, ui: &mut Ui, k: &str) -> Option<Note> {
        // Typing goes on in the expression line whatever was clicked.
        ui.focus(Some(self.expr));
        match k {
            "MC" => self.memory = 0.0,
            "MR" => {
                let v = calc::format(self.memory);
                let text = if v.starts_with('-') { format!("({v})") } else { v };
                self.fresh = false;
                self.put(ui, &text);
                return None;
            }
            _ => {
                let v = self.value_now(ui)?;
                if k == "M+" { self.memory += v } else { self.memory -= v }
            }
        }
        Some(self.show_memory(ui))
    }

    /// Starts afresh with `text` and works it out (a sum sent from elsewhere, such as
    /// a reply to a notification).
    pub fn work_out(&mut self, ui: &mut Ui, text: &str) -> Note {
        self.clear(ui);
        ui.set_text(self.expr, text);
        self.equals(ui)
    }

    /// Works out the expression line.
    pub fn equals(&mut self, ui: &mut Ui) -> Note {
        // Typing goes on in the expression line whatever was clicked.
        ui.focus(Some(self.expr));
        let mut expr = String::from(ui.text_of(self.expr).trim());
        if expr.is_empty() {
            return Note::Error("empty".into());
        }
        // Straight after an answer, `+8` means `ans+8`. The line shows that too, so a
        // sum put right after an error still carries on.
        if core::mem::take(&mut self.fresh) && self.mode != Mode::Convert && expr.starts_with(['+', '-', '*', '/', '^', '%', '!', '×', '÷', '−', '&', '|']) {
            expr.insert_str(0, "ans");
            ui.set_text(self.expr, &expr);
        }
        self.unpick(ui);
        let evaluated: Result<String, String> = match self.mode {
            Mode::Scientific => calc::eval_with(&expr, self.ans, self.memory, self.angle)
                .map(|v| {
                    self.ans = v;
                    calc::format(v)
                })
                .map_err(|e| e.message()),
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
                ui.set_text(self.prev_expr, &format!("{expr} ="));
                ui.set_text(self.result, &value);
                ui.set_color(self.result, None);
                if self.mode == Mode::Programmer {
                    self.update_base_line(ui);
                }
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
                self.fresh = true;
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
        ui.set_visible(self.base_line, mode == Mode::Programmer);
        ui.set_visible(self.angle_pill, mode == Mode::Scientific);
        if mode == Mode::Programmer {
            self.update_base_line(ui);
        }
        ui.set_text(self.help,&match mode{Mode::Scientific=>"Scientific: functions, factorial, degrees/radians and ans.".into(),Mode::Programmer=>format!("{}-bit unsigned wrapping arithmetic. Literals 0x/0b/0o; ^ means XOR. Output radix {} (Words menu).",self.word_bits,self.radix),Mode::Convert=>"Type: 32 F to C. Units: m/km/cm/mm/in/ft/mi; kg/g/lb/oz; s/ms/min/h/day; K/C/F; B/kB/MB/GB/KiB/MiB/GiB.".into()});
        ui.focus(Some(self.expr));
    }

    fn update_base_line(&self, ui: &mut Ui) {
        let v = self.integer_ans;
        let hex = programmer::format(v, 16);
        let dec = programmer::format(v, 10);
        let bin = programmer::format(v, 2);
        ui.set_text(self.base_line, &format!("HEX {hex}  ·  DEC {dec}  ·  BIN {bin}"));
    }

    fn recall(&mut self, ui: &mut Ui, row: usize) -> Option<Note> {
        let (expr, value) = self.past.get(row).cloned()?;
        {
            let (mode, bits, radix) =
                self.history_context
                    .get(row)
                    .copied()
                    .unwrap_or((Mode::Scientific, 64, 10));
            self.word_bits = bits;
            self.radix = radix;
            self.set_mode(ui, mode);
            ui.set_text(self.expr, "");
            // A negative answer goes in brackets so it stays one value in what follows.
            let text = match mode {
                Mode::Convert => expr,
                Mode::Scientific if value.starts_with('-') => format!("({value})"),
                _ => value.clone(),
            };
            self.put(ui, &text);
            self.fresh = false;
        }
        Some(Note::Recall { index: row, value })
    }

    /// Acts on one response; returns a note for the record, if any.
    pub fn on(&mut self, ui: &mut Ui, r: &Response) -> Option<Note> {
        match r {
            Response::Submitted(id) if *id == self.expr => {
                // Enter on a picked answer puts it in; otherwise it works the sum out.
                if let Some(i) = self.picked {
                    if i < self.past.len() { return self.recall(ui, i); }
                    self.unpick(ui);
                }
                return Some(self.equals(ui));
            }
            Response::Clicked(id) => {
                if *id == self.angle_pill {
                    let deg = self.angle != Angle::Degrees;
                    self.set_angle(ui, deg);
                    return Some(Note::Angle(deg));
                }
                if *id == self.copy_btn {
                    let t = String::from(self.result_text(ui));
                    ui.set_clipboard(&t);
                    ui.toast("Copied result", None, 1500);
                    ui.focus(Some(self.expr));
                    return None;
                }
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
                    "C" => self.clear(ui),
                    "MC" | "MR" | "M+" | "M−" => return self.memory_key(ui, k),
                    "±" => self.wrap(ui, "-", ""),
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
                    "×" | "÷" | "−" if self.mode == Mode::Programmer => self.put(
                        ui,
                        match k {
                            "×" => "*",
                            "÷" => "/",
                            _ => "-",
                        },
                    ),
                    "−" => self.put(ui, "−"),
                    k => self.put(ui, k),
                }
            }
            Response::Activated(id, row) if *id == self.history => return self.recall(ui, *row),
            Response::Command(c) => match c.as_str() {
                "abacus.recall-last" => return self.recall(ui, 0),
                "abacus.older" | "abacus.newer" => self.pick(ui, c == "abacus.older"),
                "abacus.square" => self.wrap(ui, "", "^2"),
                "abacus.recip" => self.wrap(ui, "1/", ""),
                "abacus.negate" => self.wrap(ui, "-", ""),
                "abacus.mem-clear" => return self.memory_key(ui, "MC"),
                "abacus.mem-recall" => return self.memory_key(ui, "MR"),
                "abacus.mem-add" => return self.memory_key(ui, "M+"),
                "abacus.mem-sub" => return self.memory_key(ui, "M−"),
                "abacus.angle" => {
                    let deg = self.angle != Angle::Degrees;
                    self.set_angle(ui, deg);
                    return Some(Note::Angle(deg));
                }
                id if id.starts_with("abacus.fn.") => {
                    if let Some((_, _, text)) = INSERTS.iter().find(|i| i.0 == id) { self.put(ui, text); }
                }
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
                "abacus.clear" => self.clear(ui),
                "abacus.clear-history" => {
                    self.past.clear();
                    self.picked = None;
                    self.history_context.clear();
                    ui.set_rows(self.history, Rows::default());
                }
                "abacus.degrees" | "abacus.radians" => {
                    let deg = c == "abacus.degrees";
                    self.set_angle(ui, deg);
                    return Some(Note::Angle(deg));
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
