//! One immutable active batch plus latest desired text, retained under Full.
use super::view::Text;
#[derive(Clone, Copy)]
struct Batch {
    text: Text,
    paint: u64,
    clear: bool,
    chunks: usize,
}
pub struct Outbox {
    active: Option<Batch>,
    latest: Option<Batch>,
    dead: bool,
}
impl Outbox {
    pub const fn new() -> Self {
        Self {
            active: None,
            latest: None,
            dead: false,
        }
    }
    pub fn queue(&mut self, text: Text, paint: u64) {
        if self.dead {
            return;
        }
        let b = Batch {
            text,
            paint,
            clear: false,
            chunks: (text.len + 7) / 8,
        };
        if self.active.is_none() {
            self.active = Some(b)
        } else {
            self.latest = Some(b)
        }
    }
    pub fn pending(&self) -> bool {
        !self.dead && (self.active.is_some() || self.latest.is_some())
    }
    pub fn close(&mut self) {
        if let Some(b) = &mut self.active {
            b.text.bytes.fill(0)
        }
        if let Some(b) = &mut self.latest {
            b.text.bytes.fill(0)
        }
        self.active = None;
        self.latest = None;
        self.dead = true;
    }
    pub fn poll(
        &mut self,
        sid: u64,
        mut send: impl FnMut(u64, u64, u64, u64) -> Result<(), u64>,
        mut done: impl FnMut(u64, usize),
    ) {
        if self.dead {
            return;
        }
        for _ in 0..4 {
            if self.active.is_none() {
                self.active = self.latest.take()
            }
            let Some(b) = self.active.as_ref() else {
                return;
            };
            let (op, a, packed) = if !b.clear {
                (0xFA, 0, 0)
            } else {
                let off = (b.chunks - 1) * 8;
                let n = (b.text.len - off).min(8);
                let mut raw = [0; 8];
                raw[..n].copy_from_slice(&b.text.bytes[off..off + n]);
                (
                    0xFB,
                    u64::from_le_bytes(raw),
                    (off as u64 & 255)
                        | ((n as u64) << 8)
                        | ((off as u64 >> 8) << 12)
                        | ((sex_pdx::desktop_style::TEXT as u64) << 32),
                )
            };
            match send(op, sid, a, packed) {
                Ok(()) => {}
                Err(sex_pdx::ERR_SERVICE_NOT_READY) => return,
                Err(_) => {
                    self.close();
                    return;
                }
            }
            let b = self.active.as_mut().unwrap();
            if !b.clear {
                b.clear = true
            } else {
                b.chunks -= 1
            }
            if b.chunks == 0 {
                done(b.paint, b.text.len);
                b.text.bytes.fill(0);
                self.active = None;
            }
        }
    }
}
