// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_extensions::{clean_path, Manifest, Pattern};

fn test_clean_path(s: &str) {
    if let Some(cleaned) = clean_path(s) {
        if std::env::var_os("SEXOS_FUZZ_STRICT_SECURITY").is_some() {
            assert!(
                !cleaned.is_empty()
                    && !cleaned.ends_with('/')
                    && !cleaned.contains("//")
                    && !cleaned.split('/').any(|seg| seg.is_empty() || seg == "." || seg == ".."),
                "clean_path invariant violated for input {:?} -> {:?}",
                s,
                cleaned
            );
        }
    }
}

fn exercise_manifest(manifest_str: &str, locale_str: &str, test_url: &str, pattern_str: &str) {
    if let Ok(mut manifest) = Manifest::parse(manifest_str) {
        if let Some(loc) = manifest.locale_file() {
            if std::env::var_os("SEXOS_FUZZ_STRICT_SECURITY").is_some() {
                assert!(
                    clean_path(&loc).is_some()
                        && !loc.contains("..")
                        && !loc.contains('\\')
                        && !loc.contains('\0'),
                    "locale_file invariant failed for loc: {:?}",
                    loc
                );
            }
            manifest.localize(locale_str);
        }

        for r in &manifest.rulesets {
            test_clean_path(&r.path);
        }
        for bg in &manifest.background {
            test_clean_path(bg);
        }
        if let Some(p) = &manifest.popup {
            test_clean_path(p);
        }
        if let Some(o) = &manifest.options_page {
            test_clean_path(o);
        }
        for cs in &manifest.content_scripts {
            for j in &cs.js {
                test_clean_path(j);
            }
            for c in &cs.css {
                test_clean_path(c);
            }
            let _ = cs.applies(test_url, false);
            let _ = cs.applies(test_url, true);
            let _ = cs.applies("https://sub.domain.org/path/file.html", false);
        }

        for pat in &manifest.host_permissions {
            let _ = pat.matches(test_url);
        }
    }

    if let Some(pat) = Pattern::parse(pattern_str) {
        let _ = pat.matches(test_url);
        let _ = pat.matches("https://example.com/index.html");
        let _ = pat.matches("http://localhost:8080/test?q=1#hash");
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Raw text mode
    if let Ok(text) = core::str::from_utf8(data) {
        if text.contains('\0') {
            let parts: Vec<&str> = text.splitn(5, '\0').collect();
            let manifest_input = parts[0];
            let locale_input = if parts.len() > 1 { parts[1] } else { "{}" };
            let url_input = if parts.len() > 2 { parts[2] } else { "https://example.com/" };
            let pattern_input = if parts.len() > 3 { parts[3] } else { "*://*.example.com/*" };
            if parts.len() > 4 {
                test_clean_path(parts[4]);
            }
            exercise_manifest(manifest_input, locale_input, url_input, pattern_input);
        } else {
            exercise_manifest(text, "{}", "https://example.com/", "*://*.example.com/*");
        }
    }

    // 2. Structured JSON manifest generation
    let mode = data[0] % 6;
    let payload = &data[1..];
    let payload_str = core::str::from_utf8(payload).unwrap_or("test");

    let version = if (data[0] & 0x01) == 0 { 3 } else { 2 };
    let has_bom = (data[0] & 0x02) != 0;
    let has_comments = (data[0] & 0x04) != 0;

    let mut json = String::new();
    if has_bom {
        json.push('\u{feff}');
    }
    if has_comments {
        json.push_str("// Comment at start of manifest\n");
    }

    let default_loc = match mode {
        0 => "en",
        1 => payload_str,
        2 => "../../../../secret",
        3 => "zh_CN",
        4 => "es",
        _ => "en_US",
    };

    let background_field = if version == 3 {
        format!(r#""background": {{ "service_worker": "worker_{}.js" }},"#, payload_str)
    } else {
        format!(r#""background": {{ "scripts": ["bg1.js", "bg_{}.js"], "page": "bg.html" }},"#, payload_str)
    };

    let action_field = if version == 3 {
        format!(r#""action": {{ "default_popup": "popup_{}.html" }},"#, payload_str)
    } else {
        format!(r#""browser_action": {{ "default_popup": "popup_{}.html" }},"#, payload_str)
    };

    json.push_str(&format!(
        r#"{{
            "manifest_version": {},
            "name": "__MSG_appName__",
            "version": "1.2.3",
            "description": "__MSG_appDesc__",
            "default_locale": "{}",
            {}
            {}
            "options_page": "options.html",
            "options_ui": {{ "page": "options_ui.html" }},
            "permissions": ["storage", "alarms", "https://*.google.com/*", "<all_urls>"],
            "host_permissions": ["*://*.github.com/*", "http://localhost:*/*"],
            "content_scripts": [
                {{
                    "matches": ["*://*.example.com/*", "<all_urls>"],
                    "exclude_matches": ["*://*.bad.com/*"],
                    "js": ["content.js", "cs_{}.js"],
                    "css": ["style.css"],
                    "run_at": "document_start",
                    "all_frames": true
                }}
            ],
            "declarative_net_request": {{
                "rule_resources": [
                    {{ "id": "rules1", "enabled": true, "path": "rules.json" }}
                ]
            }}
        }}"#,
        version,
        default_loc,
        background_field,
        action_field,
        payload_str
    ));

    if has_comments {
        json.push_str("\n// Comment at end of manifest\n");
    }

    let locale_json = format!(
        r#"{{
            "appName": {{ "message": "Localized Name: {}" }},
            "appDesc": {{ "message": "Localized Description" }}
        }}"#,
        payload_str
    );

    let test_url = format!("https://sub.{}.example.com/path?q=test#hash", payload_str);
    let pattern_str = format!("*://*.{}.example.com/*", payload_str);

    exercise_manifest(&json, &locale_json, &test_url, &pattern_str);
});
