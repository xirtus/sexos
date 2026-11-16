"""Compare the native asset oracle with actual compiled production geometry.

Only bitmap_batch::Geometry is substituted: both geometry algorithms, text
composition, shared metrics and catalog dependencies come from production files.
"""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import https_asset_gate as asset

WIDTHS = (240, 256, 566, 1200)
HEIGHTS = (144, 375, 768)


def compile_records(directory, style):
    """Execute production Rust; do not translate its arithmetic into the test."""
    directory = Path(directory)
    modules = {
        "app_catalog": ROOT / "crates/sex-pdx/src/app_catalog.rs",
        "desktop_style": style,
        "asset_view": ROOT / "apps/kaleidoscope/src/asset_view.rs",
    }
    program = "extern crate self as sex_pdx;\n"
    for name, path in modules.items():
        program += f"#[path={json.dumps(str(path))}] pub mod {name};\n"
    program += r'''
pub mod bitmap_batch {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Geometry {
        pub x: u16, pub y: u16, pub width: u8, pub height: u8, pub scale: u8,
    }
}
fn main() {
    for w in [240, 256, 566, 1200] {
        for h in [144, 375, 768] {
            for (name, mode) in [("Fit", asset_view::Mode::Fit),
                                 ("Actual", asset_view::Mode::Actual)] {
                let geometry = asset_view::geometry(w, h, 48, 48, mode);
                let frame = asset_view::render(w, h, asset_view::Status::Ready, mode, 48, 48);
                let geometry = geometry.map(|g| format!("{},{},{},{},{}",
                    g.x, g.y, g.width, g.height, g.scale)).unwrap_or_else(|| "none".into());
                let text: String = frame.bytes[..frame.len].iter().map(|b| format!("{b:02x}")).collect();
                println!("{w}\t{h}\t{name}\t{geometry}\t{}\t{}\t{}\t{text}",
                    frame.cols, frame.rows, frame.len);
            }
        }
    }
}
'''
    source, executable = directory / "probe.rs", directory / "probe"
    source.write_text(program)
    subprocess.run(["rustc", "--edition=2021", "-A", "dead_code", str(source),
                    "-o", str(executable)], check=True, capture_output=True, timeout=60)
    output = subprocess.check_output([str(executable)], text=True, timeout=10)
    records = {}
    for line in output.splitlines():
        w, h, mode, geometry, cols, rows, length, text = line.split("\t")
        records[(int(w), int(h), mode)] = dict(
            geometry=None if geometry == "none" else tuple(map(int, geometry.split(","))),
            cols=int(cols), rows=int(rows), bytes=int(length), text=bytes.fromhex(text))
    if len(records) != len(WIDTHS) * len(HEIGHTS) * 2:
        raise AssertionError("production harness did not emit every requested view/mode")
    return records


class AssetSourceMetricsTests(unittest.TestCase):
    def test_compiled_production_geometry_and_full_text_match_oracle(self):
        metrics = asset.source_metrics(ROOT)
        with tempfile.TemporaryDirectory(prefix="sexos-asset-metrics-") as temporary:
            records = compile_records(temporary, ROOT / "crates/sex-pdx/src/desktop_style.rs")
        fitted = rejected = 0
        for (w, h, mode), actual in records.items():
            with self.subTest(width=w, height=h, mode=mode):
                tile = dict(x=0, y=0, w=w, h=h)
                if actual["geometry"] is None:
                    with self.assertRaises(ValueError):
                        asset.image_geometry(tile, mode, metrics=metrics)
                    rejected += 1
                    continue
                expected = asset.image_geometry(tile, mode, metrics=metrics)
                self.assertEqual(tuple(expected[k] for k in ("x", "y", "width", "height", "scale")),
                                 actual["geometry"])
                text = asset.asset_text(tile, mode, metrics=metrics)
                for field in ("cols", "rows", "bytes"):
                    self.assertEqual(text[field], actual[field], field)
                self.assertEqual("\n".join(text["lines"]).encode(), actual["text"])
                fitted += 1
        self.assertGreater(fitted, 0)
        self.assertGreater(rejected, 0)

    def test_changed_production_title_moves_rust_geometry_but_is_not_self_approved(self):
        with tempfile.TemporaryDirectory(prefix="sexos-asset-title-mutation-") as temporary:
            root = Path(temporary)
            normal = root / "normal"
            changed = root / "changed"
            normal.mkdir()
            changed.mkdir()
            style = ROOT / "crates/sex-pdx/src/desktop_style.rs"
            original = style.read_text()
            mutated, count = re.subn(r"(pub const TITLE: u32 = )26;", r"\g<1>31;", original)
            self.assertEqual(count, 1)
            target = root / "crates/sex-pdx/src/desktop_style.rs"
            target.parent.mkdir(parents=True)
            target.write_text(mutated)
            baseline = compile_records(normal, style)
            moved = compile_records(changed, target)
            self.assertTrue(any(baseline[key]["geometry"] != moved[key]["geometry"]
                                for key in baseline), "mutation did not exercise production title math")
            # A self-consistent source/image pair cannot silently replace the
            # separately approved chrome contract used by the pixel oracle.
            with self.assertRaises(ValueError):
                asset.source_metrics(root)


if __name__ == "__main__":
    unittest.main()
