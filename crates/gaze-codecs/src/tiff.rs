//! A bounds-checked reader for TIFF structures: the byte order header, image
//! file directories (IFDs) and their entries. EXIF blocks and most camera RAW
//! files are TIFF inside, so this is the base of both.

/// Most IFDs one walk will visit; a file looping its IFD chain stops here.
pub(crate) const MAX_IFDS: usize = 32;
/// Most entries one IFD may announce before it is refused.
const MAX_ENTRIES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Order {
    Little,
    Big,
}

/// A TIFF block: `data` starts at the byte order mark, all offsets are relative to it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tiff<'a> {
    pub data: &'a [u8],
    pub order: Order,
    /// Offset of the first IFD.
    pub first_ifd: usize,
}

/// One directory entry: tag, field type, count and the raw value bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry<'a> {
    pub tag: u16,
    pub kind: u16,
    pub count: usize,
    pub bytes: &'a [u8],
}

// Field types.
pub(crate) const BYTE: u16 = 1;
pub(crate) const ASCII: u16 = 2;
pub(crate) const SHORT: u16 = 3;
pub(crate) const LONG: u16 = 4;
pub(crate) const RATIONAL: u16 = 5;
pub(crate) const UNDEFINED: u16 = 7;
pub(crate) const SLONG: u16 = 9;
pub(crate) const SRATIONAL: u16 = 10;
pub(crate) const IFD: u16 = 13;

fn type_size(kind: u16) -> Option<usize> {
    Some(match kind {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => return None,
    })
}

impl<'a> Tiff<'a> {
    /// Reads the 8-byte header. Accepts the classic `*` magic and the
    /// variants some RAW formats use in its place (Olympus ORF `RO`/`RS`,
    /// Panasonic RW2 `U`).
    pub fn parse(data: &'a [u8]) -> Option<Tiff<'a>> {
        if data.len() < 8 {
            return None;
        }
        let order = match &data[..2] {
            b"II" => Order::Little,
            b"MM" => Order::Big,
            _ => return None,
        };
        let t = Tiff { data, order, first_ifd: 0 };
        let magic = t.u16_at(2)?;
        if !matches!(magic, 42 | 0x4f52 | 0x5352 | 0x55) {
            return None;
        }
        let first_ifd = t.u32_at(4)? as usize;
        Some(Tiff { first_ifd, ..t })
    }

    pub fn u16_at(&self, off: usize) -> Option<u16> {
        let b = self.data.get(off..off.checked_add(2)?)?;
        Some(self.u16_of(b))
    }

    pub fn u32_at(&self, off: usize) -> Option<u32> {
        let b = self.data.get(off..off.checked_add(4)?)?;
        Some(self.u32_of(b))
    }

    fn u16_of(&self, b: &[u8]) -> u16 {
        let a = [b[0], b[1]];
        match self.order {
            Order::Little => u16::from_le_bytes(a),
            Order::Big => u16::from_be_bytes(a),
        }
    }

    fn u32_of(&self, b: &[u8]) -> u32 {
        let a = [b[0], b[1], b[2], b[3]];
        match self.order {
            Order::Little => u32::from_le_bytes(a),
            Order::Big => u32::from_be_bytes(a),
        }
    }

    /// The entries of the IFD at `off` and the offset of the next IFD (0 at the end).
    pub fn ifd(&self, off: usize) -> Option<(Ifd<'a>, usize)> {
        if off == 0 {
            return None;
        }
        let n = self.u16_at(off)? as usize;
        if n == 0 || n > MAX_ENTRIES {
            return None;
        }
        let start = off.checked_add(2)?;
        let end = start.checked_add(n * 12)?;
        let entries = self.data.get(start..end)?;
        let next = self.u32_at(end).unwrap_or(0) as usize;
        Some((Ifd { tiff: *self, entries }, next))
    }

    /// Offsets of the IFD chain starting at the first IFD, loop safe.
    pub fn chain(&self) -> impl Iterator<Item = Ifd<'a>> + '_ {
        let mut seen: [usize; MAX_IFDS] = [0; MAX_IFDS];
        let mut n = 0;
        let mut next = self.first_ifd;
        core::iter::from_fn(move || {
            if next == 0 || n == MAX_IFDS || seen[..n].contains(&next) {
                return None;
            }
            seen[n] = next;
            n += 1;
            let (ifd, after) = self.ifd(next)?;
            next = after;
            Some(ifd)
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ifd<'a> {
    pub tiff: Tiff<'a>,
    entries: &'a [u8],
}

impl<'a> Ifd<'a> {
    pub fn entries(&self) -> impl Iterator<Item = Entry<'a>> + '_ {
        self.entries.chunks_exact(12).filter_map(move |e| {
            let t = &self.tiff;
            let tag = t.u16_of(&e[0..2]);
            let kind = t.u16_of(&e[2..4]);
            let count = t.u32_of(&e[4..8]) as usize;
            let size = type_size(kind)?.checked_mul(count)?;
            let bytes = if size <= 4 {
                &e[8..8 + size]
            } else {
                let off = t.u32_of(&e[8..12]) as usize;
                t.data.get(off..off.checked_add(size)?)?
            };
            Some(Entry { tag, kind, count, bytes })
        })
    }

    pub fn get(&self, tag: u16) -> Option<Entry<'a>> {
        self.entries().find(|e| e.tag == tag)
    }

    /// The first value of a SHORT, LONG or BYTE entry.
    pub fn uint(&self, tag: u16) -> Option<u32> {
        self.get(tag).and_then(|e| self.uint_n(&e, 0))
    }

    /// Value `i` of a SHORT, LONG, BYTE or IFD entry.
    pub fn uint_n(&self, e: &Entry<'a>, i: usize) -> Option<u32> {
        if i >= e.count {
            return None;
        }
        let t = &self.tiff;
        match e.kind {
            BYTE | UNDEFINED => e.bytes.get(i).map(|&b| b as u32),
            SHORT => Some(t.u16_of(e.bytes.get(i * 2..i * 2 + 2)?) as u32),
            LONG | IFD | SLONG => Some(t.u32_of(e.bytes.get(i * 4..i * 4 + 4)?)),
            _ => None,
        }
    }

    /// Value `i` of a RATIONAL entry as (numerator, denominator).
    pub fn rational_n(&self, e: &Entry<'a>, i: usize) -> Option<(u32, u32)> {
        if !matches!(e.kind, RATIONAL | SRATIONAL) || i >= e.count {
            return None;
        }
        let b = e.bytes.get(i * 8..i * 8 + 8)?;
        Some((self.tiff.u32_of(&b[0..4]), self.tiff.u32_of(&b[4..8])))
    }

    pub fn rational(&self, tag: u16) -> Option<(u32, u32)> {
        self.get(tag).and_then(|e| self.rational_n(&e, 0))
    }

    /// An ASCII (or UNDEFINED, as some writers use) entry up to its first NUL,
    /// with trailing spaces removed. Empty strings are `None`.
    pub fn ascii(&self, tag: u16) -> Option<&'a str> {
        let e = self.get(tag)?;
        if !matches!(e.kind, ASCII | UNDEFINED | BYTE) {
            return None;
        }
        let end = e.bytes.iter().position(|&b| b == 0).unwrap_or(e.bytes.len());
        let s = core::str::from_utf8(&e.bytes[..end]).ok()?.trim_matches(|c: char| c == ' ' || c.is_control());
        (!s.is_empty()).then_some(s)
    }

    /// The IFD a pointer tag (Exif, GPS, SubIFD entry `i`) leads to.
    pub fn sub(&self, tag: u16, i: usize) -> Option<Ifd<'a>> {
        let e = self.get(tag)?;
        let off = self.uint_n(&e, i)? as usize;
        self.tiff.ifd(off).map(|(ifd, _)| ifd)
    }
}
