// SPDX-License-Identifier: GPL-3.0-only
//! A line of text built without a heap: numbers right-aligned, shares in tenths of a
//! percent, sizes from pages, padded columns.

pub struct Line<const N: usize> { buf: [u8; N], len: usize }

impl<const N: usize> Default for Line<N> {
    fn default() -> Self { Self::new() }
}

impl<const N: usize> Line<N> {
    pub const fn new() -> Self { Self { buf: [0; N], len: 0 } }
    pub fn as_bytes(&self) -> &[u8] { &self.buf[..self.len] }
    pub fn len(&self) -> usize { self.len }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    pub fn clear(&mut self) { self.len = 0; }
    /// Appends what fits.
    pub fn put(&mut self, b: &[u8]) -> &mut Self {
        let n = b.len().min(N - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&b[..n]);
        self.len += n;
        self
    }
    /// Spaces up to column `col`.
    pub fn pad(&mut self, col: usize) -> &mut Self {
        while self.len < col.min(N) { self.put(b" "); }
        self
    }
    /// `n` right-aligned in `width` columns.
    pub fn num(&mut self, mut n: u64, width: usize) -> &mut Self {
        let mut d = [0u8; 20];
        let mut i = d.len();
        loop { i -= 1; d[i] = b'0' + (n % 10) as u8; n /= 10; if n == 0 { break; } }
        for _ in (d.len() - i)..width { self.put(b" "); }
        self.put(&d[i..])
    }
    /// Tenths of a percent (permille) as `12.3%`, five columns wide plus the sign.
    pub fn tenths(&mut self, t: u64) -> &mut Self { self.num(t / 10, 3).put(b".").num(t % 10, 1).put(b"%") }
    /// A size in bytes as B, KiB, MiB or GiB, nine columns wide.
    pub fn size(&mut self, bytes: u64) -> &mut Self {
        const K: u64 = 1024;
        let (v, unit): (u64, &[u8]) = if bytes >= 10 * K * K * K { (bytes / (K * K * K), b" GiB") }
            else if bytes >= 10 * K * K { (bytes / (K * K), b" MiB") }
            else if bytes >= 10 * K { (bytes / K, b" KiB") } else { (bytes, b" B  ") };
        self.num(v, 5).put(unit)
    }
    /// A run's memory in 4 KiB pages; `u64::MAX` means not known.
    pub fn pages(&mut self, pages: u64) -> &mut Self {
        if pages == u64::MAX { return self.put(b"        -"); }
        self.size(pages.saturating_mul(4096))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_and_units() {
        let mut l: Line<64> = Line::new();
        l.put(b"ab").pad(4).num(7, 3).put(b"|").tenths(1000).put(b"|").tenths(5);
        assert_eq!(l.as_bytes(), b"ab    7|100.0%|  0.5%");
        l.clear();
        l.size(512).put(b"|").size(20 * 1024).put(b"|").size(300 * 1024 * 1024).put(b"|").pages(u64::MAX);
        assert_eq!(l.as_bytes(), b"  512 B  |   20 KiB|  300 MiB|        -");
        l.clear();
        l.pages(3);
        assert_eq!(l.as_bytes(), b"   12 KiB");
    }

    #[test]
    fn a_full_line_drops_the_rest() {
        let mut l: Line<4> = Line::new();
        l.put(b"abc").num(12345, 0).pad(9);
        assert_eq!(l.as_bytes(), b"abc1");
        assert_eq!(l.len(), 4);
    }
}
