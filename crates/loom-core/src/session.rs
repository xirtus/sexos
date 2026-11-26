// SPDX-License-Identifier: GPL-3.0-only
//! Bounded, versioned folder-tab state. Paths are SexOS Files paths, not host paths.
use alloc::vec::Vec;

const MAGIC: &[u8; 4] = b"LTS1";
pub const MAX_TABS: usize = 12;
const MAX_PATH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab { pub path: Vec<u8>, pub cursor: u32, pub top: u32 }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session { pub active: usize, pub tabs: Vec<Tab> }

fn valid_path(path: &[u8]) -> bool {
    path.first() == Some(&b'/') && path.len() <= MAX_PATH && !path.contains(&0)
        && !path.windows(2).any(|w| w == b"//")
        && !path.split(|b| *b == b'/').any(|part| part == b"." || part == b"..")
}

impl Session {
    pub fn encode(&self) -> Option<Vec<u8>> {
        if self.tabs.is_empty() || self.tabs.len() > MAX_TABS || self.active >= self.tabs.len()
            || self.tabs.iter().any(|t| !valid_path(&t.path)) { return None; }
        let mut out = Vec::with_capacity(6 + self.tabs.iter().map(|t| 10 + t.path.len()).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.push(self.tabs.len() as u8);
        out.push(self.active as u8);
        for tab in &self.tabs {
            out.extend_from_slice(&(tab.path.len() as u16).to_le_bytes());
            out.extend_from_slice(&tab.cursor.to_le_bytes());
            out.extend_from_slice(&tab.top.to_le_bytes());
            out.extend_from_slice(&tab.path);
        }
        Some(out)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.get(..4)? != MAGIC.as_slice() { return None; }
        let count = *bytes.get(4)? as usize;
        let active = *bytes.get(5)? as usize;
        if count == 0 || count > MAX_TABS || active >= count { return None; }
        let mut at = 6;
        let mut tabs = Vec::with_capacity(count);
        for _ in 0..count {
            let len = u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize;
            let cursor = u32::from_le_bytes(bytes.get(at + 2..at + 6)?.try_into().ok()?);
            let top = u32::from_le_bytes(bytes.get(at + 6..at + 10)?.try_into().ok()?);
            at += 10;
            if len == 0 || len > MAX_PATH { return None; }
            let path = bytes.get(at..at + len)?;
            if !valid_path(path) { return None; }
            tabs.push(Tab { path: path.to_vec(), cursor, top });
            at += len;
        }
        (at == bytes.len()).then_some(Self { active, tabs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_reject_partial_or_hostile_state() {
        let session = Session { active: 1, tabs: alloc::vec![
            Tab { path: b"/media/LOOM".to_vec(), cursor: 2, top: 1 },
            Tab { path: b"/media/LOOM/docs".to_vec(), cursor: 4, top: 3 },
        ] };
        let bytes = session.encode().unwrap();
        assert_eq!(Session::decode(&bytes), Some(session));
        for end in 0..bytes.len() { assert!(Session::decode(&bytes[..end]).is_none()); }
        let mut bad = bytes.clone(); bad[5] = 2; assert!(Session::decode(&bad).is_none());
        let mut bad = bytes.clone(); bad.extend_from_slice(b"junk"); assert!(Session::decode(&bad).is_none());
        assert!(Session { active: 0, tabs: alloc::vec![Tab { path: b"/media/../secrets".to_vec(), cursor: 0, top: 0 }] }.encode().is_none());
    }
}
