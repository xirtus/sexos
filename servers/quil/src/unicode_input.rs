//! Finite code-point entry over the existing authenticated Shell key route.
//! Pending digits are private UI intent, not document bytes or authority.
use sex_pdx::text_encoding;

pub struct Input { active: bool, value: u32, digits: u8, invalid: bool }
#[derive(Debug, PartialEq, Eq)]
pub enum Action { Pending, Cancelled, Commit { bytes: [u8; 4], len: usize }, Refused }
impl Input {
    pub const fn new() -> Self { Self { active: false, value: 0, digits: 0, invalid: false } }
    pub const fn active(&self) -> bool { self.active }
    pub fn clear(&mut self) { *self = Self::new(); }
    pub fn begin(&mut self) { self.clear(); self.active = true; }
    pub fn key(&mut self, code: u8) -> Action {
        if !self.active { return Action::Cancelled; }
        if code == 0x01 { self.clear(); return Action::Cancelled; }
        if code == 0x0e {
            if self.digits != 0 { self.value >>= 4; self.digits -= 1; }
            self.invalid = false; return Action::Pending;
        }
        if code == 0x1c {
            if self.invalid || self.digits == 0 || !text_encoding::is_text_scalar(self.value) {
                self.invalid = true; return Action::Refused;
            }
            let mut bytes = [0; 4];
            let Some(len) = text_encoding::encode(self.value, &mut bytes) else {
                self.invalid = true; return Action::Refused;
            };
            self.clear(); return Action::Commit { bytes, len };
        }
        let digit = match code { 0x02..=0x0a => Some(code - 1), 0x0b => Some(0),
            0x1e => Some(10), 0x30 => Some(11), 0x2e => Some(12),
            0x20 => Some(13), 0x12 => Some(14), 0x21 => Some(15), _ => None };
        if let Some(digit) = digit {
            if self.digits == 6 { self.invalid = true; return Action::Refused; }
            self.value = (self.value << 4) | digit as u32; self.digits += 1; self.invalid = false;
        }
        Action::Pending
    }
    pub fn prompt(&self) -> ([u8; 48], usize) {
        let mut out = [0; 48]; let mut n = 0;
        for &b in b"Unicode U+" { out[n] = b; n += 1; }
        for digit in (0..self.digits).rev() {
            out[n] = b"0123456789ABCDEF"[((self.value >> (digit as u32 * 4)) & 15) as usize]; n += 1;
        }
        for &b in if self.invalid { b" invalid | Backspace / Esc".as_slice() }
            else { b"_ | Enter / Esc".as_slice() } { out[n] = b; n += 1; }
        (out, n)
    }
}
