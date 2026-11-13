//! rustybuzz gives exactly HarfBuzz's glyphs, clusters and positions for 21 strings
//! (Arabic, Devanagari, Hebrew, Thai, combining marks, Latin ligatures and kerning),
//! recorded with `hb-shape` (HarfBuzz 8.3) by scripts/gen_shape_reference.py.
#![cfg(feature = "shaping")]
use sex_shape::shape::shape;

#[test]
fn matches_harfbuzz_reference() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
    let data = include_str!("data/hb_reference.tsv");
    let mut n = 0;
    for line in data.lines() {
        let mut parts = line.split('\t');
        let (font, text, want) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        let bytes = std::fs::read(format!("{root}{font}")).unwrap();
        let got: Vec<String> = shape(&bytes, text).unwrap().iter()
            // hb-shape counts clusters in characters; rustybuzz in bytes.
            .map(|g| format!("{},{},{},{},{},{}", g.id, text[..g.cluster as usize].chars().count(), g.x_advance, g.y_advance, g.x_offset, g.y_offset)).collect();
        assert_eq!(got.join(" "), want, "{text} in {font}");
        n += 1;
    }
    assert!(n >= 20);
}
