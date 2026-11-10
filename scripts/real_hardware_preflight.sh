#!/usr/bin/env bash
# real_hardware_preflight.sh — read-only target inventory, not boot qualification.
#
# SAFE: Log-only checks. No kernel edits, no bootloader changes, no structural changes.
# Run on the TARGET machine BEFORE attempting to boot SexOS from USB/CD.
#
# Usage:
#   ./scripts/real_hardware_preflight.sh
#
# Returns:
#   0 if all preflight checks PASS or WARN
#   1 if any CRITICAL check fails

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="${SEXOS_PREFLIGHT_REPO_ROOT:-$(cd "$SCRIPT_DIR/.." && pwd)}"
PROC_ROOT="${SEXOS_PREFLIGHT_PROC_ROOT:-/proc}"
SYS_ROOT="${SEXOS_PREFLIGHT_SYS_ROOT:-/sys}"

echo "============================================"
echo " SEXOS TARGET INVENTORY"
echo "============================================"
echo ""
echo "Inventory only: verify actual PKU/IOMMU operation in the later SexOS boot log."
if [ -n "${SEXOS_PREFLIGHT_REPO_ROOT:-}${SEXOS_PREFLIGHT_PROC_ROOT:-}${SEXOS_PREFLIGHT_SYS_ROOT:-}" ]; then
    echo "FIXTURE/OVERRIDE PATHS: this run is not real-hardware qualification evidence."
fi

PASS=0
WARN=0
FAIL=0
SKIP=0

pass()  { echo "  [PASS] $1"; PASS=$((PASS + 1)); }
warn()  { echo "  [WARN] $1"; WARN=$((WARN + 1)); }
fail()  { echo "  [FAIL] $1"; FAIL=$((FAIL + 1)); }
skip()  { echo "  [SKIP] $1"; SKIP=$((SKIP + 1)); }

# ---- 1. CPU Features ----
echo "--- CPU FEATURES ---"

# PKU check via /proc/cpuinfo
if [ -r "$PROC_ROOT/cpuinfo" ]; then
    if grep -Eq '(^|[[:space:]])pku([[:space:]]|$)' "$PROC_ROOT/cpuinfo"; then
        pass "Host reports CPU PKU support; SexOS PKRU enforcement remains unverified"
    else
        fail "CPU PKU support not reported; this inventory cannot qualify the isolated daily-driver target"
    fi
else
    skip "Cannot read /proc/cpuinfo (not Linux or no /proc)"
fi

# x86-64 check
if [ "$(uname -m 2>/dev/null)" = "x86_64" ]; then
    pass "x86-64 architecture confirmed"
else
    arch_val=$(uname -m 2>/dev/null || echo "unknown")
    fail "Architecture is '$arch_val' (expected x86_64). Run this inventory on the target."
fi

# ---- 2. Serial Port ----
echo ""
echo "--- SERIAL PORT ---"

if [ -d "$SYS_ROOT/class/tty/ttyS0" ]; then
    pass "Host exposes ttyS0; this does not establish its address or usable SexOS serial output"
else
    warn "ttyS0 not detected. Arrange supported boot-log capture; a USB adapter is not proof of COM1 support."
fi

# Check serial-USB adapter
for dev in /dev/ttyUSB*; do
    if [ -c "$dev" 2>/dev/null ]; then
        warn "Host USB-serial adapter found: $dev; this is not SexOS USB-serial driver qualification"
        break
    fi
done 2>/dev/null || true

# ---- 3. Memory ----
echo ""
echo "--- MEMORY ---"

total_ram_kb=$(awk '/^MemTotal:/ {print $2; found=1; exit} END {if (!found) print 0}' "$PROC_ROOT/meminfo" 2>/dev/null || echo 0)
case "$total_ram_kb" in ''|*[!0-9]*) total_ram_kb=0 ;; esac
if [ "$total_ram_kb" -gt 524288 ]; then
    pass "Sufficient RAM: $((total_ram_kb / 1024)) MB (minimum 512 MB required)"
else
    warn "Low RAM: $((total_ram_kb / 1024)) MB (minimum 512 MB for SexOS)"
fi

# ---- 4. UEFI/BIOS Mode ----
echo ""
echo "--- FIRMWARE ---"

if [ -d "$SYS_ROOT/firmware/efi" ]; then
    pass "Booted in UEFI mode (Limine supports UEFI)"
else
    warn "Booted in legacy BIOS/CSM mode (Limine supports both, but verify boot method)"
fi

if [ -d "$SYS_ROOT/firmware/efi" ] && command -v mokutil &>/dev/null; then
    if ! sb_state=$(mokutil --sb-state 2>/dev/null); then
        warn "Secure Boot state could not be read; verify firmware before boot"
    elif printf '%s\n' "$sb_state" | grep -qi "enabled"; then
        warn "Secure Boot is ENABLED. SexOS does not support Secure Boot. Disable in firmware."
    elif printf '%s\n' "$sb_state" | grep -qi "disabled"; then
        pass "Secure Boot is disabled"
    else
        warn "Secure Boot state is unknown; verify firmware before boot"
    fi
fi

# ---- 5. IOMMU / VT-d / AMD-Vi ----
echo ""
echo "--- DMA REMAPPING / CPU VIRTUALIZATION ---"

if [ -f "$SYS_ROOT/firmware/acpi/tables/DMAR" ]; then
    pass "Host exposes Intel DMAR firmware table; SexOS unit/device translation still requires boot evidence"
elif [ -f "$SYS_ROOT/firmware/acpi/tables/IVRS" ]; then
    pass "Host exposes AMD IVRS firmware table; SexOS unit/device translation still requires boot evidence"
else
    warn "No readable DMAR/IVRS path found; DMA-remapping support is unknown, not disproved"
fi

if grep -Eq '(^|[[:space:]])(svm|vmx)([[:space:]]|$)' "$PROC_ROOT/cpuinfo" 2>/dev/null; then
    pass "Host reports CPU virtualization; VMX/SVM does not prove IOMMU support or DMA isolation"
else
    skip "Virtualization not detected (not required for SexOS)"
fi
echo "         SexOS implements VT-d/AMD-Vi; retain [iommu.unit.enable] and per-device translation receipts."
echo "         iommu=off selects an explicitly unprotected diagnostic path, not an isolated release."

# ---- 6. USB Controller ----
echo ""
echo "--- USB ---"

# Look for XHCI controller in lspci or /sys
if command -v lspci &>/dev/null; then
    xhci_count=$(lspci -d ::0c03 2>/dev/null | grep -i xhci | wc -l || true)
    if [ "$xhci_count" -gt 0 ]; then
        pass "XHCI USB controller(s) detected: $xhci_count"
        lspci -d ::0c03 2>/dev/null | grep -i xhci | head -3 | while read -r line; do
            echo "         $line"
        done
    else
        warn "No XHCI controller detected. SexOS requires USB XHCI for USB input."
    fi
elif [ -d "$SYS_ROOT/bus/pci/devices" ]; then
    xhci_count=$(find "$SYS_ROOT/bus/pci/devices" -name "class" -exec grep -l 0c0330 {} \; 2>/dev/null | wc -l)
    if [ "$xhci_count" -gt 0 ]; then
        pass "XHCI controller(s) detected: $xhci_count"
    else
        warn "No XHCI controller found via /sys (ProgIF 0x30)."
    fi
else
    skip "Cannot enumerate USB controllers (no lspci or /sys/bus/pci)"
fi

# ---- 7. Boot Device ----
echo ""
echo "--- ISO / BOOT MEDIUM ---"

ISO="$ROOT_DIR/sexos-v1.0.0.iso"

if [ -f "$ISO" ]; then
    iso_size=$(stat -c%s "$ISO" 2>/dev/null || stat -f%z "$ISO" 2>/dev/null || echo 0)
    if [ "$iso_size" -gt 1048576 ]; then
        pass "ISO exists: $ISO ($((iso_size / 1048576)) MB)"
    else
        warn "ISO is very small ($iso_size bytes). May be incomplete."
    fi

    # Check if limine bios-install has been run (search for Limine MBR signature)
    # Limine BIOS install writes signature bytes at specific offsets.
    # This is a soft check — we look for the string "LIMINE" in the ISO.
    if strings "$ISO" 2>/dev/null | grep -F "LIMINE" >/dev/null || \
       dd if="$ISO" bs=1 skip=3 count=6 2>/dev/null | grep -q "LIMINE" 2>/dev/null; then
        pass "ISO contains Limine boot signature"
    else
        warn "ISO may be missing 'limine bios-install'. Run: ./limine/limine bios-install sexos-v1.0.0.iso"
        warn "Verify bootloader installation on the selected image; this string check is not boot validation."
    fi
else
    fail "ISO not found: $ISO. Run: ./scripts/entrypoint_build.sh"
fi

# ---- 8. Limine Tool ----
echo ""
echo "--- LIMINE TOOL ---"

LIMINE_BIN="$ROOT_DIR/limine/limine"
if [ -f "$LIMINE_BIN" ] && [ -x "$LIMINE_BIN" ]; then
    pass "Limine tool found: $LIMINE_BIN"
    limine_ver=$("$LIMINE_BIN" --version 2>/dev/null || echo "version unknown")
    echo "         $limine_ver"
else
    fail "Limine tool not found at $LIMINE_BIN. Run: ./scripts/bootstrap_limine.sh"
fi

# ---- 9. UEFI Boot Files ----
echo ""
echo "--- UEFI BOOT ---"

EFI_FILE="$ROOT_DIR/limine/BOOTX64.EFI"
if [ -f "$EFI_FILE" ]; then
    pass "UEFI boot file found: $EFI_FILE"
else
    warn "UEFI boot file not found. UEFI boot may fail."
fi

# ---- 10. Cargo Target ----
echo ""
echo "--- BUILD TARGET ---"

TARGET_JSON="$ROOT_DIR/x86_64-sex.json"
if [ -f "$TARGET_JSON" ]; then
    pass "Target spec found: $TARGET_JSON"
else
    fail "Checked-in target spec missing: $TARGET_JSON"
fi

# ---- SUMMARY ----
echo ""
echo "============================================"
echo " PREFLIGHT SUMMARY"
echo "============================================"
echo ""
printf "  %-30s %3d\n" "PASS"  "$PASS"
printf "  %-30s %3d\n" "WARN"  "$WARN"
printf "  %-30s %3d\n" "FAIL"  "$FAIL"
printf "  %-30s %3d\n" "SKIP"  "$SKIP"
echo ""

if [ "$FAIL" -gt 0 ]; then
    echo "[RESULT] CRITICAL FAILURES DETECTED. Review FAIL items before boot."
    exit 1
elif [ "$WARN" -gt 4 ]; then
    echo "[RESULT] Many warnings. Review recommended before hardware boot."
    exit 0
else
    echo "[RESULT] Inventory checks complete. This does not qualify a SexOS boot or daily-driver release."
    exit 0
fi
