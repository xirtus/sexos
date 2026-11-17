// SPDX-License-Identifier: GPL-3.0-only
//! Headless host CLI driver for Burlesque (`--inspect`, `--review`, `--apply`).

use std::fs;
use std::path::Path;
use std::string::{String, ToString};
use std::vec::Vec;

use burlesque_core::deck::{AspectRatio, Deck};
use burlesque_core::ops::{apply_ops_json, inspect_deck_json};
use burlesque_core::review::review_deck;

/// Result of executing a headless CLI invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliOutcome {
    pub stdout: String,
    pub exit_code: i32,
}

/// Load a deck from disk by extension (`.pptx`, `.odp`, `.key`, `.md`).
/// Passing `"--new"` or `"new"` creates a blank 16:9 deck.
pub fn load_deck_from_path(path: &str) -> Result<Deck, String> {
    if path == "--new" || path == "new" {
        return Ok(Deck::new(AspectRatio::Ratio16x9));
    }
    let bytes = fs::read(path).map_err(|e| format!("failed to read '{}': {}", path, e))?;
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".pptx") {
        Deck::open_pptx(&bytes).map_err(|e| format!("failed to open PPTX '{}': {}", path, e))
    } else if lower.ends_with(".odp") {
        Deck::open_odp(&bytes).map_err(|e| format!("failed to open ODP '{}': {}", path, e))
    } else if lower.ends_with(".key") {
        Deck::open_keynote(&bytes).map_err(|e| format!("failed to open .key presentation '{}': {}", path, e))
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        let s = std::str::from_utf8(&bytes)
            .map_err(|e| format!("invalid UTF-8 in '{}': {}", path, e))?;
        Deck::from_markdown(s).map_err(|e| format!("failed to parse Markdown '{}': {}", path, e))
    } else if lower.ends_with(".json") {
        // Treat a .json file as an ops sequence starting from an empty 16:9 deck
        let s = std::str::from_utf8(&bytes)
            .map_err(|e| format!("invalid UTF-8 in '{}': {}", path, e))?;
        let mut deck = Deck::new(AspectRatio::Ratio16x9);
        apply_ops_json(&mut deck, s)
            .map_err(|e| format!("failed to initialize deck from ops JSON '{}': {}", path, e))?;
        Ok(deck)
    } else {
        // Sniff ZIP vs Markdown
        if bytes.starts_with(b"PK\x03\x04") {
            Deck::open_pptx(&bytes)
                .or_else(|_| Deck::open_odp(&bytes))
                .or_else(|_| Deck::open_keynote(&bytes))
                .map_err(|e| format!("unrecognized presentation archive '{}': {}", path, e))
        } else {
            let s = std::str::from_utf8(&bytes)
                .map_err(|e| format!("unsupported binary format '{}': {}", path, e))?;
            Deck::from_markdown(s)
                .map_err(|e| format!("failed to parse presentation '{}': {}", path, e))
        }
    }
}

/// Write `deck` to `out_path` using extension-selected format (`.pptx`, `.odp`, `.md`, `.pdf`, `.json`).
pub fn save_deck_to_path(deck: &Deck, out_path: &str) -> Result<(), String> {
    let lower = out_path.to_ascii_lowercase();
    let bytes: Vec<u8> = if lower.ends_with(".pptx") {
        deck.export_pptx()
            .map_err(|e| format!("PPTX export error: {}", e))?
    } else if lower.ends_with(".odp") {
        deck.export_odp()
            .map_err(|e| format!("ODP export error: {}", e))?
    } else if lower.ends_with(".pdf") {
        deck.export_pdf()
            .map_err(|e| format!("PDF export error: {}", e))?
    } else if lower.ends_with(".md") || lower.ends_with(".markdown") {
        deck.to_markdown().into_bytes()
    } else {
        inspect_deck_json(deck).into_bytes()
    };

    if let Some(parent) = Path::new(out_path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    fs::write(out_path, &bytes).map_err(|e| format!("failed to write '{}': {}", out_path, e))
}

/// Execute headless CLI flags (`--inspect`, `--review`, `--apply`).
///
/// Supported invocations:
/// - `burlesque --inspect <deck-path>`
/// - `burlesque --review <deck-path> [--strict]`
/// - `burlesque --apply <ops.json> [--in <deck-path>] [--out <out-path>] [--inspect]`
/// - `burlesque --apply <deck-path> <ops.json> [--out <out-path>] [--inspect]`
pub fn run_headless_cli(args: &[String]) -> Result<Option<CliOutcome>, String> {
    if args.len() < 2 {
        return Ok(None);
    }

    match args[1].as_str() {
        "--inspect" => {
            let path = args
                .get(2)
                .ok_or_else(|| String::from("usage: burlesque --inspect <deck-path>"))?;
            let deck = load_deck_from_path(path)?;
            Ok(Some(CliOutcome {
                stdout: inspect_deck_json(&deck),
                exit_code: 0,
            }))
        }
        "--review" => {
            let path = args
                .get(2)
                .ok_or_else(|| String::from("usage: burlesque --review <deck-path> [--strict]"))?;
            let strict = args.iter().any(|a| a == "--strict");
            let deck = load_deck_from_path(path)?;
            let report = review_deck(&deck);
            let exit_code = if strict && report.has_errors() { 2 } else { 0 };
            Ok(Some(CliOutcome {
                stdout: report.to_json(),
                exit_code,
            }))
        }
        "--apply" => {
            let mut in_path: Option<String> = None;
            let mut ops_path: Option<String> = None;
            let mut out_path: Option<String> = None;
            let mut want_inspect = false;
            let mut positional: Vec<&str> = Vec::new();

            let mut i = 2usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--in" => {
                        i += 1;
                        let v = args.get(i).ok_or_else(|| String::from("--in requires a path"))?;
                        in_path = Some(v.clone());
                    }
                    "--ops" => {
                        i += 1;
                        let v = args.get(i).ok_or_else(|| String::from("--ops requires a path"))?;
                        ops_path = Some(v.clone());
                    }
                    "--out" => {
                        i += 1;
                        let v = args.get(i).ok_or_else(|| String::from("--out requires a path"))?;
                        out_path = Some(v.clone());
                    }
                    "--inspect" => {
                        want_inspect = true;
                    }
                    other => {
                        positional.push(other);
                    }
                }
                i += 1;
            }

            if ops_path.is_none() {
                match positional.len() {
                    1 => {
                        ops_path = Some(positional[0].to_string());
                    }
                    2 => {
                        if in_path.is_none() {
                            in_path = Some(positional[0].to_string());
                        }
                        ops_path = Some(positional[1].to_string());
                    }
                    _ => {
                        return Err(String::from(
                            "usage: burlesque --apply [<deck-path>] <ops.json> [--out <out-path>] [--inspect]",
                        ));
                    }
                }
            }

            let ops_file = ops_path.expect("checked above");
            let ops_str = fs::read_to_string(&ops_file)
                .map_err(|e| format!("failed to read ops '{}': {}", ops_file, e))?;

            let mut deck = match in_path {
                Some(ref p) => load_deck_from_path(p)?,
                None => Deck::new(AspectRatio::Ratio16x9),
            };

            let res = apply_ops_json(&mut deck, &ops_str)
                .map_err(|e| format!("apply_ops failed: {}", e))?;

            if let Some(ref out) = out_path {
                save_deck_to_path(&deck, out)?;
            }

            let stdout = if want_inspect {
                inspect_deck_json(&deck)
            } else {
                res.to_json()
            };

            Ok(Some(CliOutcome {
                stdout,
                exit_code: 0,
            }))
        }
        _ => Ok(None),
    }
}
