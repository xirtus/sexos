// SPDX-License-Identifier: GPL-3.0-only
//! Loom's folder tabs: a small set of independent folder positions.
use alloc::vec;
use alloc::vec::Vec;

pub const MAX_TABS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab { pub folder: Vec<u8>, pub cursor: usize, pub top: usize }

pub struct Tabs { items: Vec<Tab>, active: usize }
impl Tabs {
    pub fn new(folder: Vec<u8>) -> Self { Self { items: vec![Tab { folder, cursor: 0, top: 0 }], active: 0 } }
    pub fn all(&self) -> &[Tab] { &self.items }
    pub fn active(&self) -> usize { self.active }
    pub fn len(&self) -> usize { self.items.len() }

    pub fn open(&mut self, current: Tab) -> bool {
        if self.items.len() == MAX_TABS { return false; }
        self.items[self.active] = current.clone();
        self.items.push(current);
        self.active = self.items.len() - 1;
        true
    }

    pub fn next(&mut self, current: Tab) -> Option<Tab> {
        if self.items.len() < 2 { return None; }
        self.items[self.active] = current;
        self.active = (self.active + 1) % self.items.len();
        Some(self.items[self.active].clone())
    }

    pub fn close(&mut self) -> Option<Tab> {
        if self.items.len() < 2 { return None; }
        self.items.remove(self.active);
        self.active = self.active.min(self.items.len() - 1);
        Some(self.items[self.active].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabs_keep_independent_folder_and_selection_and_close() {
        let mut tabs = Tabs::new(b"/media/LOOM".to_vec());
        assert!(tabs.open(Tab { folder: b"/media/LOOM".to_vec(), cursor: 2, top: 1 }));
        assert_eq!(tabs.len(), 2);
        let first = tabs.next(Tab { folder: b"/media/LOOM/docs".to_vec(), cursor: 4, top: 3 }).unwrap();
        assert_eq!(first, Tab { folder: b"/media/LOOM".to_vec(), cursor: 2, top: 1 });
        let second = tabs.next(Tab { folder: b"/media/LOOM".to_vec(), cursor: 1, top: 0 }).unwrap();
        assert_eq!(second.folder, b"/media/LOOM/docs");
        assert_eq!((second.cursor, second.top), (4, 3));
        assert_eq!(tabs.close().unwrap().folder, b"/media/LOOM");
        assert!(tabs.close().is_none());
        for _ in 1..MAX_TABS { assert!(tabs.open(Tab { folder: b"/".to_vec(), cursor: 0, top: 0 })); }
        assert!(!tabs.open(Tab { folder: b"/".to_vec(), cursor: 0, top: 0 }));
    }
}
