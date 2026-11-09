// SPDX-License-Identifier: GPL-3.0-only
//! Pointer input for a program window: Shell to program as an input event (`app_key`
//! opcode 0x202, class [`CLASS`]). Positions are pixels from the top-left of the
//! window's pixel area; they may lie outside it while a button held down inside the
//! area keeps the pointer captured. Decoding authenticates no sender: programs check
//! the message came from Shell.
pub const CLASS: u64 = 7;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Move,
    /// A button went down: `button` is 0 left, 1 middle, 2 right.
    Down { button: u8 },
    Up { button: u8 },
    /// Wheel steps; positive moves toward the document start, like `app_scroll`.
    Wheel { steps: i8 },
    /// The pointer left the window's pixel area.
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pointer {
    pub x: i16,
    pub y: i16,
    /// Buttons held after this event, as the DOM's `buttons`: bit 0 left, 1 right, 2 middle.
    pub buttons: u8,
    pub kind: Kind,
}

impl Pointer {
    /// Layout: x (16 bits) | y (16) | buttons (8) | kind (4) | detail (8).
    pub const fn wire(self) -> u64 {
        let (kind, detail) = match self.kind {
            Kind::Move => (1u64, 0u8),
            Kind::Down { button } => (2, button),
            Kind::Up { button } => (3, button),
            Kind::Wheel { steps } => (4, steps as u8),
            Kind::Leave => (5, 0),
        };
        (self.x as u16 as u64) | (self.y as u16 as u64) << 16 | ((self.buttons & 7) as u64) << 32 | kind << 40 | (detail as u64) << 44
    }
    pub fn decode(word: u64, reserved: u64, class: u64) -> Option<Self> {
        if reserved != 0 || class != CLASS || word >> 52 != 0 || (word >> 35) & 0x1F != 0 { return None; }
        let detail = (word >> 44) as u8;
        let kind = match (word >> 40) & 0xF {
            1 if detail == 0 => Kind::Move,
            2 if detail < 3 => Kind::Down { button: detail },
            3 if detail < 3 => Kind::Up { button: detail },
            4 if detail != 0 => Kind::Wheel { steps: detail as i8 },
            5 if detail == 0 => Kind::Leave,
            _ => return None,
        };
        Some(Self { x: word as u16 as i16, y: (word >> 16) as u16 as i16, buttons: (word >> 32) as u8 & 7, kind })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_kind_round_trips() {
        for kind in [Kind::Move, Kind::Down { button: 0 }, Kind::Up { button: 2 }, Kind::Wheel { steps: -3 }, Kind::Wheel { steps: 127 }, Kind::Leave] {
            for (x, y) in [(0, 0), (409, 329), (-5, -1), (i16::MAX, i16::MIN)] {
                let p = Pointer { x, y, buttons: 5, kind };
                assert_eq!(Pointer::decode(p.wire(), 0, CLASS), Some(p));
            }
        }
    }
    #[test]
    fn rejects_other_classes_and_bad_words() {
        let p = Pointer { x: 1, y: 2, buttons: 1, kind: Kind::Move }.wire();
        assert_eq!(Pointer::decode(p, 1, CLASS), None);
        assert_eq!(Pointer::decode(p, 0, 6), None);
        assert_eq!(Pointer::decode(p | 1 << 60, 0, CLASS), None);
        assert_eq!(Pointer::decode(p & !(0xF << 40), 0, CLASS), None);
        assert_eq!(Pointer::decode(Pointer { x: 0, y: 0, buttons: 0, kind: Kind::Down { button: 0 } }.wire() | 3 << 44, 0, CLASS), None);
    }
}
