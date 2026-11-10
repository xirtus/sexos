# SPDX-License-Identifier: GPL-3.0-only
"""Check Cargo package identities, including aliases and conditional dependencies.

Complements test_kernel_design_guardrails without changing that gate: Cargo's
local dependency name need not be the package that actually gets linked.
"""
from pathlib import Path
import re
import tomllib
import unittest

from test_kernel_design_guardrails import COMPAT_CRATES

ROOT = Path(__file__).resolve().parents[2]
DEPENDENCY_TABLES = ("dependencies", "build-dependencies", "dev-dependencies")


def dependency_packages(manifest, workspace):
    """Resolve names as Cargo does for direct, target and inherited dependencies."""
    packages = set()
    tables = [manifest, *manifest.get("target", {}).values()]
    inherited = workspace.get("workspace", {}).get("dependencies", {})
    for table in tables:
        for kind in DEPENDENCY_TABLES:
            for alias, dependency in table.get(kind, {}).items():
                if isinstance(dependency, dict) and dependency.get("workspace"):
                    dependency = inherited[alias]
                package = dependency.get("package", alias) if isinstance(dependency, dict) else alias
                packages.add(package)
    return packages


def bridge_rows(ledger):
    """Only exact component cells in the active bridge table count as rows."""
    section = ledger.split("## 1.", 1)[1].split("## 2.", 1)[0]
    rows = set()
    for line in section.splitlines():
        if line.lstrip().startswith("|"):
            cell = line.split("|")[1].strip().strip("`")
            if re.fullmatch(r"(?:crates|servers|apps)/[A-Za-z0-9_.-]+", cell):
                rows.add(cell)
    return rows


def component_license(manifest, workspace):
    license_value = manifest.get("package", {}).get("license", "")
    if isinstance(license_value, dict) and license_value.get("workspace"):
        return workspace["workspace"]["package"]["license"]
    return license_value


class DependencyIdentityFixtures(unittest.TestCase):
    def test_plain_and_renamed_packages(self):
        manifest = tomllib.loads('''
            [dependencies]
            ordinary = "1"
            libc_adapter = { package = "sex-libc", path = "../sex-libc" }
        ''')
        self.assertEqual(dependency_packages(manifest, {}), {"ordinary", "sex-libc"})

    def test_all_dependency_locations_resolve_aliases(self):
        for location in ("", "target.'cfg(target_arch = \"x86_64\")'."):
            for kind in DEPENDENCY_TABLES:
                with self.subTest(location=location, kind=kind):
                    manifest = tomllib.loads(
                        f'[{location}{kind}]\nbridge = {{ package = "tuxedo", version = "1" }}'
                    )
                    self.assertEqual(dependency_packages(manifest, {}), {"tuxedo"})

    def test_inherited_alias_and_optional_dependency(self):
        workspace = tomllib.loads('''
            [workspace.dependencies]
            compatibility = { package = "sex-std", path = "crates/sex-std" }
        ''')
        manifest = tomllib.loads('''
            [dependencies]
            compatibility = { workspace = true, optional = true }
        ''')
        self.assertEqual(dependency_packages(manifest, workspace), {"sex-std"})

    def test_inherited_string_dependency(self):
        self.assertEqual(dependency_packages(
            {"dependencies": {"tuxedo": {"workspace": True}}},
            {"workspace": {"dependencies": {"tuxedo": "1"}}}), {"tuxedo"})

    def test_missing_inheritance_fails_closed(self):
        with self.assertRaises(KeyError):
            dependency_packages({"dependencies": {"tuxedo": {"workspace": True}}}, {})

    def test_inherited_license(self):
        self.assertEqual(component_license(
            {"package": {"license": {"workspace": True}}},
            {"workspace": {"package": {"license": "GPL-2.0-only"}}}), "GPL-2.0-only")

    def test_ledger_requires_an_exact_component_cell(self):
        ledger = '''## 1. Bridges
Mentioning servers/unlisted here is not a row.
| Component | Replacement |
| `servers/real` | REA: real |
| servers/real-extra | REA: extra |
| crates/adapter | mentions servers/unlisted |
## 2. Native
| servers/native | native reference only |
'''
        self.assertEqual(bridge_rows(ledger), {
            "servers/real", "servers/real-extra", "crates/adapter"})


class RepositoryDependencyIdentity(unittest.TestCase):
    def test_kernel_package_identities_exclude_compatibility(self):
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
        manifest = tomllib.loads((ROOT / "kernel/Cargo.toml").read_text())
        self.assertEqual(sorted(dependency_packages(manifest, workspace) & COMPAT_CRATES), [],
                         "Renaming a dependency does not permit a compatibility layer in the kernel")

    def test_bridge_package_identities_have_exact_ledger_rows(self):
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
        registered = bridge_rows((ROOT / "docs/BRIDGE_LEDGER.md").read_text())
        missing = []
        for top in ("crates", "servers", "apps"):
            for path in sorted((ROOT / top).glob("*/Cargo.toml")):
                component = path.parent.relative_to(ROOT).as_posix()
                if component == "servers/tuxedo":
                    continue  # the adapter itself, not a driver using it
                manifest = tomllib.loads(path.read_text())
                packages = dependency_packages(manifest, workspace)
                license_value = component_license(manifest, workspace)
                gpl2_source = any(
                    "SPDX-License-Identifier: GPL-2.0-only" in p.read_text(errors="replace")[:400]
                    for p in path.parent.rglob("*") if p.is_file() and p.suffix in (".rs", ".c", ".h")
                )
                if "tuxedo" in packages or "GPL-2.0-only" in license_value or gpl2_source:
                    if component not in registered:
                        missing.append(component)
        self.assertEqual(missing, [],
                         "Bridges (including renamed/conditional dependencies) need an active ledger row")


if __name__ == "__main__":
    unittest.main()
