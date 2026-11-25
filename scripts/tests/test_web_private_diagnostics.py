"""Keep raw Kaleidoscope serial receipts limited to structural metadata."""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
ALLOWED = {
    "[web.loaded] bytes_ok=1 items={} links={} height={} sheets={} images={} fetch_ticks={} layout_ticks={} script_ticks={} scripted={}",
    "[web.ready] target={} task={}",
    "[web.tab.opened] index={} count={} box={}",
    "[web.tab.shown] index={} count={} url={}",
    "[web.private.forgotten] cookies={} origins={}",
    "[web.session.saved] tabs={} bytes={}",
    "[web.jarbox.saved] box={} cookies={} origins={}",
    "[web.closed] target={} task={}",
}


class PrivateDiagnosticsSourceTest(unittest.TestCase):
    def test_every_content_record_uses_private_gate(self):
        for path in (ROOT / "apps/web/src").glob("*.rs"):
            source = path.read_text()
            for line_no, line in enumerate(source.splitlines(), 1):
                if "sex_pdx::serial_record!(" not in line or "$($arg)" in line:
                    continue
                match = re.search(r'sex_pdx::serial_record!\("([^"]+)"', line)
                self.assertIsNotNone(match, f"{path}:{line_no} needs a literal format")
                self.assertIn(match.group(1), ALLOWED, f"{path}:{line_no} bypasses private diagnostics")
        tab_source = (ROOT / "apps/web/src/tabs.rs").read_text()
        self.assertIn("PRIVATE_DIAGNOSTICS.address(&self.url)", tab_source)


if __name__ == "__main__":
    unittest.main()
