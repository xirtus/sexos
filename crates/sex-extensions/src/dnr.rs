// SPDX-License-Identifier: GPL-3.0-only
//! declarativeNetRequest (MV3) rules as Shields filters.
//!
//! A rule's condition maps onto filter syntax almost one for one: `urlFilter` is
//! already Adblock-style (`||`, `|`, `*`, `^`), `requestDomains` become `||d^`,
//! `initiatorDomains` `$domain=`, `resourceTypes` the type options, `domainType`
//! `$third-party`. Rules this can't express exactly (`regexFilter`, header changes,
//! scheme upgrades, request methods, tab ids) are counted in `skipped`, not guessed.
//! Rule priorities are not kept: exceptions win over blocks, as in Shields.

use alloc::string::String;
use alloc::vec::Vec;
use serde_json::Value;

/// The filters made from one ruleset file.
#[derive(Debug, Default)]
pub struct Converted { pub filters: Vec<String>, pub skipped: usize }

fn kind(t: &str) -> Option<&'static str> {
    Some(match t {
        "main_frame" => "document", "sub_frame" => "subdocument", "stylesheet" => "stylesheet", "script" => "script",
        "image" => "image", "font" => "font", "object" => "object", "xmlhttprequest" => "xmlhttprequest",
        "ping" => "ping", "media" => "media", "websocket" => "websocket", "other" | "csp_report" | "webtransport" | "webbundle" => "other",
        _ => return None,
    })
}

fn list<'a>(c: &'a Value, keys: &[&str]) -> Vec<&'a str> {
    keys.iter().filter_map(|k| c.get(*k)).filter_map(Value::as_array).flatten().filter_map(Value::as_str).collect()
}

/// Turns a ruleset file (a JSON array of rules) into filters.
pub fn convert(json: &str) -> Result<Converted, super::Error> {
    let v: Value = serde_json::from_str(json).map_err(|_| super::Error::Json)?;
    let rules = v.as_array().ok_or(super::Error::NotObject)?;
    let mut out = Converted::default();
    for r in rules {
        match rule(r) { Some(mut f) => out.filters.append(&mut f), None => out.skipped += 1 }
    }
    Ok(out)
}

/// One rule's filters, or `None` when it can't be expressed.
pub fn rule(r: &Value) -> Option<Vec<String>> {
    let action = r.get("action")?.get("type")?.as_str()?;
    let c = r.get("condition")?;
    let (prefix, document) = match action {
        "block" | "redirect" => ("", false),
        "allow" => ("@@", false),
        "allowAllRequests" => ("@@", true),
        _ => return None,
    };
    for unsupported in ["regexFilter", "requestMethods", "excludedRequestMethods", "tabIds", "excludedTabIds", "responseHeaders", "excludedResponseHeaders"] {
        if c.get(unsupported).is_some() { return None; }
    }
    let url_filter = c.get("urlFilter").and_then(Value::as_str);
    if url_filter.is_some_and(|u| u.contains('$') || u.contains("##") || u.is_empty()) { return None; }
    let mut opts: Vec<String> = Vec::new();
    let types = list(c, &["resourceTypes"]);
    let excluded = list(c, &["excludedResourceTypes"]);
    if document { opts.push(String::from("document")); }
    else {
        for t in &types { opts.push(String::from(kind(t)?)); }
        for t in &excluded { opts.push(alloc::format!("~{}", kind(t)?)); }
    }
    match c.get("domainType").and_then(Value::as_str) {
        Some("thirdParty") => opts.push(String::from("third-party")),
        Some("firstParty") => opts.push(String::from("~third-party")),
        _ => {}
    }
    let on = list(c, &["initiatorDomains", "domains"]);
    let off = list(c, &["excludedInitiatorDomains", "excludedDomains"]);
    if !on.is_empty() || !off.is_empty() {
        let mut d: Vec<String> = on.iter().map(|x| String::from(*x)).collect();
        d.extend(off.iter().map(|x| alloc::format!("~{}", x)));
        opts.push(alloc::format!("domain={}", d.join("|")));
    }
    let to_off = list(c, &["excludedRequestDomains"]);
    if !to_off.is_empty() { opts.push(alloc::format!("to={}", to_off.iter().map(|x| alloc::format!("~{}", x)).collect::<Vec<_>>().join("|"))); }
    if c.get("isUrlFilterCaseSensitive").and_then(Value::as_bool) == Some(true) { opts.push(String::from("match-case")); }
    let to = list(c, &["requestDomains"]);
    if url_filter.is_some() && !to.is_empty() { opts.push(alloc::format!("to={}", to.join("|"))); }
    let with = |pattern: &str| -> String {
        let mut f = String::from(prefix);
        f.push_str(pattern);
        if !opts.is_empty() { f.push('$'); f.push_str(&opts.join(",")); }
        f
    };
    Some(match (url_filter, to.is_empty()) {
        (Some(u), true) => alloc::vec![with(u)],
        // A filter and request domains: each domain as a `to=` alternative.
        (Some(u), false) => alloc::vec![with(u)],
        (None, false) => to.iter().map(|d| with(&alloc::format!("||{}^", d))).collect(),
        // No address condition: the options alone (on these sites, these kinds).
        (None, true) => alloc::vec![with("")],
    })
}
