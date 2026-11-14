// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1. RFC 5322 & MIME message parsing
    let msg = sex_mail::message::parse(data);
    let _ = msg.subject();
    let _ = msg.from();
    let _ = msg.message_id();
    let _ = msg.in_reply_to();
    let _ = msg.references();
    let _ = msg.has_remote_content();

    // 2. Message header/body splitting
    let (headers, body) = sex_mail::message::split(data);
    for (k, _) in &headers.0 {
        let _ = headers.text(k);
    }

    // 3. Base64 MIME decoder
    let _ = sex_mail::b64::decode(body);
});
