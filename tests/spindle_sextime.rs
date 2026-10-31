//! Native tests of Spindle's production `sextime` calculator command.
#[path = "../apps/spindle/src/sextime_command.rs"]
mod sextime_command;
use sextime_command::{parse, render};

fn text(unix: i64) -> Vec<String> {
    let lines = render(unix).unwrap();
    (0..3).map(|i| String::from_utf8(lines.rows[i][..lines.lens[i]].to_vec()).unwrap()).collect()
}

#[test]
fn epoch_and_neighbours_render_exactly() {
    assert_eq!(text(1_356_048_000), ["SexTime 0 s", "UTC 2012-12-21T00:00:00Z", "Long Count 13.0.0.0.0 (GMT 584283)"]);
    assert_eq!(text(1_356_047_999), ["SexTime -1 s", "UTC 2012-12-20T23:59:59Z", "Long Count 12.19.19.17.19 (GMT 584283)"]);
    assert_eq!(text(0), ["SexTime -1356048000 s", "UTC 1970-01-01T00:00:00Z", "Long Count 12.17.16.7.5 (GMT 584283)"]);
}

#[test]
fn parse_accepts_only_bounded_signed_decimal() {
    for (input, want) in [(&b"0"[..], Some(0)), (b"-1", Some(-1)), (b"1356048000", Some(1_356_048_000)),
                          (b"9223372036854775807", Some(i64::MAX)), (b"-9223372036854775807", Some(-i64::MAX))] {
        assert_eq!(parse(input), want, "{:?}", input);
    }
    for bad in [&b""[..], b"-", b"+1", b"1.5", b"12a", b" 1", b"0x10", b"99999999999999999999", b"9223372036854775808"] {
        assert_eq!(parse(bad), None, "{:?}", bad);
    }
}

#[test]
fn out_of_range_values_render_nothing_rather_than_wrong_dates() {
    assert!(render(i64::MIN).is_none());
    assert!(render(i64::MAX).is_none());
    let early = text(-200_000_000_000);
    assert!(early[2].ends_with("before 0.0.0.0.0"), "{early:?}");
    for unix in [-62_135_596_800i64, 253_402_300_799, 4_102_444_800] {
        for line in text(unix) { assert!(line.len() <= sextime_command::LINE_MAX); }
    }
}
