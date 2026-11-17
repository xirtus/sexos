// SPDX-License-Identifier: GPL-3.0-only
use sex_icons::*;

#[test]
fn every_icon_parses_and_draws_with_ink_at_small_and_large_sizes() {
    let mut count = 0;
    for side in [16usize, 24, 48, 96] {
        let (a, m) = scratch(side);
        let (mut acc, mut mask, mut px) = (vec![0; a], vec![0; m], vec![0u32; side * side]);
        for (kind, name, data) in all() {
            struct Nop;
            impl Sink for Nop {
                fn begin(&mut self, _: f32, _: f32, _: bool) {}
                fn line_to(&mut self, _: f32, _: f32) {}
                fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
                fn cubic_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
                fn close(&mut self) {}
                fn circle(&mut self, _: f32, _: f32, _: f32, _: bool) {}
            }
            assert!(walk(data, &mut Nop), "{name} does not parse");
            assert!(draw(kind, name, side, &Palette::BRAND, &mut acc, &mut mask, &mut px), "{name}");
            let ink = px.iter().filter(|p| **p >> 24 > 128).count();
            assert!(ink * 100 >= side * side, "{name} at {side}px has almost no ink");
            count += 1;
        }
    }
    assert!(count >= 4 * 100);
}

#[test]
fn every_app_and_file_icon_has_its_svg_and_png() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icons");
    for (kind, name, _) in all() {
        let dir = match kind { Kind::Line => "line", Kind::App => "app", Kind::File => "file" };
        assert!(root.join(format!("svg/{dir}/{name}.svg")).exists(), "run scripts/brand/icons.py: {name}");
        if kind != Kind::Line {
            for px in [16, 24, 32, 48, 64, 128, 256] {
                assert!(root.join(format!("png/{px}/{name}.png")).exists(), "run scripts/brand/icons.py: {name} {px}");
            }
        }
    }
}

#[test]
fn programs_and_files_find_their_icons() {
    assert_eq!(app_for("files"), Some("loom"));
    assert_eq!(app_for("spindle"), Some("spindle"));
    assert_eq!(app_for("term"), Some("spindle"));
    assert_eq!(app_for("nonesuch"), None);
    assert_eq!(file_for(b"Holiday.JPG", false), "image");
    assert_eq!(file_for(b"notes", false), "file");
    assert_eq!(file_for(b"music", true), "folder");
    for n in ["folder", "text", "pdf", "image", "raw", "video", "audio", "archive", "file"] {
        assert!(find(Kind::File, n).is_some(), "{n}");
    }
    for n in ["wifi", "wifi-off", "volume", "volume-off", "battery", "bell", "bluetooth", "power"] {
        assert!(find(Kind::Line, n).is_some(), "{n}");
    }
}

#[test]
fn bad_data_is_refused() {
    let (a, m) = scratch(24);
    let (mut acc, mut mask) = (vec![0; a], vec![0; m]);
    assert!(!coverage("M1 2 3", Place::square(24), 2.0, 24, 24, &mut acc, &mut mask));
    assert!(!coverage("X1 2", Place::square(24), 2.0, 24, 24, &mut acc, &mut mask));
    assert!(coverage("M2 2 L22 22", Place::square(24), 2.0, 24, 24, &mut acc, &mut mask));
}
