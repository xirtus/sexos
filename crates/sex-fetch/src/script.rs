// SPDX-License-Identifier: GPL-3.0-only
//! Classic script response checks. Execution stays a browser resource operation;
//! these checks never grant fetch/XHR access to the response's bytes.
//! References: WHATWG Fetch CORP/MIME blocking and HTML classic script fetching.
use crate::Url;

fn header<'a>(lines: &'a str, name: &str) -> Option<&'a str> {
    lines
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
}
fn javascript(mime: &str) -> bool {
    matches!(
        mime,
        "application/ecmascript"
            | "application/javascript"
            | "application/x-ecmascript"
            | "application/x-javascript"
            | "text/ecmascript"
            | "text/javascript"
            | "text/javascript1.0"
            | "text/javascript1.1"
            | "text/javascript1.2"
            | "text/javascript1.3"
            | "text/javascript1.4"
            | "text/javascript1.5"
            | "text/jscript"
            | "text/livescript"
            | "text/x-ecmascript"
            | "text/x-javascript"
    )
}

/// Whether a successful HTTP classic-script response may be executed. `cors`
/// means the normal CORS path has already checked ACAO; default classic resources
/// instead honor CORP. Cross-origin non-JavaScript types are refused defensively.
pub fn allowed(page: &str, response: &str, headers: &str, cors: bool) -> bool {
    let Ok(response) = Url::parse(response) else {
        return false;
    };
    let page = Url::parse(page).ok(); // built-in/file pages have an opaque origin
    if page.as_ref().is_some_and(|p| p.secure) && !response.secure {
        return false;
    }
    let same_origin = page
        .as_ref()
        .is_some_and(|p| p.origin() == response.origin());
    let mime = header(headers, "content-type")
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let js = javascript(&mime);
    let nosniff = header(headers, "x-content-type-options").is_some_and(|v| {
        v.split(',')
            .any(|v| v.trim().eq_ignore_ascii_case("nosniff"))
    });
    if nosniff && !js
        || !same_origin && !js
        || mime.starts_with("image/")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || matches!(mime.as_str(), "text/csv" | "text/html" | "application/json")
    {
        return false;
    }
    if cors {
        return true;
    }
    match header(headers, "cross-origin-resource-policy") {
        Some("same-origin") => same_origin,
        Some("same-site") => {
            let Some(page) = page else { return false };
            let literal =
                |h: &str| h.starts_with('[') || h.bytes().all(|b| b.is_ascii_digit() || b == b'.');
            let hosts = page.host == response.host
                || !literal(&page.host)
                    && !literal(&response.host)
                    && match (psl::domain_str(&page.host), psl::domain_str(&response.host)) {
                        (Some(a), Some(b)) => a == b,
                        _ => false,
                    };
            // CORP's schemeless site comparison has this HTTPS protection:
            // an HTTP initiator cannot read an HTTPS response under same-site.
            hosts && (page.secure || !response.secure)
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::borrow::ToOwned;
    const JS: &str = "Content-Type: application/javascript; charset=utf-8\r\n";
    #[test]
    fn classic_policy_does_not_require_acao_but_honors_mime() {
        assert!(allowed("https://a.test/", "https://b.test/x.js", JS, false));
        for mime in ["text/html", "application/json", "image/png", "text/csv", ""] {
            assert!(!allowed(
                "https://a.test/",
                "https://b.test/x",
                &alloc::format!("content-type: {mime}\r\n"),
                false
            ));
        }
        assert!(allowed("https://a.test/", "https://a.test/x", "", false));
        assert!(allowed("about:welcome", "https://b.test/x.js", JS, false));
        assert!(!allowed(
            "about:welcome",
            "https://b.test/x.js",
            &(JS.to_owned() + "Cross-Origin-Resource-Policy: same-origin\r\n"),
            false
        ));
        assert!(!allowed(
            "https://a.test/",
            "https://a.test/x",
            "X-Content-Type-Options: nosniff\r\n",
            false
        ));
    }
    #[test]
    fn corp_is_checked_against_final_origin_and_real_public_suffix() {
        assert!(!allowed(
            "https://a.test/",
            "https://b.test/x.js",
            &(JS.to_owned() + "Cross-Origin-Resource-Policy: same-origin\r\n"),
            false
        ));
        let site = JS.to_owned() + "Cross-Origin-Resource-Policy: same-site\r\n";
        assert!(allowed(
            "https://a.example.co.uk/",
            "https://b.example.co.uk/x.js",
            &site,
            false
        ));
        assert!(!allowed(
            "https://10.0.2.2/",
            "https://192.0.2.2/x.js",
            &site,
            false
        ));
        assert!(!allowed(
            "https://a.example.co.uk/",
            "https://evil.co.uk/x.js",
            &site,
            false
        ));
        assert!(!allowed(
            "http://a.example.com/",
            "https://b.example.com/x.js",
            &site,
            false
        ));
        assert!(!allowed(
            "https://a.example.com/",
            "http://b.example.com/x.js",
            &site,
            false
        ));
        assert!(allowed(
            "https://a.test/",
            "https://b.test/x.js",
            &(JS.to_owned() + "Cross-Origin-Resource-Policy: same-origin\r\n"),
            true
        ));
    }
}
