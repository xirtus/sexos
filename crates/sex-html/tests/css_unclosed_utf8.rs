// SPDX-License-Identifier: GPL-3.0-only
//! Issue #260: an unclosed block ending in a multibyte character must not panic.
use sex_html::css::parse_sheet;

#[test]
fn unclosed_block_ending_in_multibyte_char() {
    let data = core::str::from_utf8(&[123, 220, 140]).unwrap();
    let _ = parse_sheet(data, 800);
}

#[test]
fn unclosed_blocks_with_any_tail() {
    let tails = ["\u{70c}", "é", "€", "😀", "a", "", "\"é", "/*é", "'€"];
    let heads = ["{", "a{", "@media screen{", "@keyframes k{from{", "@font-face{", "@supports(x){b{", "a{color:red;}b{"];
    for h in heads {
        for t in tails {
            let css = alloc_join(h, t);
            let _ = parse_sheet(&css, 800);
        }
    }
    // A closed block still keeps its declarations.
    let sheet = parse_sheet("a{color:red}b{color:blue", 800);
    assert_eq!(sheet.rules.len(), 2);
}

fn alloc_join(a: &str, b: &str) -> String { let mut s = String::from(a); s.push_str(b); s }
