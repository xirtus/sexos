#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Mirrors Polyamour from this tree, with its history, to github.com/xirtus/polyamour.
#
# The mirror's history is this repository's history filtered to Polyamour's paths
# (git filter-repo, deterministic), plus one last commit that adds the mirror's
# README and LICENSE. Each run rebuilds that and force-pushes `main`, so never
# commit to the mirror directly: change Polyamour here.
#
#   scripts/polyamour/mirror.sh [REF] [REMOTE-URL]
#
# REF defaults to origin/master; REMOTE-URL to https://github.com/xirtus/polyamour.
# Needs git-filter-repo (pip install git-filter-repo).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
REF=${1:-origin/master}
URL=${2:-https://github.com/xirtus/polyamour}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
PATHS=(apps/polyamour crates/sex-3d scripts/polyamour scripts/program_polyamour_gate.py)
for f in "$ROOT"/docs/handoff/POLYAMOUR_*; do PATHS+=("docs/handoff/$(basename "$f")"); done

SHA=$(git -C "$ROOT" rev-parse --verify "$REF^{commit}")
git clone -q --no-local --no-checkout "$ROOT" "$WORK/mirror"
cd "$WORK/mirror"
# A clone carries only the source's branches and tags; REF may be a remote-tracking
# ref or a bare commit, so fetch it by name into a ref the clone owns.
git fetch -q "$ROOT" "$SHA:refs/heads/mirror-source"
git checkout -q -B main "$SHA"
args=()
for p in "${PATHS[@]}"; do args+=(--path "$p"); done
git filter-repo --force --refs main "${args[@]}" >/dev/null
cp "$ROOT/scripts/polyamour/mirror/README.md" README.md
cp "$ROOT/LICENSE" LICENSE
git add README.md LICENSE
git -c user.name="Polyamour mirror" -c user.email="polyamour-mirror@users.noreply.github.com" \
  commit -q -m "Mirror of SexOS $(git -C "$ROOT" rev-parse --short "$REF"): README and LICENSE"
git remote add mirror "$URL"
git push --force mirror main:main
