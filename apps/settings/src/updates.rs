// SPDX-License-Identifier: GPL-3.0-only
//! Pure, zero-heap helpers for Tatami's Live OTA System Updater (`xirtus/sexos`)
//! and user-configurable Private Dotfiles Sync (`dotfiles.conf`).
//!
//! Host-tested directly via `scripts/settings_host_gate.sh`.

/// Public upstream SexOS repository (GitHub Releases feed).
#[allow(dead_code)]
pub const SYSTEM_REPO: &str = "xirtus/sexos";
/// Default GitHub Releases download base URL for `sexos-update.txt`.
pub const DEFAULT_FEED_URL: &str = "https://github.com/xirtus/sexos/releases/latest/download/";
/// Default branch for user dotfiles repositories.
pub const DEFAULT_BRANCH: &str = "main";
/// Placeholder shown in the Dotfiles Repo input (never a hardcoded private repo).
pub const DOTFILES_PLACEHOLDER: &str = "owner/sexosdotfiles";

/// Maximum bytes read when fetching `sexos-update.txt` (4 KiB bounded).
pub const UPDATE_INDEX_MAX_BYTES: usize = 4096;
/// Maximum bytes read per dotfile during GitHub raw sync (256 KiB bounded).
pub const DOTFILE_MAX_BYTES: usize = 256 * 1024;
/// Maximum HTTP redirects to follow for update checks and dotfiles fetches.
pub const MAX_REDIRECTS: usize = 5;

/// Candidate paths for Tatami's local dotfiles configuration file.
/// Relative `dotfiles.conf` resolves to `/data/tatami/dotfiles.conf` in confined mode.
pub const DOTFILES_CONF_PATHS: [&str; 3] = [
    "dotfiles.conf",
    "@/data/tatami/dotfiles.conf",
    "/data/tatami/dotfiles.conf",
];

/// A known dotfile path synced from `raw.githubusercontent.com/<owner>/<repo>/<branch>/<remote_path>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DotfileTarget {
    pub remote_path: &'static str,
    pub local_paths: &'static [&'static str],
    pub mkdir_dir: Option<&'static str>,
}

/// Known SexOS dotfiles synced from a user's dotfiles repository.
pub const DOTFILES_TARGETS: [DotfileTarget; 3] = [
    DotfileTarget {
        remote_path: "brootlite/provider_pack.json",
        local_paths: &[
            "@/data/brootlite/provider_pack.json",
            "/data/brootlite/provider_pack.json",
        ],
        mkdir_dir: Some("/data/brootlite"),
    },
    DotfileTarget {
        remote_path: "tailor/proxy.conf",
        local_paths: &["@/data/tailor/proxy.conf", "/data/tailor/proxy.conf"],
        mkdir_dir: Some("/data/tailor"),
    },
    DotfileTarget {
        remote_path: "wifi.conf",
        local_paths: &["@/data/wifi.conf", "/data/wifi.conf"],
        mkdir_dir: None,
    },
];

/// Parsed `sexos-update.txt` metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateIndex {
    pub version: u64,
    pub bytes: u64,
}

/// Outcome of comparing a remote `sexos-update.txt` against the local installed version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCheckResult {
    None,
    Available {
        remote_version: u64,
        bytes: u64,
        local_version: u64,
    },
    UpToDate {
        local_version: u64,
        remote_version: u64,
    },
    Error,
}

/// Parses `sexos-update.txt` (`key=value` lines) for `version=<N>` and `bytes=<B>`.
pub fn parse_update_index(data: &[u8]) -> Option<UpdateIndex> {
    let mut version: Option<u64> = None;
    let mut bytes: Option<u64> = None;
    for raw_line in data.split(|&b| b == b'\n') {
        let line = trim_ascii_bytes(raw_line);
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        if let Some(val) = line.strip_prefix(b"version=") {
            version = parse_u64_strict(trim_ascii_bytes(val));
        } else if let Some(val) = line.strip_prefix(b"bytes=") {
            bytes = parse_u64_strict(trim_ascii_bytes(val));
        }
    }
    Some(UpdateIndex {
        version: version?,
        bytes: bytes?,
    })
}

/// Extracts `version=<N>` from `sex_pdx::facts::UPDATES` response bytes.
pub fn parse_local_version(facts_data: &[u8]) -> Option<u64> {
    for raw_line in facts_data.split(|&b| b == b'\n') {
        let line = trim_ascii_bytes(raw_line);
        if let Some(val) = line.strip_prefix(b"version=") {
            return parse_u64_strict(trim_ascii_bytes(val));
        }
    }
    None
}

/// Compares `index.version` against `local_version`.
pub const fn compare_update(local_version: u64, index: UpdateIndex) -> UpdateCheckResult {
    if index.version > local_version {
        UpdateCheckResult::Available {
            remote_version: index.version,
            bytes: index.bytes,
            local_version,
        }
    } else {
        UpdateCheckResult::UpToDate {
            local_version,
            remote_version: index.version,
        }
    }
}

/// Converts a byte size to rounded MiB (minimum 1 MiB for non-zero sizes < 1 MiB).
pub const fn bytes_to_mib(bytes: u64) -> u64 {
    if bytes == 0 {
        0
    } else {
        let rounded = bytes.saturating_add(524_288) / 1_048_576;
        if rounded == 0 { 1 } else { rounded }
    }
}

/// Formats the Candy Lavender alert banner text for an `UpdateCheckResult`.
///
/// - Available: `"Update Available: SexOS v<N> (<M> MiB)"`
/// - Up to date: `"Up to date (v<local>)"`
pub fn format_update_banner(result: UpdateCheckResult, out: &mut [u8]) -> usize {
    let mut w = Writer::new(out);
    match result {
        UpdateCheckResult::Available {
            remote_version,
            bytes,
            ..
        } => {
            w.put(b"Update Available: SexOS v");
            w.put_u64(remote_version);
            w.put(b" (");
            w.put_u64(bytes_to_mib(bytes));
            w.put(b" MiB)");
        }
        UpdateCheckResult::UpToDate { local_version, .. } => {
            w.put(b"Up to date (v");
            w.put_u64(local_version);
            w.put(b")");
        }
        UpdateCheckResult::None | UpdateCheckResult::Error => {}
    }
    w.at
}

/// Formats `"Synced <N> file(s) from <owner/repo>"` into `out`.
pub fn format_synced_status(count: usize, repo: &str, out: &mut [u8]) -> usize {
    let mut w = Writer::new(out);
    w.put(b"Synced ");
    w.put_u64(count as u64);
    w.put(b" file(s) from ");
    w.put(repo.as_bytes());
    w.at
}

/// Builds the full `sexos-update.txt` URL from a base release feed URL.
pub fn build_update_index_url(feed_url: &str, out: &mut [u8]) -> Option<usize> {
    let trimmed = feed_url.trim();
    let base = if trimmed.is_empty() {
        DEFAULT_FEED_URL
    } else {
        trimmed
    };
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return None;
    }
    if base.bytes().any(|b| b <= 0x20 || b >= 0x7F) {
        return None;
    }
    let suffix = if base.ends_with("/sexos-update.txt") {
        ""
    } else if base.ends_with('/') {
        "sexos-update.txt"
    } else {
        "/sexos-update.txt"
    };
    let total = base.len().checked_add(suffix.len())?;
    if total > out.len() {
        return None;
    }
    out[..base.len()].copy_from_slice(base.as_bytes());
    out[base.len()..total].copy_from_slice(suffix.as_bytes());
    Some(total)
}

/// Validates a GitHub `owner/repo` string (single slash, safe ASCII, no traversal).
pub fn is_valid_repo(repo: &str) -> bool {
    let trimmed = repo.trim();
    let Some(slash) = trimmed.find('/') else {
        return false;
    };
    let owner = &trimmed[..slash];
    let name = &trimmed[slash + 1..];
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return false;
    }
    if owner.contains("..") || name.contains("..") {
        return false;
    }
    let ok_char = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.');
    owner.bytes().all(ok_char) && name.bytes().all(ok_char)
}

/// Validates a Git branch name (safe ASCII, no traversal or spaces).
pub fn is_valid_branch(branch: &str) -> bool {
    let trimmed = branch.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.contains("..")
    {
        return false;
    }
    trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
}

/// Builds `https://raw.githubusercontent.com/<owner>/<repo>/<branch>/<path>` into `out`.
pub fn build_raw_github_url(repo: &str, branch: &str, path: &str, out: &mut [u8]) -> Option<usize> {
    let repo = repo.trim();
    if !is_valid_repo(repo) {
        return None;
    }
    let branch = {
        let b = branch.trim();
        if b.is_empty() { DEFAULT_BRANCH } else { b }
    };
    if !is_valid_branch(branch) {
        return None;
    }
    let path = path.trim();
    if path.is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.bytes().any(|b| b <= 0x20 || b >= 0x7F)
    {
        return None;
    }
    let prefix = b"https://raw.githubusercontent.com/";
    let total = prefix.len() + repo.len() + 1 + branch.len() + 1 + path.len();
    if total > out.len() {
        return None;
    }
    let mut w = Writer::new(out);
    w.put(prefix);
    w.put(repo.as_bytes());
    w.put(b"/");
    w.put(branch.as_bytes());
    w.put(b"/");
    w.put(path.as_bytes());
    Some(w.at)
}

/// User-configurable dotfiles & release feed settings persisted in `/data/tatami/dotfiles.conf`.
///
/// Zero private URLs, usernames, or tokens are ever hardcoded in public code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DotfilesConfig {
    pub repo: [u8; 96],
    pub repo_len: usize,
    pub branch: [u8; 48],
    pub branch_len: usize,
    pub token: [u8; 128],
    pub token_len: usize,
    pub auto_sync: bool,
    pub feed_url: [u8; 160],
    pub feed_url_len: usize,
}

impl DotfilesConfig {
    pub const fn new() -> Self {
        Self {
            repo: [0; 96],
            repo_len: 0,
            branch: [0; 48],
            branch_len: 0,
            token: [0; 128],
            token_len: 0,
            auto_sync: false,
            feed_url: [0; 160],
            feed_url_len: 0,
        }
    }

    /// Parses `dotfiles.conf` (`key=value` lines).
    pub fn parse(data: &[u8]) -> Self {
        let mut cfg = Self::new();
        for raw_line in data.split(|&b| b == b'\n') {
            let line = trim_ascii_bytes(raw_line);
            if line.is_empty() || line.starts_with(b"#") {
                continue;
            }
            if let Some(v) = line.strip_prefix(b"repo=") {
                cfg.set_repo(trim_ascii_bytes(v));
            } else if let Some(v) = line.strip_prefix(b"branch=") {
                cfg.set_branch(trim_ascii_bytes(v));
            } else if let Some(v) = line.strip_prefix(b"token=") {
                cfg.set_token(trim_ascii_bytes(v));
            } else if let Some(v) = line.strip_prefix(b"auto_sync=") {
                let v = trim_ascii_bytes(v);
                cfg.auto_sync = v == b"1" || v == b"true" || v == b"yes";
            } else if let Some(v) = line.strip_prefix(b"feed_url=") {
                cfg.set_feed_url(trim_ascii_bytes(v));
            }
        }
        cfg
    }

    /// Serializes configuration to `dotfiles.conf` (`key=value\n` format).
    pub fn serialize(&self, out: &mut [u8]) -> usize {
        let mut w = Writer::new(out);
        w.put(b"repo=");
        w.put(&self.repo[..self.repo_len]);
        w.put(b"\nbranch=");
        w.put(self.branch_str().as_bytes());
        w.put(b"\ntoken=");
        w.put(&self.token[..self.token_len]);
        w.put(b"\nauto_sync=");
        w.put(if self.auto_sync { b"1" } else { b"0" });
        if self.feed_url_len > 0 {
            w.put(b"\nfeed_url=");
            w.put(&self.feed_url[..self.feed_url_len]);
        }
        w.put(b"\n");
        w.at
    }

    pub fn set_repo(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.repo.len());
        self.repo[..n].copy_from_slice(&bytes[..n]);
        self.repo_len = n;
    }

    pub fn set_branch(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.branch.len());
        self.branch[..n].copy_from_slice(&bytes[..n]);
        self.branch_len = n;
    }

    pub fn set_token(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.token.len());
        self.token[..n].copy_from_slice(&bytes[..n]);
        self.token_len = n;
    }

    pub fn set_feed_url(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.feed_url.len());
        self.feed_url[..n].copy_from_slice(&bytes[..n]);
        self.feed_url_len = n;
    }

    pub fn repo_str(&self) -> &str {
        core::str::from_utf8(&self.repo[..self.repo_len]).unwrap_or("")
    }

    pub fn branch_str(&self) -> &str {
        if self.branch_len == 0 {
            DEFAULT_BRANCH
        } else {
            core::str::from_utf8(&self.branch[..self.branch_len]).unwrap_or(DEFAULT_BRANCH)
        }
    }

    pub fn token_str(&self) -> &str {
        core::str::from_utf8(&self.token[..self.token_len]).unwrap_or("")
    }

    pub fn feed_url_str(&self) -> &str {
        if self.feed_url_len == 0 {
            DEFAULT_FEED_URL
        } else {
            core::str::from_utf8(&self.feed_url[..self.feed_url_len]).unwrap_or(DEFAULT_FEED_URL)
        }
    }

    pub fn has_valid_repo(&self) -> bool {
        is_valid_repo(self.repo_str())
    }

    /// Formats a masked display representation of the token (`"••••••••"` when set, empty when unset).
    #[allow(dead_code)]
    pub fn masked_token_utf8(&self) -> &'static str {
        if self.token_len > 0 {
            "••••••••"
        } else {
            ""
        }
    }

    pub fn push_repo_char(&mut self, c: u8) {
        if c.is_ascii_graphic() && self.repo_len < self.repo.len() {
            self.repo[self.repo_len] = c;
            self.repo_len += 1;
        }
    }

    pub fn pop_repo_char(&mut self) {
        self.repo_len = self.repo_len.saturating_sub(1);
    }

    pub fn push_branch_char(&mut self, c: u8) {
        if c.is_ascii_graphic() && self.branch_len < self.branch.len() {
            self.branch[self.branch_len] = c;
            self.branch_len += 1;
        }
    }

    pub fn pop_branch_char(&mut self) {
        self.branch_len = self.branch_len.saturating_sub(1);
    }

    pub fn push_token_char(&mut self, c: u8) {
        if c.is_ascii_graphic() && self.token_len < self.token.len() {
            self.token[self.token_len] = c;
            self.token_len += 1;
        }
    }

    pub fn pop_token_char(&mut self) {
        self.token_len = self.token_len.saturating_sub(1);
    }

    pub fn push_feed_url_char(&mut self, c: u8) {
        if self.feed_url_len == 0 {
            self.set_feed_url(DEFAULT_FEED_URL.as_bytes());
        }
        if c.is_ascii_graphic() && self.feed_url_len < self.feed_url.len() {
            self.feed_url[self.feed_url_len] = c;
            self.feed_url_len += 1;
        }
    }

    pub fn pop_feed_url_char(&mut self) {
        if self.feed_url_len == 0 {
            self.set_feed_url(DEFAULT_FEED_URL.as_bytes());
        }
        self.feed_url_len = self.feed_url_len.saturating_sub(1);
    }
}

fn trim_ascii_bytes(mut s: &[u8]) -> &[u8] {
    while let Some(&b) = s.first() {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            s = &s[1..];
        } else {
            break;
        }
    }
    while let Some(&b) = s.last() {
        if matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            s = &s[..s.len() - 1];
        } else {
            break;
        }
    }
    s
}

fn parse_u64_strict(s: &[u8]) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut v = 0u64;
    for &b in s {
        if !b.is_ascii_digit() {
            return None;
        }
        v = v.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(v)
}

struct Writer<'a> {
    buf: &'a mut [u8],
    at: usize,
}

impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, at: 0 }
    }

    fn put(&mut self, bytes: &[u8]) {
        let take = bytes.len().min(self.buf.len().saturating_sub(self.at));
        self.buf[self.at..self.at + take].copy_from_slice(&bytes[..take]);
        self.at += take;
    }

    fn put_u64(&mut self, mut v: u64) {
        let mut tmp = [0u8; 20];
        let mut k = 0;
        loop {
            tmp[k] = b'0' + (v % 10) as u8;
            k += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while k > 0 {
            k -= 1;
            self.put(&[tmp[k]]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_update_index_reads_version_and_bytes() {
        let raw = b"version=42\nbytes=33554432\nsha256=deadbeef\n";
        let idx = parse_update_index(raw).expect("valid update index");
        assert_eq!(idx.version, 42);
        assert_eq!(idx.bytes, 33_554_432);
        assert_eq!(bytes_to_mib(idx.bytes), 32);

        // Handles CRLF and comments.
        let crlf = b"# SexOS release\r\nbytes=67108864\r\nversion=43\r\n";
        let idx2 = parse_update_index(crlf).expect("valid CRLF index");
        assert_eq!(idx2, UpdateIndex { version: 43, bytes: 67_108_864 });

        // Rejects missing or non-numeric fields.
        assert_eq!(parse_update_index(b"version=42\n"), None);
        assert_eq!(parse_update_index(b"bytes=1024\n"), None);
        assert_eq!(parse_update_index(b"version=abc\nbytes=1024\n"), None);
        assert_eq!(parse_update_index(b"<html>404</html>"), None);
    }

    #[test]
    fn compare_and_format_update_banners() {
        let idx = UpdateIndex {
            version: 15,
            bytes: 48 * 1024 * 1024,
        };
        let avail = compare_update(12, idx);
        assert_eq!(
            avail,
            UpdateCheckResult::Available {
                remote_version: 15,
                bytes: 48 * 1024 * 1024,
                local_version: 12,
            }
        );
        let mut buf = [0u8; 96];
        let n = format_update_banner(avail, &mut buf);
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            "Update Available: SexOS v15 (48 MiB)"
        );

        let current = compare_update(15, idx);
        assert_eq!(
            current,
            UpdateCheckResult::UpToDate {
                local_version: 15,
                remote_version: 15,
            }
        );
        let n2 = format_update_banner(current, &mut buf);
        assert_eq!(core::str::from_utf8(&buf[..n2]).unwrap(), "Up to date (v15)");
    }

    #[test]
    fn parse_local_version_from_facts() {
        assert_eq!(
            parse_local_version(b"slot=a\nversion=12\ncandidate=0\n"),
            Some(12)
        );
        assert_eq!(parse_local_version(b"slot=none\n"), None);
    }

    #[test]
    fn build_update_index_url_handles_default_and_custom_feeds() {
        let mut buf = [0u8; 256];
        let n = build_update_index_url("", &mut buf).unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            "https://github.com/xirtus/sexos/releases/latest/download/sexos-update.txt"
        );

        let n2 = build_update_index_url("https://mirror.example.org/sexos", &mut buf).unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n2]).unwrap(),
            "https://mirror.example.org/sexos/sexos-update.txt"
        );

        let n3 = build_update_index_url(
            "https://mirror.example.org/sexos/sexos-update.txt",
            &mut buf,
        )
        .unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n3]).unwrap(),
            "https://mirror.example.org/sexos/sexos-update.txt"
        );

        assert_eq!(build_update_index_url("ftp://bad.example.org", &mut buf), None);
        assert_eq!(build_update_index_url("https://bad host/a", &mut buf), None);
    }

    #[test]
    fn build_raw_github_url_validates_and_formats() {
        let mut buf = [0u8; 256];
        let n = build_raw_github_url("alice/sexosdotfiles", "main", "brootlite/provider_pack.json", &mut buf)
            .expect("valid raw github url");
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            "https://raw.githubusercontent.com/alice/sexosdotfiles/main/brootlite/provider_pack.json"
        );

        // Empty branch defaults to main.
        let n2 = build_raw_github_url("alice/dots", "", "wifi.conf", &mut buf).unwrap();
        assert_eq!(
            core::str::from_utf8(&buf[..n2]).unwrap(),
            "https://raw.githubusercontent.com/alice/dots/main/wifi.conf"
        );

        // Rejects invalid repo, traversal, or spaces.
        assert_eq!(build_raw_github_url("not-a-repo", "main", "wifi.conf", &mut buf), None);
        assert_eq!(build_raw_github_url("a/b/c", "main", "wifi.conf", &mut buf), None);
        assert_eq!(build_raw_github_url("../evil/repo", "main", "wifi.conf", &mut buf), None);
        assert_eq!(build_raw_github_url("alice/dots", "main", "../etc/passwd", &mut buf), None);
        assert_eq!(build_raw_github_url("alice/dots", "bad branch", "wifi.conf", &mut buf), None);
    }

    #[test]
    fn dotfiles_config_roundtrip_and_masking() {
        let mut cfg = DotfilesConfig::new();
        assert_eq!(cfg.repo_str(), "");
        assert_eq!(cfg.branch_str(), "main");
        assert_eq!(cfg.token_str(), "");
        assert_eq!(cfg.masked_token_utf8(), "");
        assert!(!cfg.auto_sync);
        assert!(!cfg.has_valid_repo());

        cfg.set_repo(b"alice/sexosdotfiles");
        cfg.set_branch(b"workstation");
        cfg.set_token(b"ghp_secretToken123");
        cfg.auto_sync = true;
        cfg.set_feed_url(b"https://releases.example.org/sexos/");

        assert!(cfg.has_valid_repo());
        assert_eq!(cfg.masked_token_utf8(), "••••••••");

        let mut buf = [0u8; 512];
        let n = cfg.serialize(&mut buf);
        let parsed = DotfilesConfig::parse(&buf[..n]);
        assert_eq!(parsed.repo_str(), "alice/sexosdotfiles");
        assert_eq!(parsed.branch_str(), "workstation");
        assert_eq!(parsed.token_str(), "ghp_secretToken123");
        assert!(parsed.auto_sync);
        assert_eq!(parsed.feed_url_str(), "https://releases.example.org/sexos/");
    }

    #[test]
    fn format_synced_status_message() {
        let mut buf = [0u8; 96];
        let n = format_synced_status(2, "alice/sexosdotfiles", &mut buf);
        assert_eq!(
            core::str::from_utf8(&buf[..n]).unwrap(),
            "Synced 2 file(s) from alice/sexosdotfiles"
        );
    }
}
