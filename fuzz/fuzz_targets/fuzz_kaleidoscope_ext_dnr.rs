// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use sex_extensions::dnr;
use sex_shields::{Engine, Kind, Request, Verdict};

fn exercise_dnr(json_str: &str, test_url: &str, test_page: &str, kind_byte: u8) {
    if let Ok(converted) = dnr::convert(json_str) {
        let mut engine = Engine::new();

        if std::env::var_os("SEXOS_FUZZ_STRICT_SECURITY").is_some() {
            for filter in &converted.filters {
                assert!(
                    !filter.contains("#@#")
                        && !filter.contains("##")
                        && !filter.contains("#?#")
                        && !filter.contains("#$#")
                        && !filter.contains("#%#")
                        && !filter.contains("#@?#")
                        && !filter.contains("#@$#"),
                    "DNR smuggled cosmetic syntax: {:?}",
                    filter
                );

                if let Some((_, opts_part)) = filter.split_once('$') {
                    for opt in opts_part.split(',') {
                        assert!(
                            opt != "badfilter"
                                && opt != "important"
                                && opt != "elemhide"
                                && opt != "generichide",
                            "DNR injected unrequested filter option {:?} in {:?}",
                            opt,
                            filter
                        );
                    }
                }
            }
        }

        for filter in &converted.filters {
            let _ = engine.add_filter(filter);
        }

        if std::env::var_os("SEXOS_FUZZ_STRICT_SECURITY").is_some() {
            assert_eq!(
                engine.counts().cosmetic,
                0,
                "DNR ruleset smuggled cosmetic filters into engine: count={}",
                engine.counts().cosmetic
            );
        }

        let kind = match kind_byte % 12 {
            0 => Kind::Document,
            1 => Kind::Subdocument,
            2 => Kind::Script,
            3 => Kind::Stylesheet,
            4 => Kind::Image,
            5 => Kind::Font,
            6 => Kind::Media,
            7 => Kind::Xhr,
            8 => Kind::WebSocket,
            9 => Kind::Ping,
            10 => Kind::Object,
            _ => Kind::Other,
        };

        let req = Request {
            url: test_url,
            page: test_page,
            kind,
        };

        match engine.check(&req) {
            Verdict::Block(id) => {
                let _ = engine.filter_text(id);
            }
            Verdict::Excepted(id) => {
                let _ = engine.filter_text(id);
            }
            Verdict::Pass => {}
        }

        let _ = engine.page_exceptions(test_page);

        let has_class = |name: &str| name == "ad" || name == "banner" || name.starts_with("sponsor");
        let has_id = |name: &str| name == "promo" || name == "popup" || name.ends_with("_ad");

        let _ = engine.hidden(test_page, &has_class, &has_id);
        let _ = engine.css(test_page, &has_class, &has_id);
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Raw JSON string fuzzing
    if let Ok(text) = core::str::from_utf8(data) {
        if text.contains('\0') {
            let parts: Vec<&str> = text.splitn(3, '\0').collect();
            let json_input = parts[0];
            let url_input = if parts.len() > 1 { parts[1] } else { "https://tracker.example.com/ad.js" };
            let page_input = if parts.len() > 2 { parts[2] } else { "https://news.example.com/article" };
            let kind_byte = data[0];

            if let Ok(val) = serde_json::from_str::<Value>(json_input) {
                let _ = dnr::rule(&val);
            }
            exercise_dnr(json_input, url_input, page_input, kind_byte);
        } else {
            if let Ok(val) = serde_json::from_str::<Value>(text) {
                let _ = dnr::rule(&val);
            }
            exercise_dnr(text, "https://tracker.example.com/ad.js", "https://news.example.com/article", data[0]);
        }
    }

    // 2. Structured DNR ruleset generation
    let mode = data[0] % 8;
    let payload = &data[1..];
    let payload_str = core::str::from_utf8(payload).unwrap_or("tracker");

    let action_type = match mode % 4 {
        0 => "block",
        1 => "redirect",
        2 => "allow",
        _ => "allowAllRequests",
    };

    let domain_type = match (data[0] >> 2) % 3 {
        0 => Some("thirdParty"),
        1 => Some("firstParty"),
        _ => None,
    };

    let domain_type_field = if let Some(dt) = domain_type {
        format!(r#""domainType": "{}","#, dt)
    } else {
        String::new()
    };

    let case_sensitive = (data[0] & 0x08) != 0;

    let url_filter = if (data[0] & 0x10) != 0 {
        format!(r#""urlFilter": "||{}^","#, payload_str)
    } else if (data[0] & 0x20) != 0 {
        format!(r#""urlFilter": "{}","#, payload_str)
    } else {
        String::new()
    };

    let initiator_domains = match mode {
        4 => r#""initiatorDomains": ["evil.com,badfilter"],"#,
        5 => r#""initiatorDomains": ["site.com,important"],"#,
        _ => r#""initiatorDomains": ["example.com", "sub.example.org"],"#,
    };

    let structured_json = format!(
        r#"[
            {{
                "id": 1,
                "priority": 1,
                "action": {{ "type": "{}" }},
                "condition": {{
                    {}
                    {}
                    {}
                    "requestDomains": ["ads.{}.com", "metrics.{}.org"],
                    "excludedRequestDomains": ["good.{}.com"],
                    "excludedInitiatorDomains": ["safe.{}.com"],
                    "resourceTypes": ["script", "image", "xmlhttprequest"],
                    "excludedResourceTypes": ["stylesheet"],
                    "isUrlFilterCaseSensitive": {}
                }}
            }},
            {{
                "id": 2,
                "action": {{ "type": "allow" }},
                "condition": {{
                    "urlFilter": "@@||whitelisted.example.com^",
                    "resourceTypes": ["main_frame", "sub_frame"]
                }}
            }}
        ]"#,
        action_type,
        url_filter,
        domain_type_field,
        initiator_domains,
        payload_str,
        payload_str,
        payload_str,
        payload_str,
        case_sensitive
    );

    let test_url = format!("https://ads.{}.com/pixel.png", payload_str);
    let test_page = format!("https://{}.com/home", payload_str);
    exercise_dnr(&structured_json, &test_url, &test_page, data[0]);
});
