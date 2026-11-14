// SPDX-License-Identifier: GPL-3.0-only
//! The password generator: uniform choices from the classes asked for, at least one
//! from each, from the system's entropy.
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub length: usize,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    /// Leave out characters that look alike: `0 O o 1 l I |`.
    pub avoid_ambiguous: bool,
}

impl Default for Options {
    fn default() -> Self { Self { length: 20, lower: true, upper: true, digits: true, symbols: true, avoid_ambiguous: true } }
}

pub const MIN_LENGTH: usize = 4;
pub const MAX_LENGTH: usize = 128;
const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
const SYMBOLS: &str = "!#$%&()*+,-./:;<=>?@[]^_{|}~";
const AMBIGUOUS: &str = "0Oo1lI|";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerateError { NoClasses, Length, Entropy }

impl Options {
    /// The character classes in use, each as its characters.
    pub fn classes(&self) -> Vec<Vec<u8>> {
        [(self.lower, LOWER), (self.upper, UPPER), (self.digits, DIGITS), (self.symbols, SYMBOLS)].iter()
            .filter(|(on, _)| *on)
            .map(|(_, set)| set.bytes().filter(|b| !(self.avoid_ambiguous && AMBIGUOUS.as_bytes().contains(b))).collect())
            .collect()
    }

    /// Bits of entropy of a password made with these options (an upper bound; the
    /// one-of-each rule takes a little away).
    pub fn entropy_bits(&self) -> f64 {
        let n: usize = self.classes().iter().map(Vec::len).sum();
        if n == 0 { 0.0 } else { self.length as f64 * (n as f64).log2() }
    }
}

/// A password from the system's entropy.
pub fn generate(options: &Options) -> Result<Zeroizing<String>, GenerateError> {
    generate_with(options, &mut |buf| getrandom::fill(buf).map_err(|_| GenerateError::Entropy))
}

/// A password, with random bytes from `fill` (the tests give their own).
pub fn generate_with(options: &Options, fill: &mut dyn FnMut(&mut [u8]) -> Result<(), GenerateError>) -> Result<Zeroizing<String>, GenerateError> {
    let classes = options.classes();
    if classes.is_empty() { return Err(GenerateError::NoClasses); }
    if options.length < MIN_LENGTH.max(classes.len()) || options.length > MAX_LENGTH { return Err(GenerateError::Length); }
    let all: Vec<u8> = classes.concat();
    let mut out = Zeroizing::new(vec![0u8; options.length]);
    // One from each class first, the rest from all of them, then shuffled, so every
    // class appears and no position is predictable.
    for (i, class) in classes.iter().enumerate() { out[i] = class[below(class.len(), fill)?]; }
    for slot in out.iter_mut().skip(classes.len()) { *slot = all[below(all.len(), fill)?]; }
    for i in (1..out.len()).rev() { let j = below(i + 1, fill)?; out.swap(i, j); }
    Ok(Zeroizing::new(String::from_utf8(out.to_vec()).expect("ASCII")))
}

/// A uniform number below `n` (n <= 256), by rejection so no value is favoured.
fn below(n: usize, fill: &mut dyn FnMut(&mut [u8]) -> Result<(), GenerateError>) -> Result<usize, GenerateError> {
    debug_assert!((1..=256).contains(&n));
    let limit = 256 - 256 % n;
    loop {
        let mut b = [0u8; 1];
        fill(&mut b)?;
        if (b[0] as usize) < limit { return Ok(b[0] as usize % n); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift, so the property tests are repeatable.
    fn rng(seed: u64) -> impl FnMut(&mut [u8]) -> Result<(), GenerateError> {
        let mut s = seed.max(1);
        move |buf: &mut [u8]| { for b in buf { s ^= s << 13; s ^= s >> 7; s ^= s << 17; *b = s as u8; } Ok(()) }
    }

    #[test]
    fn length_and_classes_hold() {
        let mut r = rng(7);
        for length in [4, 5, 12, 20, 64, 128] {
            for mask in 1u8..16 {
                let o = Options { length, lower: mask & 1 != 0, upper: mask & 2 != 0, digits: mask & 4 != 0, symbols: mask & 8 != 0, avoid_ambiguous: length % 2 == 0 };
                let p = generate_with(&o, &mut r).unwrap();
                assert_eq!(p.len(), length);
                let has = |set: &str| p.bytes().any(|b| set.as_bytes().contains(&b));
                assert_eq!(has(LOWER), o.lower, "{o:?} {}", &*p);
                assert_eq!(has(UPPER), o.upper);
                assert_eq!(has(DIGITS), o.digits);
                assert_eq!(has(SYMBOLS), o.symbols);
                if o.avoid_ambiguous { assert!(!has(AMBIGUOUS), "{}", &*p); }
            }
        }
    }

    #[test]
    fn refuses_what_cannot_be_made() {
        let mut r = rng(1);
        let none = Options { lower: false, upper: false, digits: false, symbols: false, ..Options::default() };
        assert_eq!(generate_with(&none, &mut r), Err(GenerateError::NoClasses));
        assert_eq!(generate_with(&Options { length: 3, ..Options::default() }, &mut r), Err(GenerateError::Length));
        assert_eq!(generate_with(&Options { length: 129, ..Options::default() }, &mut r), Err(GenerateError::Length));
        assert_eq!(generate_with(&Options::default(), &mut |_| Err(GenerateError::Entropy)), Err(GenerateError::Entropy));
    }

    #[test]
    fn choices_are_spread_evenly() {
        // 20,000 digits-only characters: each digit near 2,000 (well within 25%).
        let o = Options { length: 100, lower: false, upper: false, digits: true, symbols: false, avoid_ambiguous: false };
        let mut r = rng(99);
        let mut counts = [0usize; 10];
        for _ in 0..200 { for b in generate_with(&o, &mut r).unwrap().bytes() { counts[(b - b'0') as usize] += 1; } }
        assert!(counts.iter().all(|c| (1500..2500).contains(c)), "{counts:?}");
    }

    #[test]
    fn system_entropy_gives_different_passwords() {
        let a = generate(&Options::default()).unwrap();
        let b = generate(&Options::default()).unwrap();
        assert_ne!(*a, *b);
        assert!(Options::default().entropy_bits() > 100.0);
    }
}
