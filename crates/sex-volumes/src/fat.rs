// SPDX-License-Identifier: GPL-3.0-only
//! FAT12/16/32: being written.
use crate::{Disk, Entry, Error, Format, Result, Stat, Volume};
use alloc::{string::String, vec::Vec};
pub fn looks_like_fat(_boot: &[u8]) -> bool { false }
pub struct Fat;
impl Fat { pub fn mount(_disk: &mut dyn Disk) -> Result<Self> { Err(Error::Unsupported) } }
impl Volume for Fat {
    fn format(&self) -> Format { unreachable!() }
    fn label(&self) -> String { String::new() }
    fn read_only(&self) -> bool { true }
    fn statfs(&mut self, _: &mut dyn Disk) -> Result<(u64, u64)> { Err(Error::Unsupported) }
    fn stat(&mut self, _: &mut dyn Disk, _: &[u8]) -> Result<Stat> { Err(Error::Unsupported) }
    fn list(&mut self, _: &mut dyn Disk, _: &[u8]) -> Result<Vec<Entry>> { Err(Error::Unsupported) }
    fn read(&mut self, _: &mut dyn Disk, _: &[u8], _: u64, _: &mut [u8]) -> Result<usize> { Err(Error::Unsupported) }
}
