// SPDX-License-Identifier: GPL-3.0-only
//! Ntfs: not written yet.
use crate::{Disk, Entry, Error, Format, Result, Stat, Volume};
use alloc::{string::String, vec::Vec};

pub struct Ntfs;
impl Ntfs {
    pub fn mount(_disk: &mut dyn Disk) -> Result<Self> { Err(Error::Unsupported) }
}
impl Volume for Ntfs {
    fn format(&self) -> Format { unreachable!() }
    fn label(&self) -> String { String::new() }
    fn read_only(&self) -> bool { true }
    fn statfs(&mut self, _: &mut dyn Disk) -> Result<(u64, u64)> { Err(Error::Unsupported) }
    fn stat(&mut self, _: &mut dyn Disk, _: &[u8]) -> Result<Stat> { Err(Error::Unsupported) }
    fn list(&mut self, _: &mut dyn Disk, _: &[u8]) -> Result<Vec<Entry>> { Err(Error::Unsupported) }
    fn read(&mut self, _: &mut dyn Disk, _: &[u8], _: u64, _: &mut [u8]) -> Result<usize> { Err(Error::Unsupported) }
}
