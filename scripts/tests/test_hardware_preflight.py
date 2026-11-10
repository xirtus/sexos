"""Run the read-only Bash inventory against explicitly labelled fixture roots."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "real_hardware_preflight.sh"


class HardwarePreflight(unittest.TestCase):
    def inventory(self, flags="pku vmx", table="DMAR", target=True, mok_status=0):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            repo, proc, sys, bins = [root / name for name in ("repo", "proc", "sys", "bin")]
            for directory in (repo, proc, sys / "firmware/efi", bins):
                directory.mkdir(parents=True)
            (proc / "cpuinfo").write_text(f"flags\t: {flags}\n")
            (proc / "meminfo").write_text("MemTotal: 8388608 kB\n")
            if table:
                tables = sys / "firmware/acpi/tables"
                tables.mkdir(parents=True)
                (tables / table).write_bytes(b"fixture")
            # A sibling target must never hide a missing checked-in target.
            (root / "x86_64-sex.json").write_text("{}")
            if target:
                (repo / "x86_64-sex.json").write_text("{}")
            limine = repo / "limine"
            limine.mkdir()
            (limine / "BOOTX64.EFI").write_bytes(b"fixture")
            with (repo / "sexos-v1.0.0.iso").open("wb") as image:
                image.write(b"LIMINE fixture\n")
                image.truncate(1048577)
            commands = {
                "uname": "echo x86_64",
                "lspci": "exit 0",
                "mokutil": f"echo 'SecureBoot disabled'; exit {mok_status}",
            }
            for name, body in commands.items():
                path = bins / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(0o755)
            tool = limine / "limine"
            tool.write_text("#!/bin/sh\necho fixture-limine\n")
            tool.chmod(0o755)
            env = dict(os.environ, PATH=str(bins) + os.pathsep + os.environ["PATH"],
                       SEXOS_PREFLIGHT_REPO_ROOT=str(repo), SEXOS_PREFLIGHT_PROC_ROOT=str(proc),
                       SEXOS_PREFLIGHT_SYS_ROOT=str(sys))
            result = subprocess.run(["bash", str(SCRIPT)], env=env, text=True,
                                    capture_output=True, timeout=10)
            self.assertEqual(result.stderr, "", result.stderr)
            self.assertIn("FIXTURE/OVERRIDE PATHS", result.stdout)
            return result

    def test_intel_table_is_inventory_not_dma_enforcement(self):
        result = self.inventory()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("Intel DMAR firmware table", result.stdout)
        self.assertIn("SexOS unit/device translation still requires boot evidence", result.stdout)
        self.assertIn("VMX/SVM does not prove IOMMU", result.stdout)
        self.assertNotIn("SexOS does not use IOMMU", result.stdout)

    def test_amd_table_and_cpu_are_independent(self):
        result = self.inventory(flags="pku svm", table="IVRS")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("AMD IVRS firmware table", result.stdout)
        self.assertIn("VMX/SVM does not prove IOMMU", result.stdout)

    def test_vmx_without_firmware_table_cannot_prove_dma_isolation(self):
        result = self.inventory(table=None)
        self.assertIn("DMA-remapping support is unknown", result.stdout)
        self.assertNotIn("[PASS] Host exposes Intel", result.stdout)
        self.assertIn("unprotected diagnostic path", result.stdout)

    def test_missing_pku_fails_daily_driver_inventory(self):
        result = self.inventory(flags="vmx")
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("CPU PKU support not reported", result.stdout)

    def test_pku_substring_does_not_count_as_flag(self):
        result = self.inventory(flags="not_pku vmx")
        self.assertEqual(result.returncode, 1, result.stdout)

    def test_sibling_target_cannot_mask_missing_repository_target(self):
        result = self.inventory(target=False)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("Checked-in target spec missing", result.stdout)

    def test_failed_mokutil_cannot_prove_secure_boot_disabled(self):
        result = self.inventory(mok_status=1)
        self.assertIn("Secure Boot state could not be read", result.stdout)
        self.assertNotIn("[PASS] Secure Boot is disabled", result.stdout)


if __name__ == "__main__":
    unittest.main()
