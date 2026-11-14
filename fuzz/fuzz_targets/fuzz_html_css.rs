// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = core::str::from_utf8(data) {
        // 1. HTML5 DOM parsing and styling
        let dom = sex_html::parse::parse(text);
        let _ = sex_html::styles(&dom, &[], 800, 600);

        // 2. CSS stylesheet parsing
        let _ = sex_html::css::parse_sheet(text, 800);

        // 3. URL parsing & resolving
        let _ = sex_html::url::resolve("http://example.com/dir/page.html", text);
        let _ = sex_html::url::fragment(text);
    }
});
