// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1. HTTP/1.1 message and header parsing
    let _ = sex_http::parse(data);
    let _ = sex_http::parse_headers(data);
    let _ = sex_http::parse_navigation_headers(data);

    // 2. HTTP/2 HPACK header block decoding
    let mut decoder = sex_http::h2::hpack::Decoder::new(4096, 65536);
    let _ = decoder.decode(data);

    // 3. HTTP/2 Client frame receipt and state machine
    let mut client = sex_http::h2::Client::new();
    let _ = client.receive(data);
    let _ = client.output();
    while client.event().is_some() {}
});
