"""Compile the real no_std mail-route modules and run their identity tests."""
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
PDX = ROOT / "crates" / "sex-pdx" / "src"


class MailBrokerRouteTests(unittest.TestCase):
    def test_exact_caller_and_no_account_fail_closed(self):
        with tempfile.TemporaryDirectory(prefix="sexos-mail-route-") as tmp:
            source = Path(tmp) / "route.rs"
            binary = Path(tmp) / "route-tests"
            source.write_text(
                "\n".join(
                    f'#[path = "{PDX / (module + ".rs")}"] mod {module};'
                    for module in ("app_catalog", "browser_sources", "web", "mail_broker")
                ) + "\n"
            )
            subprocess.run(
                ["rustc", "--edition=2021", "--test", "-A", "dead_code", str(source), "-o", str(binary)],
                check=True, cwd=ROOT, timeout=90,
            )
            subprocess.run([str(binary), "mail_broker::tests::"], check=True, cwd=ROOT, timeout=30)


if __name__ == "__main__":
    unittest.main()
