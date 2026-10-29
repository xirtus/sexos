#!/usr/bin/env bash
# master_runtime_gate.sh — SexOS Master Runtime Proof Gate V1.
#
# Builds, boots QEMU, captures serial log, and scans for:
#   - All 6 PDs spawn and run (sexdisplay, sexdrive, silk-shell, sexinput, silkbar, linen)
#   - Clock/silkbar liveness marker
#   - No fault.kill, no panic, no #PF/#GP
#   - Scheduler round-robins all PDs (task.running for each)
#
# Usage:
#   ./scripts/master_runtime_gate.sh [--skip-build] [--probe N] [--keep-log] [--nvme|--no-nvme]
#
# Options:
#   --skip-build   Use existing ISO (sexos-v1.0.0.iso) instead of rebuilding
#   --probe N      Wait N seconds for runtime probe (default: 25)
#   --keep-log     Keep the serial log after gate run
#   --no-nvme      Disable QEMU NVMe test device (default: enabled)
#   --nvme         Enable QEMU NVMe test device
#
# Environment: GATE_DIR isolates run artifacts; REPORT_PATH overrides the
# default $GATE_DIR/report.md. Custom ISO is supported only with --skip-build.
# Historical handoff reports are preserved by default.
#
# Returns:
#   0 if ALL gates PASS
#   1 if any gate FAILS
#
# See: docs/handoff/MASTER_RUNTIME_GATE_V1.md

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# ---- config ----
SKIP_BUILD=false
PROBE_SECONDS=25
KEEP_LOG=false
ENABLE_NVME="${SEXOS_GATE_NVME:-1}"
PERSIST_REBOOT_PROOF="${SEXOS_PERSISTENCE_REBOOT_PROOF:-0}"
PRESERVE_NVME_IMG="${SEXOS_NVME_PRESERVE_IMG:-0}"
GATE_DIR="${GATE_DIR:-$ROOT_DIR/.gate_master}"
LOG="${LOG_PATH:-$GATE_DIR/serial.log}"
LOG_A="${GATE_DIR}/serial.boot_a.log"
LOG_B="${GATE_DIR}/serial.boot_b.log"
QEMU_PID=""
ISO="${ISO:-sexos-v1.0.0.iso}"
QEMU_BIN="${QEMU_BIN:-qemu-system-x86_64}"
REPORT_PATH="${REPORT_PATH:-$GATE_DIR/report.md}"
PROVENANCE_PATH="$GATE_DIR/provenance.json"
PROBE_GATE="FAIL"
PROBE_STATUS="not_run"
RUN_STARTED_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_STARTED_SECONDS=$SECONDS

# ---- gate state ----
BUILD_GATE="FAIL"
SPAWN_GATE="FAIL"
CLOCK_GATE="FAIL"
SCHED_GATE="FAIL"
FAULT_GATE="FAIL"
BOOTGRAPH_GATE="FAIL"
CAP_GRANT_GATE="FAIL"
ORDER_GATE="FAIL"
BOOTGRAPH_CLOCK_GATE="FAIL"
SEXFILES_GATE="FAIL"
FINAL_SCORE="RED_MASTER"
SEXFILES_BOOT_PROOF="${SEXOS_SEXFILES_BOOT_PROOF:-0}"

# ---- arg parse ----
while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build) SKIP_BUILD=true; shift ;;
        --probe)
            if [[ $# -lt 2 || ! "$2" =~ ^[1-9][0-9]*$ ]]; then
                echo "error: --probe requires a positive integer number of seconds" >&2
                exit 1
            fi
            PROBE_SECONDS="$2"; shift 2 ;;
        --keep-log)   KEEP_LOG=true; shift ;;
        --no-nvme)    ENABLE_NVME=0; shift ;;
        --nvme)       ENABLE_NVME=1; shift ;;
        *)
            echo "error: unknown option: $1"
            echo "usage: $0 [--skip-build] [--probe N] [--keep-log] [--nvme|--no-nvme]"
            exit 1 ;;
    esac
done

mkdir -p "$GATE_DIR" "$(dirname "$LOG")" "$(dirname "$REPORT_PATH")"
BUILD_LOG="$GATE_DIR/build.log"
# Start a new observation even when reusing GATE_DIR. Preserve only the NVMe
# image when requested; commands and diagnostics below append within this run
# so persistence boot A and B remain together without including an older run.
rm -f "$REPORT_PATH" "$PROVENANCE_PATH" "$BUILD_LOG" \
    "$LOG" "$LOG_A" "$LOG_B" "$GATE_DIR/qmp.sock"
: > "$GATE_DIR/qemu-command.txt"
: > "$GATE_DIR/qemu.log"

# The sealed entrypoint packages exactly this repository-relative output.
# A successful build cannot establish provenance for an arbitrary ISO override.
if [[ "$SKIP_BUILD" != true && "$ISO" != "sexos-v1.0.0.iso" ]]; then
    echo "error: custom ISO requires --skip-build; sealed builds produce sexos-v1.0.0.iso" >&2
    exit 1
fi

# ---- helpers ----
cleanup() {
    set +e
    if [[ -n "$QEMU_PID" ]] && kill -0 "$QEMU_PID" 2>/dev/null; then
        kill "$QEMU_PID" 2>/dev/null || true
        sleep 1
        kill -9 "$QEMU_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

has_marker() {
    local pattern="$1"
    grep -qE "$pattern" "$LOG" 2>/dev/null && echo 1 || echo 0
}

count_marker() {
    local pattern="$1"
    local count
    count="$(grep -cE "$pattern" "$LOG" 2>/dev/null || true)"
    # grep -c returns exit=1 for zero matches but still prints "0";
    # normalize to a single numeric token for arithmetic tests.
    count="${count//$'\n'/}"
    if [[ -z "$count" ]]; then
        echo 0
    else
        echo "$count"
    fi
}

print_gate() {
    local name="$1" state="$2"
    printf '  %-28s %s\n' "$name" "$state"
}

# Fingerprint production inputs on both sides of the sealed build. This also
# covers untracked new modules; a concurrent source edit invalidates the bind.
capture_production_sources() {
    python3 - "$1" <<'PYSOURCES'
import hashlib,json,pathlib,sys
root=pathlib.Path('.')
files=[]
for directory in ['kernel','crates','servers','apps','sex-rt','purple-scanout','sexc','.cargo','targets']:
    base=root/directory
    if base.is_dir():
        files += [p for p in base.rglob('*') if p.is_file() and 'target' not in p.parts
                  and p.suffix in ['.rs','.toml','.ld','.json','.S','.asm']]
for name in ['Cargo.toml','Cargo.lock','rust-toolchain.toml','limine.cfg','sexos_contract.toml','sexos_build_spec.toml','ARCHITECTURE.md','x86_64-sex.json','aarch64-sex.json']:
    p=root/name
    if p.is_file():files.append(p)
result={str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(set(files))}
pathlib.Path(sys.argv[1]).write_text(json.dumps(result,sort_keys=True,indent=2)+"\n")
PYSOURCES
}
SOURCE_BEFORE="$GATE_DIR/build-sources.before.json"
SOURCE_AFTER="$GATE_DIR/build-sources.after.json"
rm -f "$SOURCE_BEFORE" "$SOURCE_AFTER"

# ---- 1. BUILD ----
echo ""
echo "============================================"
echo " MASTER RUNTIME GATE V1"
echo "============================================"
echo ""

if [ "$SKIP_BUILD" = true ]; then
    echo "[gate] --skip-build: using existing ISO"
    if [ ! -f "$ISO" ]; then
        echo "[FAIL] ISO not found: $ISO"
        exit 1
    fi
    BUILD_GATE="SKIP"
else
    echo "[gate] BUILD phase..."
    capture_production_sources "$SOURCE_BEFORE"
    if ./scripts/entrypoint_build.sh >"$BUILD_LOG" 2>&1 \
        && capture_production_sources "$SOURCE_AFTER" \
        && cmp -s "$SOURCE_BEFORE" "$SOURCE_AFTER"; then
        BUILD_GATE="PASS"
        echo "[gate] BUILD: PASS"
    else
        BUILD_GATE="FAIL"
        echo "[gate] BUILD: FAIL"
        echo "[gate] Build log: $BUILD_LOG"
        # A failed build cannot authorize a probe of a stale root ISO.
        # Preserve a fresh failure report without binding any image as tested.
        python3 - "$PROVENANCE_PATH" "$RUN_STARTED_UTC" <<'PYBUILDFAIL'
import json,pathlib,sys
pathlib.Path(sys.argv[1]).write_text(json.dumps({
    "started_utc":sys.argv[2],"build_gate":"FAIL","probe_gate":"NOT_RUN",
    "source_image_binding":"build_failed_no_image_tested","iso_sha256":None
},indent=2)+"\n")
PYBUILDFAIL
        cat > "$REPORT_PATH" <<EOFBUILDFAIL
# Master runtime gate

| BUILD_GATE | FAIL |
| PROBE_GATE | NOT_RUN |
| FINAL_SCORE | RED_MASTER |

Build failed. No existing image was booted or accepted. See $BUILD_LOG.
EOFBUILDFAIL
        echo "[gate] FINAL_SCORE: RED_MASTER (build failed; probe not run)"
        exit 1
    fi
fi

# ---- 2. BOOT + PROBE ----
echo ""
echo "[gate] BOOT+PROBE phase (${PROBE_SECONDS}s)..."
mkdir -p "$GATE_DIR"
rm -f "$LOG" "$GATE_DIR/qmp.sock"
# Pre-create log path so --keep-log always leaves a deterministic file artifact.
: > "$LOG"

NVME_IMG="${GATE_DIR}/nvme.img"
NVME_ARGS=()
NVME_QEMU_DESC="(nvme disabled)"
if [ "$ENABLE_NVME" = "1" ]; then
    # Tiny deterministic probe image in gate dir.
    if [ "$PERSIST_REBOOT_PROOF" = "1" ] || [ "$PRESERVE_NVME_IMG" = "1" ]; then
        if [ ! -f "$NVME_IMG" ]; then
            dd if=/dev/zero of="$NVME_IMG" bs=512 count=2048 2>/dev/null || true
        fi
    else
        dd if=/dev/zero of="$NVME_IMG" bs=512 count=2048 2>/dev/null || true
    fi
    if "$QEMU_BIN" -device help 2>/dev/null | grep -E 'name[[:space:]]+"nvme"[[:space:]]*,|(^|[[:space:],])nvme([[:space:],]|$)' >/dev/null; then
        NVME_ARGS=(
            -drive "if=none,id=nvm,file=${NVME_IMG},format=raw"
            -device "nvme,serial=sexos01,drive=nvm"
        )
        NVME_QEMU_DESC="-drive if=none,id=nvm,file=$NVME_IMG,format=raw -device nvme,serial=sexos01,drive=nvm"
    else
        echo "[FAIL] QEMU nvme device not supported by this QEMU build."
        echo "[hint] use --no-nvme (or SEXOS_GATE_NVME=0) for absent-path proof."
        exit 1
    fi
else
    echo "[gate] NVMe disabled; absent marker path expected."
fi

# Record what was actually tested. With --skip-build the source/image
# relationship is deliberately unverified; hashes identify evidence, not origin.
python3 - "$PROVENANCE_PATH" "$ISO" "$SKIP_BUILD" "$RUN_STARTED_UTC" "$BUILD_GATE" "$SOURCE_BEFORE" <<'PYPROVENANCE'
import hashlib, json, os, pathlib, subprocess, sys
out, iso, skip, started, build_gate, source_manifest = sys.argv[1:]
def git(*args):
    return subprocess.check_output(["git", *args])
image = pathlib.Path(iso)
def sha256_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()
image_hash = sha256_file(image) if image.is_file() else None
revision = git("rev-parse", "HEAD").decode().strip()
status = git("status", "--porcelain").decode()
source_diff = git("diff", "HEAD", "--binary")
untracked = {}
for raw in git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0"):
    if raw:
        path = pathlib.Path(os.fsdecode(raw))
        if path.is_file():
            untracked[str(path)] = sha256_file(path)
production_sources=json.loads(pathlib.Path(source_manifest).read_text()) if build_gate=="PASS" else None
kernel=pathlib.Path('target/x86_64-sex/release/sex-kernel')
kernel_hash=sha256_file(kernel) if build_gate=="PASS" and kernel.is_file() else None
data = dict(production_source_sha256=production_sources,kernel_sha256=kernel_hash,
            sources_unchanged_during_build=build_gate=="PASS",started_utc=started, git_revision=revision, git_status=status,
            tracked_diff_sha256=hashlib.sha256(source_diff).hexdigest(),
            untracked_file_sha256=untracked, build_gate=build_gate,
            iso_path=str(image.resolve()), iso_sha256=image_hash,
            source_image_binding="sealed_build_this_run" if build_gate == "PASS" else "unverified_existing_image")
pathlib.Path(out).write_text(json.dumps(data, indent=2) + "\n")
PYPROVENANCE

run_boot_once() {
    local boot_log="$1"
    local args=(
        -M q35 -m 512M -cpu max,+pku -cdrom "$ISO"
        -device nec-usb-xhci,id=xhci -device usb-tablet,bus=xhci.0
        ${NVME_ARGS[@]+"${NVME_ARGS[@]}"} -serial "file:$boot_log"
        -display none -no-reboot -no-shutdown
    )
    printf '%q ' "$QEMU_BIN" "${args[@]}" >> "$GATE_DIR/qemu-command.txt"
    printf '\n' >> "$GATE_DIR/qemu-command.txt"
    "$QEMU_BIN" "${args[@]}" >> "$GATE_DIR/qemu.log" 2>&1 &
    QEMU_PID=$!
    echo "[gate] QEMU PID: $QEMU_PID; log: $boot_log"
}

finish_probe() {
    local exit_status=0
    sleep "$PROBE_SECONDS"
    # QEMU must stay alive throughout the observation window. A process which
    # exits early with even status 0 is not a complete runtime observation.
    if ! kill -0 "$QEMU_PID" 2>/dev/null; then
        wait "$QEMU_PID" || exit_status=$?
        PROBE_STATUS="early_exit:$exit_status"
        QEMU_PID=""
        return 1
    fi
    kill "$QEMU_PID" 2>/dev/null || true
    # Bound shutdown and reap the child before starting a second persistence boot.
    for _ in 1 2 3; do
        if ! kill -0 "$QEMU_PID" 2>/dev/null; then break; fi
        sleep 1
    done
    if kill -0 "$QEMU_PID" 2>/dev/null; then
        kill -9 "$QEMU_PID" 2>/dev/null || true
        PROBE_STATUS="forced_kill"
        wait "$QEMU_PID" 2>/dev/null || true
        QEMU_PID=""
        return 1
    fi
    wait "$QEMU_PID" || exit_status=$?
    QEMU_PID=""
    # QEMU handles SIGTERM as exit 0; test wrappers may return 128+SIGTERM.
    if [[ "$exit_status" != 0 && "$exit_status" != 143 ]]; then
        PROBE_STATUS="shutdown_exit:$exit_status"
        return 1
    fi
    PROBE_STATUS="observed_full_window"
    return 0
}

if [ "$PERSIST_REBOOT_PROOF" = "1" ]; then
    : > "$LOG_A"
    : > "$LOG_B"
    run_boot_once "$LOG_A"
else
    run_boot_once "$LOG"
fi
if finish_probe; then
    PROBE_GATE="PASS"
else
    echo "[FAIL] QEMU probe incomplete: $PROBE_STATUS (see $GATE_DIR/qemu.log)"
fi

if [ "$PERSIST_REBOOT_PROOF" = "1" ] && [ "$PROBE_GATE" = "PASS" ]; then
    run_boot_once "$LOG_B"
    if ! finish_probe; then
        PROBE_GATE="FAIL"
        echo "[FAIL] QEMU boot B incomplete: $PROBE_STATUS"
    fi
    LOG="$LOG_B"
fi

# ---- 3. SCAN LOG ----
echo ""
echo "[gate] SCAN phase..."

if [ ! -f "$LOG" ]; then
    echo "[FAIL] No serial log found at $LOG"
    FINAL_SCORE="RED_MASTER"
fi

LOG_LINES=$(wc -l < "$LOG" 2>/dev/null || echo 0)
echo "[gate] Log lines: $LOG_LINES"
echo ""

# --- SPAWN GATE: all 6 PDs must be spawned ---
echo "--- SPAWN CHECKS ---"

PD_LIST=(
    "sext"
    "sexdisplay"
    "sexdrive"
    "silk-shell"
    "sexinput"
    "silkbar"
    "linen"
)

MISSING_PD=""
for pd in "${PD_LIST[@]}"; do
    if [ "$(has_marker "Spawned PD.*${pd}")" -eq 1 ]; then
        print_gate "  v $pd" "spawned"
    else
        print_gate "  x $pd" "MISSING"
        MISSING_PD="$pd"
    fi
done

if [ -z "$MISSING_PD" ]; then
    SPAWN_GATE="PASS"
else
    SPAWN_GATE="FAIL"
fi
echo ""

# --- CLOCK GATE: silkbar clock must tick ---
echo "--- CLOCK CHECK ---"
CLOCK_COUNT=$(count_marker "silkbar\.clock\.send")
if [ "$CLOCK_COUNT" -ge 2 ]; then
    CLOCK_GATE="PASS"
    echo "  v silkbar.clock.send: ${CLOCK_COUNT} ticks"
else
    CLOCK_GATE="FAIL"
    echo "  x silkbar.clock.send: ${CLOCK_COUNT} ticks (need >=2)"
fi
echo ""

# --- SCHED GATE: all PDs must have task.running evidence ---
echo "--- SCHEDULER CHECK ---"
# PD ID mapping: sexdisplay=1, sexdrive=2, silk-shell=3, sexinput=4,
#                 sexusb=5(bonus), silkbar=6, linen=7
PD_ID_MAP=(
    "1:sexdisplay"
    "2:sexdrive"
    "3:silk-shell"
    "4:sexinput"
    "6:silkbar"
    "7:linen"
)
MISSING_RUNNING=""
for entry in "${PD_ID_MAP[@]}"; do
    pd_id="${entry%%:*}"
    pd_name="${entry##*:}"
    count=$(count_marker "task\.running.*pd_id=${pd_id}([^0-9]|$)")
    if [ "$count" -ge 1 ]; then
        print_gate "  v $pd_name (PD $pd_id)" "running (${count}x)"
    else
        print_gate "  x $pd_name (PD $pd_id)" "NO task.running"
        MISSING_RUNNING="$pd_name"
    fi
done

if [ -z "$MISSING_RUNNING" ]; then
    SCHED_GATE="PASS"
else
    SCHED_GATE="FAIL"
fi
SCHED_ADVISORY="ADVISORY"
echo ""

# --- FAULT GATE: no panics, faults, kills ---
echo "--- FAULT CHECK ---"
FAULT_PATTERNS=(
    "panic"
    "PAGE FAULT"
    "#PF"
    "GENERAL PROTECTION"
    "GP FAULT"
    "#GP"
    "DOUBLE FAULT"
    "TRIPLE FAULT"
    "EXCEPTION: INVALID OPCODE"
    "EXCEPTION: UD ENTER"
    "PKU SECURITY VIOLATION"
    "TEST FAILED:"
    "fault\.kill"
    "FATAL"
)

FAULT_HIT=""
for pat in "${FAULT_PATTERNS[@]}"; do
    if grep -qiE "$pat" "$LOG" 2>/dev/null; then
        print_gate "  x pattern: $pat" "FOUND"
        FAULT_HIT="$pat"
    fi
done

if [ -z "$FAULT_HIT" ]; then
    FAULT_GATE="PASS"
    echo "  v No faults/panics detected"
else
    FAULT_GATE="FAIL"
fi
echo ""

# --- BOOTGRAPH PARSER GATES ---
echo "--- BOOTGRAPH CHECK ---"
BOOTGRAPH_CHECKER="$ROOT_DIR/scripts/check_bootgraph_log.py"
BOOTGRAPH_FAIL_LINE=""
if [ ! -x "$BOOTGRAPH_CHECKER" ]; then
    BOOTGRAPH_GATE="FAIL"
    CAP_GRANT_GATE="FAIL"
    ORDER_GATE="FAIL"
    BOOTGRAPH_CLOCK_GATE="FAIL"
    BOOTGRAPH_FAIL_LINE="BOOTGRAPH FAIL: CHECKER_MISSING path=$BOOTGRAPH_CHECKER"
    echo "  x checker missing: $BOOTGRAPH_CHECKER"
else
    CHECKER_STATUS=0
    BOOTGRAPH_OUT="$("$BOOTGRAPH_CHECKER" "$LOG" --allow-fault 2>&1)" || CHECKER_STATUS=$?
    BOOTGRAPH_FAIL_LINE="$(printf '%s\n' "$BOOTGRAPH_OUT" | sed -n '/^BOOTGRAPH FAIL:/{p;}' || true)"
    BOOTGRAPH_GATE="$(printf '%s\n' "$BOOTGRAPH_OUT" | awk '/^BOOTGRAPH_GATE:/ {print $2}')"
    CAP_GRANT_GATE="$(printf '%s\n' "$BOOTGRAPH_OUT" | awk '/^CAP_GRANT_GATE:/ {print $2}')"
    ORDER_GATE="$(printf '%s\n' "$BOOTGRAPH_OUT" | awk '/^ORDER_GATE:/ {print $2}')"
    BOOTGRAPH_CLOCK_GATE="$(printf '%s\n' "$BOOTGRAPH_OUT" | awk '/^CLOCK_GATE:/ {print $2}')"
    if [ -z "$BOOTGRAPH_GATE" ]; then BOOTGRAPH_GATE="FAIL"; fi
    if [ -z "$CAP_GRANT_GATE" ]; then CAP_GRANT_GATE="FAIL"; fi
    if [ -z "$ORDER_GATE" ]; then ORDER_GATE="FAIL"; fi
    if [ -z "$BOOTGRAPH_CLOCK_GATE" ]; then BOOTGRAPH_CLOCK_GATE="FAIL"; fi
    if [ "$CHECKER_STATUS" -ne 0 ] && [ -z "$BOOTGRAPH_FAIL_LINE" ]; then
        BOOTGRAPH_GATE="FAIL"
        BOOTGRAPH_FAIL_LINE="BOOTGRAPH FAIL: CHECKER_EXIT status=$CHECKER_STATUS"
    fi
    if [ -n "$BOOTGRAPH_FAIL_LINE" ]; then
        echo "  x $BOOTGRAPH_FAIL_LINE"
    else
        echo "  v BOOTGRAPH PASS"
    fi
fi
echo ""

# --- SEXFILES GATE: sexfiles must boot and emit ready marker ---
echo "--- SEXFILES CHECK ---"
SEXFILES_READY=$(has_marker '\[sexfiles\.ready\]')
SEXFILES_KSPAWN=$(has_marker '\[kernel\.spawn\.sexfiles\]')
SEXFILES_TASK_COUNT=$(count_marker 'task\.running.*pd_id=11([^0-9]|$)')

if [ "$SEXFILES_READY" -eq 1 ]; then
    print_gate "  v sexfiles.ready" "FOUND"
else
    print_gate "  x sexfiles.ready" "MISSING"
fi

if [ "$SEXFILES_KSPAWN" -eq 1 ]; then
    print_gate "  v kernel.spawn.sexfiles" "FOUND"
else
    print_gate "  x kernel.spawn.sexfiles" "MISSING"
fi

if [ "$SEXFILES_TASK_COUNT" -ge 1 ]; then
    print_gate "  v sexfiles (PD 11)" "running (${SEXFILES_TASK_COUNT}x)"
else
    print_gate "  x sexfiles (PD 11)" "NO task.running"
fi

if [ "$SEXFILES_READY" -eq 1 ] && [ "$SEXFILES_TASK_COUNT" -ge 1 ]; then
    SEXFILES_GATE="PASS"
elif [ "$SEXFILES_BOOT_PROOF" != "1" ]; then
    SEXFILES_GATE="SKIP"
    echo "  (i) SEXOS_SEXFILES_BOOT_PROOF not set — sexfiles gate skipped"
else
    SEXFILES_GATE="FAIL"
fi
echo ""
echo "============================================"
echo " MASTER RUNTIME GATE V1 - RESULTS"
echo "============================================"
echo ""

print_gate "BUILD_GATE" "$BUILD_GATE"
print_gate "PROBE_GATE" "$PROBE_GATE ($PROBE_STATUS)"
print_gate "SPAWN_GATE" "$SPAWN_GATE"
print_gate "FAULT_GATE" "$FAULT_GATE"
print_gate "BOOTGRAPH_GATE" "$BOOTGRAPH_GATE"
print_gate "CAP_GRANT_GATE" "$CAP_GRANT_GATE"
print_gate "ORDER_GATE" "$ORDER_GATE"
print_gate "CLOCK_GATE" "$BOOTGRAPH_CLOCK_GATE"
print_gate "SCHED_GATE" "$SCHED_GATE ($SCHED_ADVISORY)"
print_gate "SEXFILES_GATE" "$SEXFILES_GATE"

if { [ "$BUILD_GATE" = "PASS" ] || [ "$BUILD_GATE" = "SKIP" ]; } && [ "$PROBE_GATE" = "PASS" ]; then
    if [ "$SPAWN_GATE" = "PASS" ] && [ "$FAULT_GATE" = "PASS" ] \
        && [ "$BOOTGRAPH_GATE" = "PASS" ] && [ "$CAP_GRANT_GATE" = "PASS" ] \
        && [ "$ORDER_GATE" = "PASS" ] && [ "$BOOTGRAPH_CLOCK_GATE" = "PASS" ]; then
        if [ "$SEXFILES_GATE" = "PASS" ] || [ "$SEXFILES_GATE" = "SKIP" ]; then
            FINAL_SCORE="GREEN_MASTER"
        else
            FINAL_SCORE="YELLOW_MASTER"
        fi
    elif [ "$SPAWN_GATE" = "PASS" ] && [ "$FAULT_GATE" = "PASS" ] \
        && [ "$BOOTGRAPH_GATE" = "PASS" ] && [ "$CAP_GRANT_GATE" = "PASS" ] \
        && [ "$ORDER_GATE" = "PASS" ]; then
        FINAL_SCORE="YELLOW_MASTER"
    else
        FINAL_SCORE="RED_MASTER"
    fi
else
    FINAL_SCORE="RED_MASTER"
fi

echo ""
print_gate "FINAL_SCORE" "$FINAL_SCORE"
echo ""

# Emit typed block proof markers when requested so callers can pipe/grep stdout.
if [ "${SEXOS_SEXFILES_REAL_BLOCK_PROOF:-0}" = "1" ]; then
    echo "--- TYPED BLOCK PROOF MARKERS ---"
    grep -E 'sexfiles\.(realread|diskfs\.typed|block\.proof)\.|sexdrive\.block\.read\.api\.|sexdrive\.block\.typed|sexblock\.abi' "$LOG" || true
    echo ""
fi

if [ "$PERSIST_REBOOT_PROOF" = "1" ]; then
    echo "--- PERSISTENCE BOOT A MARKERS ---"
    grep -E 'sexfiles\.persistence\.boot_a|sexfiles\.persistence\.boot_b\.read_before_write\.mismatch|sexdrive\.nvme\.write\.' "$LOG_A" || true
    echo ""
    echo "--- PERSISTENCE BOOT B MARKERS ---"
    grep -E 'sexfiles\.persistence\.boot_b|sexdrive\.block\.read\.api|sexfiles\.diskfs\.typed\.read\.call' "$LOG_B" || true
    echo ""
fi

# Marker summary
m_spawn_total=$(count_marker "Spawned PD")
m_clock_total=$CLOCK_COUNT
m_task_total=$(count_marker "task\.running")
FAULT_REGEX=$(IFS='|'; echo "${FAULT_PATTERNS[*]}")
m_fault_total=$(grep -ciE "$FAULT_REGEX" "$LOG" 2>/dev/null || true)
m_fault_total=${m_fault_total:-0}

# ---- 5. HANDOFF DOC ----
cat > "$REPORT_PATH" << MDEOF
# MASTER_RUNTIME_GATE_V1

- date: $RUN_STARTED_UTC
- git commit: $(git rev-parse HEAD)
- provenance: $PROVENANCE_PATH (source dirty state, tracked diff hash, ISO SHA-256, build binding)
- source/image binding: $([ "$BUILD_GATE" = PASS ] && echo sealed_build_this_run || echo unverified_existing_image)
- qemu command: $GATE_DIR/qemu-command.txt
- qemu diagnostics: $GATE_DIR/qemu.log
- elapsed_seconds: $((SECONDS - RUN_STARTED_SECONDS))
- log_path: $LOG
- probe_seconds: $PROBE_SECONDS
- nvme_enabled: $ENABLE_NVME
- nvme_img: $NVME_IMG
- qemu: $QEMU_BIN -M q35 -m 512M -cpu max,+pku -cdrom $ISO -device nec-usb-xhci,id=xhci -device usb-tablet,bus=xhci.0 $NVME_QEMU_DESC -serial file:\$LOG -display none -no-reboot -no-shutdown

## Gate Results

| Gate | Status |
|------|--------|
| BUILD_GATE | $BUILD_GATE |
| PROBE_GATE | $PROBE_GATE ($PROBE_STATUS) |
| SPAWN_GATE | $SPAWN_GATE |
| FAULT_GATE | $FAULT_GATE |
| BOOTGRAPH_GATE | $BOOTGRAPH_GATE |
| BOOTGRAPH_CLOCK_GATE | $BOOTGRAPH_CLOCK_GATE |
| CAP_GRANT_GATE | $CAP_GRANT_GATE |
| ORDER_GATE | $ORDER_GATE |
| SEXFILES_GATE (non-scoring qualifier) | $SEXFILES_GATE |
| CLOCK_GATE (legacy visible clock check) | $CLOCK_GATE |
| SCHED_GATE (advisory) | $SCHED_GATE ($SCHED_ADVISORY) |
| **FINAL_SCORE** | **$FINAL_SCORE** |

## Marker Counts

- Spawned PDs: $m_spawn_total
- Clock ticks (silkbar.clock.send): $m_clock_total
- task.running total: $m_task_total
- Fault/panic hits: $m_fault_total

## PD Spawn Details

$(for pd in "${PD_LIST[@]}"; do
    if [ "$(has_marker "Spawned PD.*${pd}")" -eq 1 ]; then
        echo "- v $pd: spawned"
    else
        echo "- x $pd: MISSING"
    fi
done)

- sexfiles: SEXFILES_GATE=$SEXFILES_GATE ready=$SEXFILES_READY kspawn=$SEXFILES_KSPAWN

$(for entry in "${PD_ID_MAP[@]}"; do
    pd_id="${entry%%:*}"
    pd_name="${entry##*:}"
    count=$(count_marker "task\.running.*pd_id=${pd_id}([^0-9]|$)")
    echo "- $pd_name (PD $pd_id): task.running ${count}x"
done)

- sexfiles (PD 11): task.running ${SEXFILES_TASK_COUNT}x

## Clock Liveness

- silkbar.clock.send ticks: $m_clock_total
- Minimum required: 2

## FINAL_SCORE Criteria (exact)

Score uses these gates:

1. **Build prerequisite**: BUILD_GATE is PASS or SKIP; PROBE_GATE must PASS
2. **Hard scoring gates**: SPAWN_GATE, FAULT_GATE, BOOTGRAPH_GATE, CAP_GRANT_GATE, ORDER_GATE, BOOTGRAPH_CLOCK_GATE

Outcome logic:

- **GREEN_MASTER**: all hard scoring gates PASS and SEXFILES_GATE is PASS or SKIP
- **YELLOW_MASTER**: all hard scoring gates except BOOTGRAPH_CLOCK_GATE PASS, or hard gates PASS with SEXFILES_GATE not PASS/SKIP
- **RED_MASTER**: any other case (including BUILD_GATE FAIL)

Advisory/non-scoring rows:

- **SCHED_GATE** is advisory only and does not currently change FINAL_SCORE
- **CLOCK_GATE** is legacy visible clock check; FINAL_SCORE uses BOOTGRAPH_CLOCK_GATE
- **SEXFILES_GATE** is a non-scoring qualifier used only to distinguish GREEN vs YELLOW when hard gates pass

## Fail Criteria

- Any hard scoring gate fails
- QEMU fails to boot within probe window
- ISO missing when --skip-build used

## Run Command

\`\`\`
./scripts/master_runtime_gate.sh
\`\`\`

### Variants

\`\`\`
# Skip rebuild (use existing ISO)
./scripts/master_runtime_gate.sh --skip-build

# Custom probe duration (e.g., 30 seconds)
./scripts/master_runtime_gate.sh --probe 30

# Keep serial log after run
./scripts/master_runtime_gate.sh --keep-log
\`\`\`

## Known Notes

- sexusb (PD 5) is also present and running but excluded from the 6-PD gate requirement.
- purple-scanout module is loaded but is cosmetic/diagnostic only.
- QEMU is run headless (-display none); no window appears unless --display is overridden.
- Serial log is captured to $GATE_DIR/serial.log and preserved only with --keep-log.
MDEOF

HANDOFF_WROTE=1

echo "[gate] Wrote $REPORT_PATH"

# ---- 6. CLEANUP LOG ----
if [ "$KEEP_LOG" = false ]; then
    rm -f "$LOG"
    echo "[gate] Log cleaned (use --keep-log to preserve)"
fi

# ---- 7. EXIT ----
if [ "$FINAL_SCORE" = "GREEN_MASTER" ]; then
    echo "[gate] ALL CHECKS PASSED (GREEN)"
    exit 0
elif [ "$FINAL_SCORE" = "YELLOW_MASTER" ]; then
    echo "[gate] PARTIAL PASS (YELLOW) - review fault details"
    if [ -n "$BOOTGRAPH_FAIL_LINE" ]; then
        echo "[gate] $BOOTGRAPH_FAIL_LINE"
    fi
    exit 1
else
    echo "[gate] GATE FAILED (RED)"
    if [ -n "$BOOTGRAPH_FAIL_LINE" ]; then
        echo "[gate] $BOOTGRAPH_FAIL_LINE"
    fi
    exit 1
fi
