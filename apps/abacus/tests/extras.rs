// SPDX-License-Identifier: GPL-3.0-only
use abacus::{
    calc::{self, Angle},
    programmer as p, units, Abacus, Mode, Note,
};
use weft::{Response, Strings, Theme, Ui};
#[test]
fn programmer_is_exact_above_f64_precision_and_wraps_at_word_width() {
    assert_eq!(p::eval("18446744073709551615", 0, 64), Ok(u64::MAX));
    assert_eq!(p::eval("9007199254740993+2", 0, 64), Ok(9007199254740995));
    assert_eq!(p::eval("255+1", 0, 8), Ok(0));
    assert_eq!(p::eval("-1", 0, 16), Ok(65535));
    assert_eq!(p::eval("0xFFFF*2", 0, 16), Ok(65534));
}
#[test]
fn programmer_bitwise_precedence_bases_and_ans() {
    assert_eq!(p::eval("0xF0 | 0b1010", 0, 16), Ok(250));
    assert_eq!(p::eval("0o70 ^ 0xF", 0, 64), Ok(55));
    assert_eq!(p::eval("~0 & 0xFF", 0, 32), Ok(255));
    assert_eq!(p::eval("1+2<<3", 0, 32), Ok(24));
    assert_eq!(p::eval("3|4&1", 0, 32), Ok(3));
    assert_eq!(p::eval("ans+1", u64::MAX, 64), Ok(0));
    assert_eq!(p::format(u64::MAX, 16), "0xFFFFFFFFFFFFFFFF");
}
#[test]
fn programmer_refuses_ambiguous_or_unsafe_inputs() {
    for text in [
        "", "1/0", "1%0", "1<<64", "2>>65", "0x", "0b2", "1.5", "1 2", "0xG", "(1+2",
    ] {
        assert!(p::eval(text, 0, 64).is_err(), "{text}");
    }
    assert!(p::eval("1", 0, 7).is_err());
    assert!(p::eval(&"-".repeat(200), 0, 64).is_err());
}
#[test]
fn units_convert_dimensions_affine_temperatures_and_binary_storage() {
    for (text, want) in [
        ("2.5 km to m", "2500 m"),
        ("32 F to C", "0 C"),
        ("100 C to F", "212 F"),
        ("1 MiB to KiB", "1024 KiB"),
        ("1 MB to B", "1000000 B"),
        ("1 day to h", "24 h"),
    ] {
        assert_eq!(units::eval(text).unwrap(), want);
    }
    assert!((units::convert(1., "lb", "kg").unwrap() - 0.45359237).abs() < 1e-12);
    assert!((units::convert(1., "in", "cm").unwrap() - 2.54).abs() < 1e-12);
}
#[test]
fn units_refuse_cross_dimensions_nonfinite_and_below_absolute_zero() {
    for text in [
        "1 m to kg",
        "1 frogs to m",
        "NaN m to m",
        "inf m to m",
        "-1 K to C",
        "-500 F to C",
        "1 m to m trailing",
        "1 m m",
    ] {
        assert!(units::eval(text).is_err(), "{text}");
    }
}
#[test]
fn scientific_extras_refuse_nonfinite_results_and_bound_nesting() {
    assert_eq!(calc::eval("cbrt(-27)", 0., Angle::Degrees).unwrap(), -3.);
    assert_eq!(calc::eval("log2(8)", 0., Angle::Degrees).unwrap(), 3.);
    assert_eq!(calc::eval("exp2(3)", 0., Angle::Degrees).unwrap(), 8.);
    assert_eq!(calc::eval("trunc(1.8)", 0., Angle::Degrees).unwrap(), 1.);
    assert!(calc::eval("exp(10000)", 0., Angle::Degrees).is_err());
    assert!(calc::eval(&"(".repeat(100), 0., Angle::Degrees).is_err());
}
fn result(a: &mut Abacus, ui: &mut Ui, text: &str) -> String {
    ui.set_text(a.expr, text);
    match a.equals(ui) {
        Note::Result { value, .. } => value,
        Note::Error(e) => panic!("{e}"),
    }
}
#[test]
fn real_mode_buttons_and_history_preserve_exact_programmer_context() {
    let mut ui = Ui::new(800, 700, Theme::light(), Strings::new("en"));
    let mut a = Abacus::build(&mut ui);
    a.on(&mut ui, &Response::Clicked(a.modes[1].0));
    assert_eq!(a.mode, Mode::Programmer);
    a.on(&mut ui, &Response::Command("abacus.bits64".into()));
    a.on(&mut ui, &Response::Command("abacus.hex".into()));
    assert_eq!(
        result(&mut a, &mut ui, "18446744073709551615"),
        "0xFFFFFFFFFFFFFFFF"
    );
    a.on(&mut ui, &Response::Command("abacus.bits8".into()));
    a.on(&mut ui, &Response::Clicked(a.modes[2].0));
    assert_eq!(result(&mut a, &mut ui, "32 F to C"), "0 C");
    ui.set_text(a.expr, "discard me");
    a.on(&mut ui, &Response::Activated(a.history, 1));
    assert_eq!(a.mode, Mode::Programmer);
    assert_eq!(a.word_bits, 64);
    assert_eq!(a.radix, 16);
    assert_eq!(ui.text_of(a.expr), "0xFFFFFFFFFFFFFFFF");
    assert_eq!(result(&mut a, &mut ui, "ans+1"), "0x100");
    a.on(&mut ui, &Response::Activated(a.history, 1));
    assert_eq!(a.mode, Mode::Convert);
    assert_eq!(ui.text_of(a.expr), "32 F to C");
    a.on(&mut ui, &Response::Command("abacus.clear-history".into()));
    assert!(a.past.is_empty());
    let mut pixels = vec![0; 800 * 700];
    assert!(ui.paint(&mut pixels).is_some());
}
