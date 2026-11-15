// SPDX-License-Identifier: GPL-3.0-only
#![no_main]

use libfuzzer_sys::fuzz_target;
use sex_extensions::{dnr, package, Manifest};
use sex_shields::Engine;

fn make_zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();

    for (name, data, compress) in entries {
        let offset = out.len() as u32;
        let crc = crc32fast::hash(data);
        let name_bytes = name.as_bytes();

        let (method, payload, packed_size): (u16, Vec<u8>, u32) = if *compress {
            let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
            let len = deflated.len() as u32;
            (8, deflated, len)
        } else {
            (0, data.to_vec(), data.len() as u32)
        };

        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // time & date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&packed_size.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(&payload);

        cd.extend_from_slice(b"PK\x01\x02");
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&20u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&method.to_le_bytes());
        cd.extend_from_slice(&0u32.to_le_bytes());
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&packed_size.to_le_bytes());
        cd.extend_from_slice(&(data.len() as u32).to_le_bytes());
        cd.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u16.to_le_bytes());
        cd.extend_from_slice(&0u32.to_le_bytes());
        cd.extend_from_slice(&offset.to_le_bytes());
        cd.extend_from_slice(name_bytes);
    }

    let cd_offset = out.len() as u32;
    let cd_size = cd.len() as u32;
    out.extend_from_slice(&cd);

    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());

    out
}

fn exercise_package(data: &[u8]) {
    let _ = package::zip_of(data);

    if let Ok(files) = package::unpack(data) {
        if std::env::var_os("SEXOS_FUZZ_STRICT_SECURITY").is_some() {
            for (path, _) in &files {
                assert!(
                    !path.is_empty()
                        && !path.ends_with('/')
                        && !path.contains("..")
                        && !path.contains("//")
                        && sex_extensions::clean_path(path).as_deref() == Some(path.as_str()),
                    "Security invariant violated for unpacked path: {:?}",
                    path
                );
            }
        }

        // Simulate Kaleidoscope install_extension + Ext::load
        if let Some((_, manifest_bytes)) = files.iter().find(|(p, _)| p == "manifest.json") {
            if let Ok(manifest_str) = core::str::from_utf8(manifest_bytes) {
                if let Ok(mut manifest) = Manifest::parse(manifest_str) {
                    if let Some(loc) = manifest.locale_file() {
                        if let Some((_, m_bytes)) = files.iter().find(|(p, _)| p == &loc) {
                            if let Ok(m_str) = core::str::from_utf8(m_bytes) {
                                manifest.localize(m_str);
                            }
                        }
                    }

                    let sets: Vec<String> = manifest
                        .rulesets
                        .iter()
                        .filter(|r| r.enabled)
                        .map(|r| r.path.clone())
                        .collect();
                    if !sets.is_empty() {
                        let mut engine = Engine::new();
                        for path in sets {
                            if let Some((_, rule_bytes)) = files.iter().find(|(p, _)| p == &path) {
                                if let Ok(json) = core::str::from_utf8(rule_bytes) {
                                    if let Ok(converted) = dnr::convert(json) {
                                        for filter in &converted.filters {
                                            let _ = engine.add_filter(filter);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    // 1. Raw bytes fuzzing
    exercise_package(data);

    // 2. Structured synthetic ZIP and CRX packages
    let mode = data[0] % 5;
    let payload = &data[1..];

    let manifest_sample = br#"{
        "manifest_version": 3,
        "name": "__MSG_extName__",
        "version": "1.0.0",
        "default_locale": "en",
        "declarative_net_request": {
            "rule_resources": [
                { "id": "ruleset_1", "enabled": true, "path": "rules.json" }
            ]
        }
    }"#;

    let locale_sample = br#"{
        "extName": { "message": "Fuzz Extension" }
    }"#;

    let rules_sample = br#"[
        {
            "id": 1,
            "action": { "type": "block" },
            "condition": { "urlFilter": "||ads.example.com^" }
        }
    ]"#;

    let compress = (data[0] & 0x08) != 0;

    let (manifest_data, custom_name, custom_data) = if payload.len() > 16 {
        let (left, right) = payload.split_at(payload.len() / 2);
        let name_str = core::str::from_utf8(&left[..left.len().min(32)]).unwrap_or("extra.txt");
        (&manifest_sample[..], name_str, right)
    } else {
        (payload, "extra.txt", payload)
    };

    let entries = [
        ("manifest.json", manifest_data, compress),
        ("_locales/en/messages.json", &locale_sample[..], compress),
        ("rules.json", &rules_sample[..], compress),
        (custom_name, custom_data, compress),
    ];

    let zip_bytes = make_zip(&entries);

    match mode {
        0 => {
            // Raw structured ZIP
            exercise_package(&zip_bytes);
        }
        1 => {
            // Synthetic CRX2
            let mut crx2 = Vec::new();
            crx2.extend_from_slice(b"Cr24");
            crx2.extend_from_slice(&2u32.to_le_bytes()); // version 2
            let pubkey = b"synthetic_pubkey_12345678";
            let sig = b"synthetic_sig_87654321";
            crx2.extend_from_slice(&(pubkey.len() as u32).to_le_bytes());
            crx2.extend_from_slice(&(sig.len() as u32).to_le_bytes());
            crx2.extend_from_slice(pubkey);
            crx2.extend_from_slice(sig);
            crx2.extend_from_slice(&zip_bytes);
            exercise_package(&crx2);
        }
        2 => {
            // Synthetic CRX3
            let mut crx3 = Vec::new();
            crx3.extend_from_slice(b"Cr24");
            crx3.extend_from_slice(&3u32.to_le_bytes()); // version 3
            let header = b"synthetic_crx3_header_bytes";
            crx3.extend_from_slice(&(header.len() as u32).to_le_bytes());
            crx3.extend_from_slice(header);
            crx3.extend_from_slice(&zip_bytes);
            exercise_package(&crx3);
        }
        3 => {
            // Wrapped in a top-level directory (as in GitHub release zips)
            let wrapped_entries = [
                ("wrapper_dir/manifest.json", manifest_data, compress),
                ("wrapper_dir/_locales/en/messages.json", &locale_sample[..], compress),
                ("wrapper_dir/rules.json", &rules_sample[..], compress),
                ("wrapper_dir/extra.txt", custom_data, compress),
            ];
            let wrapped_zip = make_zip(&wrapped_entries);
            exercise_package(&wrapped_zip);
        }
        _ => {
            // Mutated header + zip
            let mut mutated = zip_bytes;
            if !mutated.is_empty() && payload.len() >= 4 {
                mutated[0] = payload[0];
                mutated[1] = payload[1];
            }
            exercise_package(&mutated);
        }
    }
});
