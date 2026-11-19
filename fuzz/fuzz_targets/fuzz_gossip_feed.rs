// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Direct byte parsing (RSS / Atom / RDF)
    if let Ok(feed) = sex_feed::parse_bytes(data) {
        exercise_feed(&feed, data);
    }

    // 2. Adversarial encoding envelopes
    // Malformed UTF-16 LE BOM with odd byte count
    let mut utf16_le = vec![0xFF, 0xFE];
    utf16_le.extend_from_slice(data);
    if let Ok(feed) = sex_feed::parse_bytes(&utf16_le) {
        exercise_feed(&feed, data);
    }

    // Malformed UTF-16 BE BOM with odd byte count
    let mut utf16_be = vec![0xFE, 0xFF];
    utf16_be.extend_from_slice(data);
    if let Ok(feed) = sex_feed::parse_bytes(&utf16_be) {
        exercise_feed(&feed, data);
    }

    // windows-1251 declaration envelope
    let mut cp1251 = b"<?xml version=\"1.0\" encoding=\"windows-1251\"?>\n".to_vec();
    cp1251.extend_from_slice(data);
    if let Ok(feed) = sex_feed::parse_bytes(&cp1251) {
        exercise_feed(&feed, data);
    }

    // latin1 declaration envelope
    let mut latin1 = b"<?xml version=\"1.0\" encoding=\"iso-8859-1\"?>\n".to_vec();
    latin1.extend_from_slice(data);
    if let Ok(feed) = sex_feed::parse_bytes(&latin1) {
        exercise_feed(&feed, data);
    }

    // 3. String-level parsing if valid UTF-8
    if let Ok(s) = core::str::from_utf8(data) {
        if let Ok(feed) = sex_feed::parse(s) {
            exercise_feed(&feed, data);
        }

        // OPML import / export round-trip
        let subs = sex_feed::opml::import(s);
        let exported = sex_feed::opml::export(&subs);
        let _ = sex_feed::opml::import(&exported);

        // Subscriptions store parsing and serialization
        let subscriptions = sex_feed::store::Subscriptions::parse(s);
        let serialized_subs = subscriptions.serialize();
        let _ = sex_feed::store::Subscriptions::parse(&serialized_subs);

        // ReadState store parsing and manipulation
        let mut read_state = sex_feed::store::ReadState::parse(s);
        let serialized_read = read_state.serialize();
        let _ = sex_feed::store::ReadState::parse(&serialized_read);

        for sub in subs.iter().take(4) {
            let _ = read_state.is_read(&sub.url, &sub.title);
            read_state.set(&sub.url, &sub.title, true);
            read_state.set(&sub.url, &sub.title, false);
            read_state.forget(&sub.url);
        }

        // Gossip UI text operations (apps/gossip/src/draw.rs and main.rs)
        let _ = gossip_clip(s, 0);
        let _ = gossip_clip(s, 1);
        let _ = gossip_clip(s, data[0] as usize);
        let _ = gossip_wrap(s, 0);
        let _ = gossip_wrap(s, 1);
        let _ = gossip_wrap(s, (data[0] % 80) as usize);
    }
});

// Mirror of apps/gossip/src/draw.rs:86
fn gossip_clip(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}

// Mirror of apps/gossip/src/main.rs:74
fn gossip_wrap(text: &str, cols: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.len() + 1 + word.len() > cols {
                out.push(line);
                line = String::new();
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

fn exercise_feed(feed: &sex_feed::Feed, data: &[u8]) {
    let _ = &feed.title;
    let _ = &feed.link;
    let preview_len = if !data.is_empty() { data[0] as usize } else { 0 };

    for item in feed.items.iter().take(16) {
        let text = item.text();
        let _ = &item.id;
        let _ = &item.title;
        let _ = &item.link;
        let _ = &item.date;
        let _ = &item.summary;

        // Fuzz arbitrary preview boundaries on text
        let _ = gossip_clip(&text, 0);
        let _ = gossip_clip(&text, 1);
        let _ = gossip_clip(&text, 2);
        let _ = gossip_clip(&text, preview_len);

        let _ = gossip_wrap(&text, 0);
        let _ = gossip_wrap(&text, 1);
        let _ = gossip_wrap(&text, (preview_len % 80).max(1));

        if let Some(enc) = &item.enclosure {
            let _ = (&enc.url, &enc.mime, enc.length);
        }
    }
}
