#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FENCE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/sexos-browser-fence.XXXXXX")"
trap 'rm -rf "$FENCE_DIR"' EXIT
python3 - "$ROOT_DIR/apps/kaleidoscope/src/main.rs" "$FENCE_DIR" <<'PY'
from pathlib import Path
import re,sys
source=Path(sys.argv[1]).read_text();parts=[]
for name in ('fence_asset','update_asset_peer'):
 found=re.findall(r'^    fn '+name+r'\b.*?^    }',source,re.M|re.S)
 assert len(found)==1,name
 parts.extend(found)
actual='impl App {\n'+'\n'.join(parts)+'\n}'
assert 'self.update_asset_peer(peer_view,peer_ticket);'in source
assert 'if !self.asset_fenced{\n                match peer_view{'in actual
Path(sys.argv[2],'actual.rs').write_text(actual)
# Exact negative control: remove only the real peer-adoption fence guard.
Path(sys.argv[2],'unguarded.rs').write_text(actual.replace('if !self.asset_fenced{\n                match peer_view{','if true{\n                match peer_view{',1))
PY
BROWSER_ASSET_FENCE="$FENCE_DIR/actual.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/browser_asset_fence.rs" -o "$FENCE_DIR/tests"
"$FENCE_DIR/tests"
BROWSER_ASSET_FENCE="$FENCE_DIR/unguarded.rs" rustc --edition=2021 --test "$ROOT_DIR/tests/browser_asset_fence.rs" -o "$FENCE_DIR/unguarded-tests"
NEGATIVE_LOG="${BROWSER_ASSET_FENCE_NEGATIVE_LOG:-$FENCE_DIR/negative.log}"
if "$FENCE_DIR/unguarded-tests" fenced_ > "$NEGATIVE_LOG" 2>&1; then
    echo '[browser.asset.fence.negative] FAIL removed production guard was accepted'
    exit 1
fi
rg -q 'fenced_q_cancel_does_not_claim_display_clear .*FAILED' "$NEGATIVE_LOG"
rg -q 'fenced_late_verified_never_adopts_or_reallocates_image_storage .*FAILED' "$NEGATIVE_LOG"
echo '[browser.asset.fence.negative] PASS exact_removed_guard_rejected_by_both_actual_method_regressions'
echo '[browser.asset.fence.host] PASS scope=actual_Browser_fence_and_peer_transition_with_host_custody_callbacks_and_actual_bitmap_outbox_not_native_authority_or_pixels'
