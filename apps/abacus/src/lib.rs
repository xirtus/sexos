// SPDX-License-Identifier: GPL-3.0-only
//! Abacus, the SexOS calculator, built on Weft: a keypad, an expression line you can
//! type into, a history to reuse answers from, menus, the command palette and
//! keyboard shortcuts. The window code lives here so tests drive it on the host.
#![no_std]
extern crate alloc;

pub mod calc;

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
    pub result: Id,
    pub expr: Id,
    pub history: Id,
    pub keys: Vec<(Id, &'static str)>,
    pub ans: f64,
    pub angle: Angle,
    pub past: Vec<(String, String)>,
}

/// Something worth a line in the serial record.
pub enum Note { Result { expr: String, value: String }, Error(String) }

impl Abacus {
    pub fn build(ui: &mut Ui) -> Abacus {
        for c in [
            Command::new("abacus.copy", "Copy Result").shortcut("Ctrl+Shift+C").icon("copy"),
            Command::new("abacus.clear", "Clear").shortcut("Escape").icon("close").keywords("reset all clear"),
            Command::new("abacus.clear-history", "Clear History").icon("trash"),
            Command::new("abacus.degrees", "Degrees").checked(true).keywords("angle"),
            Command::new("abacus.radians", "Radians").checked(false).keywords("angle"),
            Command::new("abacus.about", "About Abacus").icon("info"),
        ] { ui.commands.add(c); }
        let root = ui.root();
        ui.set_layout(root, Layout::column());
        ui.menubar(root, vec![
            Menu::new("Edit").command("abacus.copy").command("weft.paste").separator().command("abacus.clear").command("abacus.clear-history"),
            Menu::new("View").command("abacus.degrees").command("abacus.radians").separator().command("weft.palette"),
            Menu::new("Help").command("abacus.about"),
        ]);
        let body = ui.group(root, Layout::column().gap(10.0).padding(12.0).fill());
        let result = ui.text(body, "0", TextStyle::Display);
        ui.set_align(result, Align::End);
        ui.set_layout(result, Layout::default().fill_width());
        ui.set_label(result, "Result");
        let expr = ui.field(body, "", "Type a sum, like 12 + 30");
        ui.set_layout(expr, Layout::default().fill_width());
        ui.set_label(expr, "Expression");
        let split = ui.split(body, false, 0.62);
        let pad = ui.group(split, Layout::grid(5).gap(6.0));
        let mut keys = Vec::new();
        for row in KEYS {
            for k in row {
                let b = ui.button(pad, k);
                ui.set_layout(b, Layout::default().fill_width().min_height(40.0));
                match k { "=" => ui.set_button_style(b, ButtonStyle::Primary), "C" => ui.set_button_style(b, ButtonStyle::Danger), "⌫" => ui.set_label(b, "Backspace"), _ => {} }
                keys.push((b, k));
            }
        }
        let side = ui.column(split);
        ui.text(side, "History", TextStyle::Caption);
        let history = ui.list_with(side, alloc::boxed::Box::new(Rows::default()), vec![Column::new("Sum", 150.0), Column::new("Answer", 100.0).right()]);
        ui.set_tooltip(history, "Double-click or Enter to reuse an answer");
        ui.focus(Some(expr));
        Abacus { result, expr, history, keys, ans: 0.0, angle: Angle::Degrees, past: Vec::new() }
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
        if expr.is_empty() { return Note::Error("empty".into()); }
        match calc::eval(&expr, self.ans, self.angle) {
            Ok(v) => {
                self.ans = v;
                let value = calc::format(v);
                ui.set_text(self.result, &value);
                ui.set_color(self.result, None);
                self.past.insert(0, (expr.clone(), value.clone()));
                self.past.truncate(500);
                ui.set_rows(self.history, Rows(self.past.iter().map(|(e, v)| vec![e.clone(), format!("= {v}")]).collect()));
                ui.set_text(self.expr, "");
                Note::Result { expr, value }
            }
            Err(e) => {
                let msg = match e {
                    calc::Error::DivideByZero => String::from("Can't divide by zero"),
                    calc::Error::Domain => String::from("Out of range"),
                    calc::Error::Unknown(n) => format!("Unknown: {n}"),
                    calc::Error::Syntax(_) => String::from("Check the sum"),
                };
                ui.set_text(self.result, &msg);
                let danger = ui.theme().palette.danger;
                ui.set_color(self.result, Some(danger));
                Note::Error(msg)
            }
        }
    }

    /// Acts on one response; returns a note for the record, if any.
    pub fn on(&mut self, ui: &mut Ui, r: &Response) -> Option<Note> {
        match r {
            Response::Submitted(id) if *id == self.expr => return Some(self.equals(ui)),
            Response::Clicked(id) => {
                let k = self.keys.iter().find(|(b, _)| b == id)?.1;
                match k {
                    "=" => return Some(self.equals(ui)),
                    "C" => { ui.set_text(self.expr, ""); ui.set_text(self.result, "0"); ui.set_color(self.result, None); ui.focus(Some(self.expr)); }
                    "⌫" => { let mut t = String::from(ui.text_of(self.expr)); t.pop(); ui.set_text(self.expr, &t); ui.focus(Some(self.expr)); }
                    "sin" | "cos" | "tan" | "ln" | "log" => { let s = format!("{k}("); self.put(ui, &s); }
                    "√" => self.put(ui, "√("),
                    "ans" => self.put(ui, "ans"),
                    "−" => self.put(ui, "−"),
                    k => self.put(ui, k),
                }
            }
            Response::Activated(id, row) if *id == self.history => {
                if let Some((_, v)) = self.past.get(*row).cloned() { self.put(ui, &v); }
            }
            Response::Command(c) => match c.as_str() {
                "abacus.clear" => { ui.set_text(self.expr, ""); ui.set_text(self.result, "0"); ui.set_color(self.result, None); }
                "abacus.clear-history" => { self.past.clear(); ui.set_rows(self.history, Rows::default()); }
                "abacus.degrees" | "abacus.radians" => {
                    self.angle = if c == "abacus.degrees" { Angle::Degrees } else { Angle::Radians };
                    ui.commands.set_checked("abacus.degrees", self.angle == Angle::Degrees);
                    ui.commands.set_checked("abacus.radians", self.angle == Angle::Radians);
                    ui.toast(if self.angle == Angle::Degrees { "Angles in degrees" } else { "Angles in radians" }, None, 2000);
                }
                "abacus.about" => { ui.message("Abacus", "The SexOS calculator, built on Weft.", &["OK"], 0, 0); }
                _ => {}
            },
            _ => {}
        }
        None
    }

    /// Text for Copy Result.
    pub fn result_text<'a>(&self, ui: &'a Ui) -> &'a str { ui.text_of(self.result) }
}
