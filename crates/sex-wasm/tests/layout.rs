// SPDX-License-Identifier: GPL-3.0-only
//! Window layout plugins: the checked-in guests are built from their sources, the
//! spiral places windows as documented, and bad plugins end with the right error.
use sex_wasm::layout::{LayoutError, LayoutLimits, LayoutPlugin};

const GUESTS: [&str; 3] = ["spiral", "outside", "spin"];

fn guest(name: &str) -> Vec<u8> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("guests/layouts");
    std::fs::read(dir.join(format!("{name}.wasm"))).unwrap()
}

#[test]
fn layout_guests_are_current() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("guests/layouts");
    for name in GUESTS {
        let built = wat::parse_file(dir.join(format!("{name}.wat"))).unwrap();
        if std::env::var_os("SEX_WASM_BLESS").is_some() { std::fs::write(dir.join(format!("{name}.wasm")), &built).unwrap(); }
        assert_eq!(std::fs::read(dir.join(format!("{name}.wasm"))).unwrap(), built, "{name}.wasm is stale: rerun with SEX_WASM_BLESS=1");
    }
}

/// The spiral as `scripts/wasm_layout_gate.py` expects it.
fn spiral(n: u32, mut w: i64, mut h: i64) -> Vec<[i64; 4]> {
    let (mut x, mut y) = (0i64, 0i64);
    let mut out = Vec::new();
    let put = |out: &mut Vec<[i64; 4]>, x: i64, y: i64, w: i64, h: i64| out.push([x + 4, y + 4, w - 8, h - 8]);
    for i in 0..n {
        if i + 1 == n { put(&mut out, x, y, w, h); break; }
        match i % 4 {
            0 => { let half = w / 2; put(&mut out, x, y, half, h); x += half; w -= half; }
            1 => { let half = h / 2; put(&mut out, x, y, w, half); y += half; h -= half; }
            2 => { let half = w / 2; put(&mut out, x + w - half, y, half, h); w -= half; }
            _ => { let half = h / 2; put(&mut out, x, y + h - half, w, half); h -= half; }
        }
    }
    out
}

#[test]
fn the_spiral_halves_what_is_left_turning_each_time() {
    let p = LayoutPlugin::new(&guest("spiral"), LayoutLimits::default()).unwrap();
    assert_eq!(p.layout(1, 1280, 752).unwrap(), vec![[4, 4, 1272, 744]]);
    assert_eq!(p.layout(3, 1280, 752).unwrap(), vec![[4, 4, 632, 744], [644, 4, 632, 368], [644, 380, 632, 368]]);
    for n in 1..=12 {
        assert_eq!(p.layout(n, 1280, 752).unwrap(), spiral(n, 1280, 752), "n={n}");
    }
    // Past a dozen the tiles shrink to nothing and widths wrap: the Shell refuses
    // that answer (below the minimum size) and keeps the previous layout.
    assert!(p.layout(24, 1280, 752).unwrap().iter().any(|r| r[2] > 1280 || r[3] > 752 || r[2] < 120));
    // The same plugin answers again and again: each call is a fresh instance.
    for _ in 0..3 { assert_eq!(p.layout(2, 800, 600).unwrap(), spiral(2, 800, 600)); }
}

#[test]
fn bad_plugins_end_with_their_reason() {
    let outside = LayoutPlugin::new(&guest("outside"), LayoutLimits::default()).unwrap();
    let r = outside.layout(3, 1280, 752).unwrap();
    assert!(r.iter().all(|r| r[0] >= 1280), "{r:?}");
    let spin = LayoutPlugin::new(&guest("spin"), LayoutLimits::default()).unwrap();
    assert_eq!(spin.layout(3, 1280, 752), Err(LayoutError::OutOfFuel));
    // Fuel runs out on every call, and the next call still gets a clean instance.
    assert_eq!(spin.layout(3, 1280, 752), Err(LayoutError::OutOfFuel));
    let p = LayoutPlugin::new(&guest("spiral"), LayoutLimits::default()).unwrap();
    assert_eq!(p.layout(0, 1280, 752), Err(LayoutError::BadCount));
    assert_eq!(p.layout(25, 1280, 752), Err(LayoutError::BadCount));
}

fn module(wat: &str) -> Result<LayoutPlugin, LayoutError> { LayoutPlugin::new(&wat::parse_str(wat).unwrap(), LayoutLimits::default()) }

#[test]
fn modules_are_checked_before_they_run() {
    assert!(matches!(LayoutPlugin::new(b"not wasm", LayoutLimits::default()), Err(LayoutError::Module(_))));
    // No imports at all: a plugin reaches nothing but its own memory.
    let e = module(r#"(module (import "sex" "log" (func (param i32 i32) (result i32))) (memory (export "memory") 1)
        (func (export "layout") (param i32 i32 i32) (result i32) (i32.const 0)))"#).err().unwrap();
    assert!(matches!(&e, LayoutError::Module(m) if m.contains("sex.log")), "{e:?}");
    assert!(matches!(module(r#"(module (func (export "layout") (param i32 i32 i32) (result i32) (i32.const 0)))"#), Err(LayoutError::Module(_))));
    assert!(matches!(module(r#"(module (memory (export "memory") 1))"#), Err(LayoutError::Module(_))));
    // The wrong signature is found at the call.
    let p = module(r#"(module (memory (export "memory") 1) (func (export "layout") (param i32) (result i32) (i32.const 0)))"#).unwrap();
    assert!(matches!(p.layout(1, 10, 10), Err(LayoutError::Module(_))));
    // A pointer whose rectangles leave memory, a negative one, and a trap.
    let p = module(r#"(module (memory (export "memory") 1) (func (export "layout") (param i32 i32 i32) (result i32) (i32.const 65530)))"#).unwrap();
    assert_eq!(p.layout(1, 10, 10), Err(LayoutError::BadResult));
    let p = module(r#"(module (memory (export "memory") 1) (func (export "layout") (param i32 i32 i32) (result i32) (i32.const -1)))"#).unwrap();
    assert_eq!(p.layout(1, 10, 10), Err(LayoutError::BadResult));
    let p = module(r#"(module (memory (export "memory") 1) (func (export "layout") (param i32 i32 i32) (result i32) unreachable))"#).unwrap();
    assert!(matches!(p.layout(1, 10, 10), Err(LayoutError::Trapped(_))));
    // Memory past the limit cannot be had.
    let p = module(r#"(module (memory (export "memory") 1) (func (export "layout") (param i32 i32 i32) (result i32)
        (drop (memory.grow (i32.const 64))) (i32.mul (memory.size) (i32.const 65536))))"#).unwrap();
    assert_eq!(p.layout(1, 10, 10), Err(LayoutError::BadResult));
}
