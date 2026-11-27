// SPDX-License-Identifier: GPL-3.0-only
//! Capability tiers, typed RPC methods, and zero-dependency JSON helpers for Tether v2.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Live permission tier controlled by the human sitting at the SexOS Tether GUI.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CapabilityMode {
    /// Read-only inspection (logs, sysinfo, fs_read, screenshot, a11y_tree, ACP chat).
    Observe = 0,
    /// Developer mode: adds exec, fs_write, deploy_app, hotswap, snapshot create/restore.
    Dev = 1,
    /// Full machine control: adds synthetic input injection, silk_ctl, and A/B OTA push.
    FullControl = 2,
}

impl CapabilityMode {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Observe),
            1 => Some(Self::Dev),
            2 => Some(Self::FullControl),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Observe => "Observe",
            Self::Dev => "Dev",
            Self::FullControl => "Full Control",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Observe => Self::Dev,
            Self::Dev => Self::FullControl,
            Self::FullControl => Self::Observe,
        }
    }

    pub fn allows(self, method: RpcMethod) -> bool {
        self >= method.required_capability()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcMethod {
    SysInfo,
    Servers,
    Logs,
    Exec,
    FsStat,
    FsList,
    FsRead,
    FsWrite,
    FsDelete,
    Hotswap,
    Screenshot,
    A11yTree,
    SilkCtl,
    InputInject,
    SnapshotCreate,
    SnapshotList,
    SnapshotRestore,
    OtaPush,
    WaveReview,
}

impl RpcMethod {
    pub fn as_name(self) -> &'static str {
        match self {
            Self::SysInfo => "sysinfo",
            Self::Servers => "servers",
            Self::Logs => "logs",
            Self::Exec => "exec",
            Self::FsStat => "fs_stat",
            Self::FsList => "fs_list",
            Self::FsRead => "fs_read",
            Self::FsWrite => "fs_write",
            Self::FsDelete => "fs_delete",
            Self::Hotswap => "hotswap",
            Self::Screenshot => "screenshot",
            Self::A11yTree => "a11y_tree",
            Self::SilkCtl => "silk_ctl",
            Self::InputInject => "input_inject",
            Self::SnapshotCreate => "snapshot_create",
            Self::SnapshotList => "snapshot_list",
            Self::SnapshotRestore => "snapshot_restore",
            Self::OtaPush => "ota_push",
            Self::WaveReview => "wave_review",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "sysinfo" => Some(Self::SysInfo),
            "servers" => Some(Self::Servers),
            "logs" => Some(Self::Logs),
            "exec" => Some(Self::Exec),
            "fs_stat" => Some(Self::FsStat),
            "fs_list" => Some(Self::FsList),
            "fs_read" => Some(Self::FsRead),
            "fs_write" => Some(Self::FsWrite),
            "fs_delete" => Some(Self::FsDelete),
            "hotswap" => Some(Self::Hotswap),
            "screenshot" => Some(Self::Screenshot),
            "a11y_tree" => Some(Self::A11yTree),
            "silk_ctl" => Some(Self::SilkCtl),
            "input_inject" => Some(Self::InputInject),
            "snapshot_create" => Some(Self::SnapshotCreate),
            "snapshot_list" => Some(Self::SnapshotList),
            "snapshot_restore" => Some(Self::SnapshotRestore),
            "ota_push" => Some(Self::OtaPush),
            "wave_review" => Some(Self::WaveReview),
            _ => None,
        }
    }

    pub fn required_capability(self) -> CapabilityMode {
        match self {
            Self::SysInfo
            | Self::Servers
            | Self::Logs
            | Self::FsStat
            | Self::FsList
            | Self::FsRead
            | Self::Screenshot
            | Self::A11yTree
            | Self::SnapshotList
            | Self::WaveReview => CapabilityMode::Observe,
            Self::Exec
            | Self::FsWrite
            | Self::FsDelete
            | Self::Hotswap
            | Self::SnapshotCreate
            | Self::SnapshotRestore => CapabilityMode::Dev,
            Self::SilkCtl | Self::InputInject | Self::OtaPush => CapabilityMode::FullControl,
        }
    }

    /// True if this RPC mutates live SexOS state or interacts with the desktop and
    /// therefore requires an exclusive hardware lease when multiple subagents are active.
    pub fn is_mutating(self) -> bool {
        !matches!(
            self,
            Self::SysInfo
                | Self::Servers
                | Self::Logs
                | Self::FsStat
                | Self::FsList
                | Self::FsRead
                | Self::Screenshot
                | Self::A11yTree
                | Self::SnapshotList
        )
    }
}

/// Escapes a string for safe embedding inside a JSON string literal (without outer quotes).
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Finds the byte index right after `"key":` (skipping whitespace) in a JSON object slice.
fn find_json_value_start(json: &str, key: &str) -> Option<usize> {
    let bytes = json.as_bytes();
    let needle = format!("\"{}\"", key);
    let nb = needle.as_bytes();
    let mut i = 0;
    let mut in_str = false;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            if bytes[i..].starts_with(nb) {
                let mut j = i + nb.len();
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b':' {
                    j += 1;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    return Some(j);
                }
            }
            in_str = true;
            i += 1;
            continue;
        }
        i += 1;
    }
    None
}

/// Extracts and unescapes a string value for `"key"` from a JSON object.
pub fn json_get_str(json: &str, key: &str) -> Option<String> {
    let start = find_json_value_start(json, key)?;
    let bytes = json.as_bytes();
    if bytes.get(start) != Some(&b'"') {
        return None;
    }
    let mut out = Vec::new();
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            return String::from_utf8(out).ok();
        }
        if b == b'\\' && i + 1 < bytes.len() {
            i += 1;
            match bytes[i] {
                b'"' => out.push(b'"'),
                b'\\' => out.push(b'\\'),
                b'/' => out.push(b'/'),
                b'n' => out.push(b'\n'),
                b'r' => out.push(b'\r'),
                b't' => out.push(b'\t'),
                b'b' => out.push(0x08),
                b'f' => out.push(0x0c),
                b'u' if i + 4 < bytes.len() => {
                    if let Ok(hex) = core::str::from_utf8(&bytes[i + 1..i + 5]) {
                        if let Ok(code) = u32::from_str_radix(hex, 16) {
                            if let Some(ch) = char::from_u32(code) {
                                let mut tmp = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                            }
                        }
                    }
                    i += 4;
                }
                other => out.push(other),
            }
        } else {
            out.push(b);
        }
        i += 1;
    }
    None
}

/// Extracts an unsigned integer value for `"key"` from a JSON object.
pub fn json_get_u64(json: &str, key: &str) -> Option<u64> {
    let start = find_json_value_start(json, key)?;
    let rest = &json[start..];
    let end = rest
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    rest[..end].parse::<u64>().ok()
}

/// Extracts a boolean value for `"key"` from a JSON object.
pub fn json_get_bool(json: &str, key: &str) -> Option<bool> {
    let start = find_json_value_start(json, key)?;
    let rest = &json[start..];
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

/// Builds a compact JSON RPC response payload.
pub fn rpc_ok_json(id: u64, result_json: &str) -> String {
    format!("{{\"id\":{},\"ok\":true,\"result\":{}}}", id, result_json)
}

pub fn rpc_err_json(id: u64, code: &str, message: &str) -> String {
    format!(
        "{{\"id\":{},\"ok\":false,\"error\":{{\"code\":\"{}\",\"message\":\"{}\"}}}}",
        id,
        json_escape(code),
        json_escape(message)
    )
}
