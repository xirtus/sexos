// SPDX-License-Identifier: GPL-3.0-only
//! `ipp://`, `ipps://`, `http://` and `https://` addresses.
use alloc::format;
use alloc::string::{String, ToString};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    /// Starts with `/`.
    pub path: String,
}

impl Url {
    pub fn parse(s: &str) -> Option<Url> {
        let (scheme, rest) = s.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let default = match scheme.as_str() { "ipp" => 631, "ipps" => 631, "http" => 80, "https" => 443, _ => return None };
        let (authority, path) = match rest.find('/') { Some(i) => (&rest[..i], &rest[i..]), None => (rest, "/") };
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']')?;
            (h, after.strip_prefix(':'))
        } else {
            match authority.rsplit_once(':') { Some((h, p)) => (h, Some(p)), None => (authority, None) }
        };
        if host.is_empty() { return None; }
        let port = match port { Some(p) => p.parse().ok()?, None => default };
        Some(Url { scheme, host: host.to_string(), port, path: path.to_string() })
    }

    /// Whether the transport is TLS (ipps/https): not spoken by Needle yet.
    pub fn tls(&self) -> bool { self.scheme == "ipps" || self.scheme == "https" }

    /// The `Host:` header value.
    pub fn host_header(&self) -> String {
        let h = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        format!("{h}:{}", self.port)
    }

    /// A path or absolute URL from a `Location:` header, against this URL.
    pub fn join(&self, location: &str) -> Option<Url> {
        if location.contains("://") { return Url::parse(location); }
        let path = if location.starts_with('/') {
            location.to_string()
        } else {
            let base = &self.path[..self.path.rfind('/').map_or(0, |i| i + 1)];
            format!("{base}{location}")
        };
        Some(Url { path, ..self.clone() })
    }
}

impl core::fmt::Display for Url {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}://{}{}", self.scheme, self.host_header(), self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_and_join() {
        let u = Url::parse("ipp://10.0.2.2:8631/ipp/print").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("10.0.2.2", 8631, "/ipp/print"));
        assert_eq!(Url::parse("ipp://printer.local").unwrap().port, 631);
        let v6 = Url::parse("http://[fd00::1]:80/eSCL").unwrap();
        assert_eq!(v6.host, "fd00::1");
        assert_eq!(v6.host_header(), "[fd00::1]:80");
        let e = Url::parse("http://h:9/eSCL").unwrap();
        assert_eq!(e.join("/eSCL/ScanJobs/1").unwrap().path, "/eSCL/ScanJobs/1");
        assert_eq!(e.join("http://h:9/eSCL/ScanJobs/2").unwrap().path, "/eSCL/ScanJobs/2");
        assert!(Url::parse("ftp://x/").is_none());
    }
}
