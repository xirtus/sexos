// SPDX-License-Identifier: GPL-3.0-only
//! Golden answers: each expression, the text the calculator shows for it.
use sex_calc_core::{eval, format, Angle, Calculator, Context, Error, FORMAT_BYTES};

fn shown(expr: &str, ctx: &Context) -> Result<String, Error> {
    let mut out = [0u8; FORMAT_BYTES];
    eval(expr, ctx).map(|v| format(v, &mut out).to_string())
}

const RADIANS: &[(&str, &str)] = &[
    ("1+2", "3"),
    ("2+3*4", "14"),
    ("(2+3)*4", "20"),
    ("10-4-3", "3"),
    ("100/10/5", "2"),
    ("7%3", "1"),
    ("-7%3", "-1"),
    ("2^10", "1024"),
    ("2^3^2", "512"),
    ("-2^2", "-4"),
    ("(-2)^2", "4"),
    ("2^-1", "0.5"),
    ("--3", "3"),
    ("+5", "5"),
    ("3-(-2)", "5"),
    ("0.1+0.2", "0.3"),
    ("1/3", "0.333333333333"),
    ("2/3", "0.666666666667"),
    (".5*4", "2"),
    ("1.5e3", "1500"),
    ("2e-3", "0.002"),
    ("2e", "5.43656365692"),
    ("2pi", "6.28318530718"),
    ("3(4+1)", "15"),
    ("(1+2)(3+4)", "21"),
    ("pi", "3.14159265359"),
    ("e", "2.71828182846"),
    ("sqrt(16)", "4"),
    ("sqrt(2)", "1.41421356237"),
    ("ln(e)", "1"),
    ("log(1000)", "3"),
    ("exp(0)", "1"),
    ("abs(-3.5)", "3.5"),
    ("sin(0)", "0"),
    ("cos(0)", "1"),
    ("sin(pi/2)", "1"),
    ("atan(1)*4", "3.14159265359"),
    ("2sin(pi/6)", "1"),
    ("5!", "120"),
    ("3!+1", "7"),
    ("10^12", "1e12"),
    ("123456789*1000", "123456789000"),
    ("1/1000000", "0.000001"),
    ("1/10000000", "1e-7"),
    ("2^100", "1.26765060023e30"),
    ("-0.5*2", "-1"),
    ("  1 +  2 * 3 ", "7"),
    ("((((1))))", "1"),
    ("sqrt(sqrt(81))", "3"),
    ("170!", "7.25741561531e306"),
];

const DEGREES: &[(&str, &str)] = &[
    ("sin(30)", "0.5"),
    ("sin(180)", "0"),
    ("cos(90)", "0"),
    ("cos(60)", "0.5"),
    ("tan(45)", "1"),
    ("sin(-90)", "-1"),
    ("cos(360)", "1"),
    ("asin(1)", "90"),
    ("acos(0.5)", "60"),
    ("atan(1)", "45"),
];

const ERRORS: &[(&str, Error)] = &[
    ("1/0", Error::DivideByZero),
    ("5%0", Error::DivideByZero),
    ("0^-1", Error::DivideByZero),
    ("(1+2", Error::Unbalanced),
    ("1+2)", Error::Unbalanced),
    (")(", Error::Unbalanced),
    ("sqrt(-1)", Error::Domain),
    ("ln(0)", Error::Domain),
    ("log(-5)", Error::Domain),
    ("asin(2)", Error::Domain),
    ("(-8)^0.5", Error::Domain),
    ("(-1)!", Error::Domain),
    ("2.5!", Error::Domain),
    ("171!", Error::Overflow),
    ("10^400", Error::Overflow),
    ("exp(1000)", Error::Overflow),
    ("", Error::Empty),
    ("   ", Error::Empty),
    ("1+", Error::Syntax),
    ("*2", Error::Syntax),
    ("2 3", Error::Syntax),
    ("1..2", Error::Syntax),
    ("foo(1)", Error::Unknown),
    ("x+1", Error::Unknown),
    ("sin 30", Error::Syntax),
    ("()", Error::Syntax),
];

#[test]
fn radians() {
    let ctx = Context::default();
    for (expr, want) in RADIANS {
        assert_eq!(shown(expr, &ctx).as_deref(), Ok(*want), "{expr}");
    }
}

#[test]
fn degrees() {
    let ctx = Context { angle: Angle::Degrees, ..Context::default() };
    for (expr, want) in DEGREES {
        assert_eq!(shown(expr, &ctx).as_deref(), Ok(*want), "{expr}");
    }
    assert_eq!(eval("tan(90)", &ctx), Err(Error::Domain));
    assert_eq!(eval("tan(-270)", &ctx), Err(Error::Domain));
}

#[test]
fn errors() {
    let ctx = Context::default();
    for (expr, want) in ERRORS {
        assert_eq!(eval(expr, &ctx), Err(*want), "{expr}");
    }
}

#[test]
fn golden_count() {
    assert!(RADIANS.len() + DEGREES.len() + ERRORS.len() >= 80);
}

#[test]
fn ans_memory_and_history() {
    let mut c = Calculator::new();
    assert_eq!(c.evaluate("6*7"), Ok(42.0));
    assert_eq!(c.evaluate("ans/2"), Ok(21.0));
    c.memory_add(c.ctx.ans);
    c.memory_add(4.0);
    c.memory_subtract(5.0);
    assert_eq!(c.evaluate("mem*2"), Ok(40.0));
    assert_eq!(c.evaluate("1/0"), Err(Error::DivideByZero));
    // A failed line leaves ans and the history as they were.
    assert_eq!(c.ctx.ans, 40.0);
    assert_eq!(c.history_len(), 3);
    assert_eq!(c.history(0).unwrap().expr(), "mem*2");
    assert_eq!(c.history(2).unwrap().value, 42.0);
    c.memory_clear();
    assert_eq!(c.memory(), 0.0);
    c.toggle_angle();
    assert_eq!(c.evaluate("sin(90)"), Ok(1.0));
    for i in 0..30 { c.evaluate(&format!("{i}+1")).unwrap(); }
    assert_eq!(c.history_len(), 20);
    assert_eq!(c.history(0).unwrap().expr(), "29+1");
    assert_eq!(c.history(19).unwrap().expr(), "10+1");
    assert!(c.history(20).is_none());
    c.clear_history();
    assert_eq!(c.history_len(), 0);
}

#[test]
fn formatting() {
    let mut out = [0u8; FORMAT_BYTES];
    for (v, want) in [(0.0, "0"), (-0.0, "0"), (1e-300, "1e-300"), (-1.5e20, "-1.5e20"), (999999999999.0, "999999999999"), (1e11 + 0.25, "100000000000"), (0.000123, "0.000123"), (f64::MAX, "1.79769313486e308")] {
        assert_eq!(format(v, &mut out), want, "{v}");
    }
}
