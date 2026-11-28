//! The Dreamspell (13 Moon) kin count: 260 kin of 20 solar seals and 13
//! galactic tones, its wave spells and the destiny oracle (guide, analog,
//! antipode, occult). This is the modern Argüelles count, a different count
//! from the traditional Tzolk'in in [`crate::Tzolkin`]: it skips 29 February
//! (that day keeps 28 February's kin) and is anchored on 21 December 2012 =
//! Kin 207, Blue Crystal Hand.

use crate::{Gregorian, SexTimestamp};

/// Seal 1..=20 (20 = Yellow Sun) and tone 1..=13 of one kin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kin(u16);

pub const SEALS: [&str; 20] = [
    "Red Dragon", "White Wind", "Blue Night", "Yellow Seed", "Red Serpent",
    "White Worldbridger", "Blue Hand", "Yellow Star", "Red Moon", "White Dog",
    "Blue Monkey", "Yellow Human", "Red Skywalker", "White Wizard", "Blue Eagle",
    "Yellow Warrior", "Red Earth", "White Mirror", "Blue Storm", "Yellow Sun",
];
/// Icon names (`assets/icons/icons.txt`, `line seal-*`), in seal order.
pub const SEAL_ICONS: [&str; 20] = [
    "seal-dragon", "seal-wind", "seal-night", "seal-seed", "seal-serpent",
    "seal-worldbridger", "seal-hand", "seal-star", "seal-moon", "seal-dog",
    "seal-monkey", "seal-human", "seal-skywalker", "seal-wizard", "seal-eagle",
    "seal-warrior", "seal-earth", "seal-mirror", "seal-storm", "seal-sun",
];
pub const TONES: [&str; 13] = [
    "Magnetic", "Lunar", "Electric", "Self-Existing", "Overtone", "Rhythmic", "Resonant",
    "Galactic", "Solar", "Planetary", "Spectral", "Crystal", "Cosmic",
];

/// Seal colour: the four seal colours repeat red, white, blue, yellow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Colour { Red, White, Blue, Yellow }

/// The four companions of a kin, the destiny oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Oracle { pub destiny: Kin, pub guide: Kin, pub analog: Kin, pub antipode: Kin, pub occult: Kin }

impl Kin {
    /// Kin 1..=260.
    pub const fn new(number: u16) -> Option<Self> { if number >= 1 && number <= 260 { Some(Self(number)) } else { None } }
    /// The kin with `seal` 1..=20 and `tone` 1..=13 (one exists for every pair).
    pub fn from_seal_tone(seal: u8, tone: u8) -> Option<Self> {
        if !(1..=20).contains(&seal) || !(1..=13).contains(&tone) { return None; }
        (0..13u16).map(|k| k * 20 + seal as u16).find(|n| (n - 1) % 13 + 1 == tone as u16).and_then(Self::new)
    }
    pub const fn number(self) -> u16 { self.0 }
    pub const fn seal(self) -> u8 { ((self.0 - 1) % 20 + 1) as u8 }
    pub const fn tone(self) -> u8 { ((self.0 - 1) % 13 + 1) as u8 }
    pub fn seal_name(self) -> &'static str { SEALS[self.seal() as usize - 1] }
    pub fn tone_name(self) -> &'static str { TONES[self.tone() as usize - 1] }
    pub fn icon(self) -> &'static str { SEAL_ICONS[self.seal() as usize - 1] }
    pub const fn colour(self) -> Colour {
        match self.seal() % 4 { 1 => Colour::Red, 2 => Colour::White, 3 => Colour::Blue, _ => Colour::Yellow }
    }

    /// The kin of a civil day (`ts` already shifted into the person's zone).
    pub fn from_timestamp(ts: SexTimestamp) -> Option<Self> {
        let g = Gregorian::from_timestamp(ts)?;
        let day = Gregorian { hour: 0, minute: 0, second: 0, ..g };
        // 29 February keeps the kin of 28 February.
        let day = if g.month == 2 && g.day == 29 { Gregorian { day: 28, ..day } } else { day };
        let days = day.to_timestamp()?.utc_day(); // days since 2012-12-21
        let leap = leap_days_before(day.year, day.month) - leap_days_before(2012, 12);
        let n = (206 + days - leap).rem_euclid(260) + 1;
        Self::new(n as u16)
    }
    pub fn from_unix(seconds: i64) -> Option<Self> { Self::from_timestamp(SexTimestamp::from_unix(seconds, 0)?) }

    /// The wave spell (13 kin) this kin is in: its first kin, a tone-1 kin.
    pub fn wave_spell(self) -> Kin { Kin(((self.0 - 1) / 13 * 13 + 1) as u16) }
    /// Which day of its wave spell, 1..=13 (the same as its tone).
    pub const fn wave_day(self) -> u8 { self.tone() }
    /// Wave spell 1..=20.
    pub const fn wave_number(self) -> u8 { ((self.0 - 1) / 13 + 1) as u8 }

    pub fn oracle(self) -> Oracle {
        let (s, t) = (self.seal() as i32, self.tone());
        let seal = |v: i32| (v - 1).rem_euclid(20) as u8 + 1;
        let guide_step = match t { 1 | 6 | 11 => 0, 2 | 7 | 12 => 12, 3 | 8 | 13 => 4, 4 | 9 => 16, _ => 8 };
        let at = |sl: u8, tn: u8| Kin::from_seal_tone(sl, tn).unwrap_or(self);
        Oracle {
            destiny: self,
            guide: at(seal(s + guide_step), t),
            analog: at(seal(19 - s), t),
            antipode: at(seal(s + 10), t),
            occult: at(seal(21 - s), 14 - t),
        }
    }
}

/// 29 Februaries before `year`-`month` (a month after February counts its
/// own year's), from year 0.
fn leap_days_before(year: i64, month: u8) -> i64 {
    let through = |y: i64| y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400);
    through(year - 1) + (crate::is_leap_year(year) && month > 2) as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    fn day(y: i64, m: u8, d: u8) -> Kin {
        Kin::from_timestamp(Gregorian { year: y, month: m, day: d, hour: 12, minute: 0, second: 0 }.to_timestamp().unwrap()).unwrap()
    }
    #[test]
    fn anchors() {
        assert_eq!(day(2012, 12, 21).number(), 207); // Blue Crystal Hand
        assert_eq!((day(2012, 12, 21).seal_name(), day(2012, 12, 21).tone_name()), ("Blue Hand", "Crystal"));
        assert_eq!(day(2013, 7, 26).number(), 164); // Yellow Galactic Seed, 13 Moon new year
        assert_eq!(day(2026, 11, 28).number(), 94); // Electric White Wizard
        assert_eq!(day(2026, 11, 27).number(), 93);
        assert_eq!(day(2024, 2, 29), day(2024, 2, 28));
        assert_eq!(day(2024, 3, 1).number(), day(2024, 2, 28).number() % 260 + 1);
        assert_eq!(day(2012, 12, 20).number(), 206);
        assert_eq!(day(2012, 2, 29), day(2012, 2, 28));
        assert_eq!(day(2012, 3, 1).number(), day(2012, 2, 28).number() % 260 + 1);
        // Every year advances 365 kin = 105 mod 260, leap or not.
        for y in 1990..2100 {
            assert_eq!(day(y + 1, 7, 26).number(), (day(y, 7, 26).number() - 1 + 105) % 260 + 1, "{y}");
        }
    }
    #[test]
    fn oracle_of_kin_94() {
        let k = Kin::new(94).unwrap();
        let o = k.oracle();
        assert_eq!(o.guide.seal_name(), "White Mirror");
        assert_eq!(o.antipode.seal_name(), "Yellow Seed");
        assert_eq!(o.analog.seal_name(), "Red Serpent");
        assert_eq!((o.occult.seal_name(), o.occult.tone_name()), ("Blue Hand", "Spectral"));
        assert_eq!(o.occult.number(), 261 - 94);
        assert_eq!(k.wave_spell().seal_name(), "Yellow Human");
        assert_eq!((k.wave_number(), k.wave_day()), (8, 3));
        assert_eq!(k.colour(), Colour::White);
        for n in 1..=260 { let k = Kin::new(n).unwrap(); assert_eq!(Kin::from_seal_tone(k.seal(), k.tone()), Some(k)); assert_eq!(k.oracle().occult.number(), 261 - n); }
    }
}
