#!/usr/bin/env bash
# ci_scope.sh EVENT [BASE] — how much of CI a change needs: full, smoke or docs.
#
#   full   every gate (pull requests that touch code, nightly, manual runs)
#   smoke  build and boot the image once and refresh the cargo cache: a push to
#          master, whose pull request already passed every gate; the nightly
#          full run catches two green PRs that break each other
#   docs   only documents changed (*.md or docs/), and nothing outside the
#          documents names them, so no gate can read them; the root allowlist
#          still runs
#
# BASE is the commit to compare HEAD with (a pull request's merge commit has the
# base branch as its first parent, so HEAD^1 by default).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
event="${1:?usage: ci_scope.sh EVENT [BASE]}"
case "$event" in
  push) echo smoke; exit 0 ;;
  pull_request) ;;
  *) echo full; exit 0 ;;
esac
base="${2:-HEAD^1}"
mapfile -t changed < <(git diff --name-only "$base" HEAD) || { echo full; exit 0; }
[[ ${#changed[@]} -gt 0 ]] || { echo full; exit 0; }
for path in "${changed[@]}"; do
  case "$path" in
    docs/*|*.md) ;;
    *) echo "[ci.scope] code: $path" >&2; echo full; exit 0 ;;
  esac
  # A document some program, script or test reads (include_str!, a gate's fixture)
  # is code for this purpose.
  name="$(basename "$path")"
  if git grep -q -F -e "$name" -- . ':!docs' ':!*.md' 2>/dev/null; then
    echo "[ci.scope] read by code: $path" >&2; echo full; exit 0
  fi
done
echo "[ci.scope] documents only: ${#changed[@]} files" >&2
echo docs
