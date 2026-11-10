# SPDX-License-Identifier: GPL-3.0-only
"""Mechanical checks for docs/DESIGN_GUARDRAILS.md.

The kernel keeps mechanism only, never grows a Linux-shaped API, never links a
compatibility layer, and every Linux-code bridge has a row in docs/BRIDGE_LEDGER.md.
"""
from pathlib import Path
import re
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
KERNEL = ROOT / "kernel"

# Non-blank lines of Rust under kernel/src. Raising it is allowed: say why in the
# budget table of docs/DESIGN_GUARDRAILS.md in the same PR.
KERNEL_LINE_BUDGET = 21_500

# Compatibility layers and userspace stacks: libraries a program links, never the kernel.
COMPAT_CRATES = {
    "sex-libc", "sex-std", "sex-app", "sex-net", "sex-tls", "sex-fetch",
    "relibc", "sexc", "tuxedo", "libsys",
}

# Kernel source files whose name says "Unix process model / Linux driver layer".
LINUX_SHAPED = re.compile(
    r"^(fork|pipe|exec|execve|signal|signals|socket|sockets|ioctl|errno|posix|"
    r"linux|dde|tuxedo|translator|vfs|tty)\.rs$"
)


def kernel_sources():
    return sorted(p for p in (KERNEL / "src").rglob("*") if p.is_file())


class KernelGuardrails(unittest.TestCase):
    def test_kernel_does_not_depend_on_compat_layers(self):
        manifest = tomllib.loads((KERNEL / "Cargo.toml").read_text())
        names = set()
        def collect(table):
            for key in ("dependencies", "build-dependencies", "dev-dependencies"):
                names.update(table.get(key, {}).keys())
        collect(manifest)
        for target in manifest.get("target", {}).values():
            collect(target)
        self.assertEqual(sorted(names & COMPAT_CRATES), [],
                         "the kernel must not link a compatibility layer (DESIGN_GUARDRAILS 2)")

    def test_no_linux_shaped_kernel_modules(self):
        bad = [str(p.relative_to(ROOT)) for p in kernel_sources() if LINUX_SHAPED.match(p.name)]
        self.assertEqual(bad, [], "fork/pipe/socket/... belong in a library or server (DESIGN_GUARDRAILS 2)")

    def test_no_stray_files_in_kernel_source(self):
        bad = [str(p.relative_to(ROOT)) for p in kernel_sources()
               if ".bak" in p.name or p.suffix == ".patch" or ":" in p.name]
        self.assertEqual(bad, [], "backups and editor leftovers do not live in kernel/src")

    def test_kernel_line_budget(self):
        lines = 0
        for p in kernel_sources():
            if p.suffix == ".rs":
                lines += sum(1 for line in p.read_text(errors="replace").splitlines() if line.strip())
        self.assertLessEqual(lines, KERNEL_LINE_BUDGET,
                             f"kernel/src has {lines} non-blank lines; over budget. Move the feature "
                             "to a server, or raise the budget with a reason in docs/DESIGN_GUARDRAILS.md")


    def test_kernel_never_runs_firmware_aml(self):
        # AML is untrusted firmware code: it runs in the ring-3 sexacpi server, under PKU.
        # The kernel parses static tables only (DESIGN_GUARDRAILS 1, audit item A).
        bad = [str(p.relative_to(ROOT)) for p in kernel_sources() if p.suffix == ".rs"
               and re.search(r"\bInterpreter\s*::\s*new\b|aml::Interpreter|\.evaluate(_if_present)?\s*\(",
                             p.read_text(errors="replace"))]
        self.assertEqual(bad, [], "firmware AML belongs in servers/sexacpi, not the kernel")


class BridgeLedger(unittest.TestCase):
    def test_every_bridge_has_a_ledger_row(self):
        ledger = (ROOT / "docs" / "BRIDGE_LEDGER.md").read_text()
        bridges_table = ledger.split("## 1.", 1)[1].split("## 2.", 1)[0]
        missing = []
        for manifest_path in sorted(ROOT.glob("*/*/Cargo.toml")):
            top = manifest_path.parts[len(ROOT.parts)]
            if top not in ("crates", "servers", "apps"):
                continue
            component = manifest_path.parent
            rel = str(component.relative_to(ROOT))
            if rel == "servers/tuxedo":
                continue  # the adapter itself, not a bridge
            try:
                manifest = tomllib.loads(manifest_path.read_text())
            except tomllib.TOMLDecodeError:
                continue
            licence = manifest.get("package", {}).get("license", "")
            deps = set(manifest.get("dependencies", {}).keys())
            gpl2_only = "GPL-2.0-only" in licence or any(
                "SPDX-License-Identifier: GPL-2.0-only" in p.read_text(errors="replace")[:400]
                for p in component.rglob("*") if p.suffix in (".rs", ".c", ".h") and p.is_file())
            if gpl2_only or "tuxedo" in deps:
                if rel not in bridges_table:
                    missing.append(rel)
        self.assertEqual(missing, [],
                         "Linux-code bridges need a row with their REA replacement in docs/BRIDGE_LEDGER.md")


if __name__ == "__main__":
    unittest.main()
