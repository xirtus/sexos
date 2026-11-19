// SPDX-License-Identifier: GPL-3.0-only
//! Render the exact retained game UI on the host for visual inspection.
use std::{io::Write, path::Path};
use velvet::Velvet;
use weft::{Response, Strings, Theme, Ui};
fn main() {
    let out = std::env::args().nth(1).expect("output directory");
    std::fs::create_dir_all(&out).unwrap();
    let mut ui = Ui::new(640, 400, Theme::light(), Strings::new("en"));
    let mut v = Velvet::build(&mut ui, 42);
    for (name, command) in [
        ("snags", "velvet.snags"),
        ("mingle", "velvet.mingle"),
        ("twin-flames", "velvet.twins"),
    ] {
        v.on(&mut ui, &Response::Command(command.into()));
        let mut pixels = vec![0u32; 640 * 400];
        ui.paint(&mut pixels);
        let mut f = std::fs::File::create(Path::new(&out).join(format!("{name}.ppm"))).unwrap();
        f.write_all(b"P6\n640 400\n255\n").unwrap();
        for p in pixels {
            f.write_all(&[
                ((p >> 16) & 255) as u8,
                ((p >> 8) & 255) as u8,
                (p & 255) as u8,
            ])
            .unwrap();
        }
    }
}
