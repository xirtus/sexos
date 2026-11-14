#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Qupid external ProviderPack seed & ISO staging helper for SexOS.
#
# Public SexOS builds ship only clean open defaults (Cinemeta, Internet Archive,
# Academic Torrents, FOSS trackers, Official Kodi Omega mirror). This script lets a
# builder inject or scrub an external `provider-pack.json` locally without modifying
# any tracked source file:
#
#   ./scripts/qupid_pack.sh apply /path/to/provider-pack.json
#   ./scripts/qupid_pack.sh stage-iso /path/to/provider-pack.json [iso_root_dir]
#   ./scripts/qupid_pack.sh status [iso_root_dir]
#   ./scripts/qupid_pack.sh clean [iso_root_dir]
#   ./scripts/qupid_pack.sh verify-clean [iso_root_dir]

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SEED_DEST="${REPO_ROOT}/crates/sex-episodes/qupid_seed_pack.json"

validate_pack() {
    local src="$1"
    if [[ ! -f "$src" ]]; then
        echo "error: provider pack not found: $src" >&2
        exit 1
    fi
    python3 - "$src" <<'PY'
import json, sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as fh:
    doc = json.load(fh)

if not isinstance(doc, dict):
    raise SystemExit("error: provider pack root must be a JSON object")

name = doc.get("name", "Custom Provider Pack")
trackers = doc.get("trackers", [])
addons = doc.get("stremioAddons", [])
indexers = doc.get("swarmIndexers", [])
feeds = doc.get("featuredFeeds", [])
kodi = doc.get("kodiRepositories", [])

for key, val in [
    ("trackers", trackers),
    ("stremioAddons", addons),
    ("swarmIndexers", indexers),
    ("featuredFeeds", feeds),
    ("kodiRepositories", kodi),
]:
    if not isinstance(val, list):
        raise SystemExit(f"error: '{key}' must be a list")

print(
    f"validated '{name}': {len(addons)} Stremio addon(s), "
    f"{len(indexers)} swarm indexer(s), {len(feeds)} featured feed(s), "
    f"{len(trackers)} tracker(s), {len(kodi)} Kodi repo(s)"
)
PY
}

summarize_file() {
    local label="$1"
    local file="$2"
    python3 - "$label" "$file" <<'PY'
import json, sys

label, path = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as fh:
    doc = json.load(fh)
print(
    f"  {label}: SEEDED ({doc.get('name', 'Custom')}) — "
    f"addons={len(doc.get('stremioAddons', []))}, "
    f"indexers={len(doc.get('swarmIndexers', []))}, "
    f"feeds={len(doc.get('featuredFeeds', []))}, "
    f"trackers={len(doc.get('trackers', []))}, "
    f"kodi_repos={len(doc.get('kodiRepositories', []))}"
)
PY
}

cmd="${1:-status}"
case "$cmd" in
    apply)
        pack_src="${2:-}"
        iso_dir="${3:-${REPO_ROOT}/iso_root}"
        if [[ -z "$pack_src" ]]; then
            echo "usage: $0 apply <provider-pack.json> [iso_root_dir]" >&2
            exit 1
        fi
        validate_pack "$pack_src"
        cp "$pack_src" "$SEED_DEST"
        echo "seeded compile-time Qupid pack -> ${SEED_DEST#$REPO_ROOT/}"
        if [[ -d "$iso_dir" ]]; then
            mkdir -p "$iso_dir/cfg/qupid"
            cp "$pack_src" "$iso_dir/cfg/qupid/provider-pack.json"
            echo "staged runtime ISO Qupid pack  -> ${iso_dir#$REPO_ROOT/}/cfg/qupid/provider-pack.json"
        fi
        ;;
    stage-iso)
        pack_src="${2:-}"
        iso_dir="${3:-${REPO_ROOT}/iso_root}"
        if [[ -z "$pack_src" ]]; then
            echo "usage: $0 stage-iso <provider-pack.json> [iso_root_dir]" >&2
            exit 1
        fi
        validate_pack "$pack_src"
        mkdir -p "$iso_dir/cfg/qupid"
        cp "$pack_src" "$iso_dir/cfg/qupid/provider-pack.json"
        echo "staged runtime ISO Qupid pack -> ${iso_dir}/cfg/qupid/provider-pack.json"
        ;;
    clean)
        iso_dir="${2:-${REPO_ROOT}/iso_root}"
        rm -f "$SEED_DEST" "$iso_dir/cfg/qupid/provider-pack.json"
        echo "cleaned Qupid seed and ISO provider pack (public open defaults active)"
        ;;
    status)
        iso_dir="${2:-${REPO_ROOT}/iso_root}"
        echo "Qupid ProviderPack status in ${REPO_ROOT}:"
        if [[ -f "$SEED_DEST" ]]; then
            summarize_file "compile-time seed" "$SEED_DEST"
        else
            echo "  compile-time seed: CLEAN (Qupid Open Media Defaults)"
        fi
        if [[ -f "$iso_dir/cfg/qupid/provider-pack.json" ]]; then
            summarize_file "iso_root runtime " "$iso_dir/cfg/qupid/provider-pack.json"
        else
            echo "  iso_root runtime : CLEAN (not staged)"
        fi
        ;;
    verify-clean)
        iso_dir="${2:-${REPO_ROOT}/iso_root}"
        failed=0
        if [[ -f "$SEED_DEST" ]]; then
            echo "FAIL: compile-time seed pack still present: $SEED_DEST" >&2
            failed=1
        fi
        if [[ -f "$iso_dir/cfg/qupid/provider-pack.json" ]]; then
            echo "FAIL: staged ISO pack still present: $iso_dir/cfg/qupid/provider-pack.json" >&2
            failed=1
        fi
        if git -C "$REPO_ROOT" ls-files --error-unmatch crates/sex-episodes/qupid_seed_pack.json >/dev/null 2>&1; then
            echo "FAIL: crates/sex-episodes/qupid_seed_pack.json is tracked by git!" >&2
            failed=1
        fi
        if [[ "$failed" -ne 0 ]]; then
            exit 1
        fi
        echo "OK: Qupid tree is 100% clean (public open defaults only)."
        ;;
    *)
        echo "usage: $0 {apply|stage-iso|clean|status|verify-clean} ..." >&2
        exit 1
        ;;
esac
