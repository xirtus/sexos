//! Bounded, allocation-free RGB PNG writer over an immutable XRGB frame.
//! Uses zlib stored blocks; output can be streamed through a small file buffer.
const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const fn table() -> [u32; 256] {
    let mut out = [0; 256];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xedb88320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        out[n] = c;
        n += 1;
    }
    out
}
const CRC: [u32; 256] = table();
fn crc(c: u32, b: u8) -> u32 {
    CRC[((c ^ b as u32) & 255) as usize] ^ (c >> 8)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Geometry,
    Pixels,
    Overflow,
}
/// Contains counters only; no source pointer or borrowed slice crosses a turn.
pub struct Encoder {
    width: usize,
    pixels: usize,
    cached: u32,
    raw: usize,
    raw_at: usize,
    block_left: usize,
    phase: u8,
    head: [u8; 40],
    at: usize,
    end: usize,
    crc: u32,
    a: u32,
    b: u32,
    total: usize,
    written: usize,
}
impl Encoder {
    pub fn new(width: usize, height: usize) -> Result<Self, Error> {
        if width == 0 || height == 0 || width > u32::MAX as usize || height > u32::MAX as usize {
            return Err(Error::Geometry);
        }
        let pixels = width.checked_mul(height).ok_or(Error::Overflow)?;
        let raw = width
            .checked_mul(3)
            .and_then(|n| n.checked_add(1))
            .and_then(|n| n.checked_mul(height))
            .ok_or(Error::Overflow)?;
        let blocks = raw.checked_add(65534).ok_or(Error::Overflow)? / 65535;
        let idat = blocks
            .checked_mul(5)
            .and_then(|n| n.checked_add(raw))
            .and_then(|n| n.checked_add(6))
            .ok_or(Error::Overflow)?;
        if idat > u32::MAX as usize {
            return Err(Error::Overflow);
        }
        let mut head = [0; 40];
        head[..8].copy_from_slice(&SIGNATURE);
        head[8..12].copy_from_slice(&13u32.to_be_bytes());
        head[12..16].copy_from_slice(b"IHDR");
        head[16..20].copy_from_slice(&(width as u32).to_be_bytes());
        head[20..24].copy_from_slice(&(height as u32).to_be_bytes());
        head[24] = 8;
        head[25] = 2;
        let checksum = !head[12..29].iter().fold(!0, |c, &v| crc(c, v));
        head[29..33].copy_from_slice(&checksum.to_be_bytes());
        head[33..37].copy_from_slice(&(idat as u32).to_be_bytes());
        // The final 'T' is emitted by the next phase to keep a tiny fixed prefix.
        head[37..40].copy_from_slice(b"IDA");
        Ok(Self {
            width,
            pixels,
            cached: 0,
            raw,
            raw_at: 0,
            block_left: 0,
            phase: 0,
            head,
            at: 0,
            end: 40,
            crc: !0,
            a: 1,
            b: 0,
            total: idat.checked_add(57).ok_or(Error::Overflow)?,
            written: 0,
        })
    }
    pub fn total_bytes(&self) -> usize {
        self.total
    }
    pub fn done(&self) -> bool {
        self.phase == 8
    }
    fn prefix(&mut self, bytes: &[u8], phase: u8) {
        self.head[..bytes.len()].copy_from_slice(bytes);
        self.at = 0;
        self.end = bytes.len();
        self.phase = phase;
    }
    fn byte(&mut self, pixel: &mut impl FnMut(usize) -> u32) -> Option<u8> {
        loop {
            match self.phase {
                0 => {
                    if self.at < self.end {
                        let b = self.head[self.at];
                        self.at += 1;
                        return Some(b);
                    }
                    self.crc = b"IDAT".iter().fold(!0, |c, &b| crc(c, b));
                    self.prefix(&[b'T', 0x78, 0x01], 1);
                }
                1 => {
                    if self.at < self.end {
                        let b = self.head[self.at];
                        self.at += 1;
                        if self.at > 1 {
                            self.crc = crc(self.crc, b)
                        }
                        return Some(b);
                    }
                    self.phase = 2;
                }
                2 => {
                    if self.raw_at == self.raw {
                        let adler = (self.b << 16) | self.a;
                        self.prefix(&adler.to_be_bytes(), 5);
                        continue;
                    }
                    let n = (self.raw - self.raw_at).min(65535) as u16;
                    self.block_left = n as usize;
                    let l = n.to_le_bytes();
                    let neg = (!n).to_le_bytes();
                    self.prefix(
                        &[
                            (self.raw - self.raw_at <= 65535) as u8,
                            l[0],
                            l[1],
                            neg[0],
                            neg[1],
                        ],
                        3,
                    );
                }
                3 => {
                    if self.at < self.end {
                        let b = self.head[self.at];
                        self.at += 1;
                        self.crc = crc(self.crc, b);
                        return Some(b);
                    }
                    self.phase = 4;
                }
                4 => {
                    if self.block_left == 0 {
                        self.phase = 2;
                        continue;
                    }
                    let row = self.width * 3 + 1;
                    let x = self.raw_at % row;
                    let v = if x == 0 {
                        0
                    } else {
                        let channel = (x - 1) % 3;
                        if channel == 0 {
                            self.cached = pixel((self.raw_at / row) * self.width + (x - 1) / 3)
                        }
                        (self.cached >> (16 - 8 * channel)) as u8
                    };
                    self.raw_at += 1;
                    self.block_left -= 1;
                    self.a = (self.a + v as u32) % 65521;
                    self.b = (self.b + self.a) % 65521;
                    self.crc = crc(self.crc, v);
                    return Some(v);
                }
                5 => {
                    if self.at < self.end {
                        let b = self.head[self.at];
                        self.at += 1;
                        self.crc = crc(self.crc, b);
                        return Some(b);
                    }
                    self.prefix(&(!self.crc).to_be_bytes(), 6);
                }
                6 => {
                    if self.at < self.end {
                        let b = self.head[self.at];
                        self.at += 1;
                        return Some(b);
                    }
                    let mut end = [0; 12];
                    end[4..8].copy_from_slice(b"IEND");
                    let c = !b"IEND".iter().fold(!0, |c, &b| crc(c, b));
                    end[8..].copy_from_slice(&c.to_be_bytes());
                    self.prefix(&end, 7);
                }
                7 => {
                    if self.at < self.end {
                        let b = self.head[self.at];
                        self.at += 1;
                        return Some(b);
                    }
                    self.phase = 8;
                }
                _ => return None,
            }
        }
    }
    /// Caller guarantees the same immutable frame for all calls. Short output
    /// buffers are valid, including boundaries inside a chunk CRC or block header.
    pub fn fill(&mut self, pixels: &[u32], out: &mut [u8]) -> Result<usize, Error> {
        self.fill_with(pixels.len(), |i| pixels[i], out)
    }
    /// Stream from a bounded reader, without borrowing shared backing as a slice.
    pub fn fill_with(
        &mut self,
        length: usize,
        mut pixel: impl FnMut(usize) -> u32,
        out: &mut [u8],
    ) -> Result<usize, Error> {
        if length != self.pixels {
            return Err(Error::Pixels);
        }
        let mut n = 0;
        while n < out.len() {
            let Some(b) = self.byte(&mut pixel) else {
                break;
            };
            out[n] = b;
            n += 1;
        }
        self.written = self.written.checked_add(n).ok_or(Error::Overflow)?;
        if self.phase == 8 && self.written != self.total {
            return Err(Error::Overflow);
        }
        Ok(n)
    }
}
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    fn encoded(w: usize, h: usize, chunk: usize) -> std::vec::Vec<u8> {
        let p = std::vec![0x123456;w*h];
        let mut e = Encoder::new(w, h).unwrap();
        let mut out = std::vec::Vec::new();
        let mut b = std::vec![0;chunk];
        loop {
            let n = e.fill(&p, &mut b).unwrap();
            out.extend_from_slice(&b[..n]);
            if n == 0 {
                break;
            }
        }
        assert_eq!(out.len(), e.total_bytes());
        out
    }
    #[test]
    fn chunk_boundaries() {
        for chunk in [1, 2, 3, 4, 7, 31, 64, 65535] {
            assert_eq!(encoded(129, 173, chunk), encoded(129, 173, 100003));
        }
    }
    #[test]
    fn geometry_and_pixel_lengths() {
        assert!(Encoder::new(0, 1).is_err());
        assert!(Encoder::new(usize::MAX, 2).is_err());
        assert_eq!(
            Encoder::new(2, 1).unwrap().fill(&[0], &mut [0; 20]),
            Err(Error::Pixels)
        );
    }
    #[test]
    fn one_pixel_golden() {
        let b = encoded(1, 1, 1);
        assert_eq!(&b[..8], &SIGNATURE);
        assert_eq!(&b[16..24], &[0, 0, 0, 1, 0, 0, 0, 1]);
        assert_eq!(
            b.as_slice(),
            &[
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 2, 0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 15, 73, 68, 65, 84, 120, 1, 1, 4, 0,
                251, 255, 0, 18, 52, 86, 0, 248, 0, 157, 248, 215, 100, 140, 0, 0, 0, 0, 73, 69,
                78, 68, 174, 66, 96, 130
            ]
        );
    }
}
