// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The radix-3 page table Booter walks to copy GSP-RM out of system memory.
//! Level 0 is one entry pointing at level 1 (one page); level 1 lists the
//! pages of level 2; level 2 lists every 4 KiB page of the image.

use crate::{wr64, Dma, Error};

pub const PAGE: usize = 4096;

/// Bytes of level 2 needed for an image of `image_len` bytes.
pub fn lvl2_len(image_len: usize) -> usize { crate::align_up((image_len.div_ceil(PAGE) * 8) as u64, PAGE as u64) as usize }

/// Fill the three levels. `image_page(i)` is the bus address of image page `i`.
pub fn build(lvl0: &mut Dma, lvl1: &mut Dma, lvl2: &mut Dma, image_len: usize, image_page: impl Fn(usize) -> u64) -> Result<(), Error> {
    let pages = image_len.div_ceil(PAGE);
    let need2 = lvl2_len(image_len);
    if lvl0.len() < 8 { return Err(Error::BufferTooSmall { need: 8, have: lvl0.len() }); }
    if lvl2.len() < need2 { return Err(Error::BufferTooSmall { need: need2, have: lvl2.len() }); }
    let l2_pages = need2 / PAGE;
    if l2_pages > PAGE / 8 { return Err(Error::Gsp("radix3: image larger than 1 GiB")); }
    if lvl1.len() < l2_pages * 8 { return Err(Error::BufferTooSmall { need: l2_pages * 8, have: lvl1.len() }); }
    lvl0.cpu.fill(0);
    lvl1.cpu.fill(0);
    lvl2.cpu[..need2].fill(0);
    wr64(lvl0.cpu, 0, lvl1.bus);
    for i in 0..l2_pages { wr64(lvl1.cpu, i * 8, lvl2.bus + (i * PAGE) as u64); }
    for i in 0..pages { wr64(lvl2.cpu, i * 8, image_page(i)); }
    Ok(())
}
