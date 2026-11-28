# SPDX-License-Identifier: GPL-3.0-only
# Sourced by the host gates that build sex-time with plain rustc. sex-time
# takes its Maya calendar math from dreamroot-core (github.com/xirtus/dreamroot),
# pinned by git rev in crates/sex-time/Cargo.toml, so those gates build that
# crate first:
#   dreamroot_core_rlib DIR [rustc flags...]   writes DIR/libdreamroot_core.rlib
# then pass `--extern dreamroot_core=DIR/libdreamroot_core.rlib` when building
# sex-time and `-L dependency=DIR` to everything that links sex-time.

dreamroot_core_src() {
  local root rev short dir
  root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  rev="$(sed -n 's/^dreamroot-core = .*rev = "\([0-9a-f]*\)".*/\1/p' "$root/crates/sex-time/Cargo.toml")"
  [ -n "$rev" ] || { echo "dreamroot-core rev not found in crates/sex-time/Cargo.toml" >&2; return 1; }
  short="${rev:0:7}"
  # Cargo's own checkout, when a cargo build already fetched it.
  for dir in "${CARGO_HOME:-$HOME/.cargo}"/git/checkouts/dreamroot-*/"$short"; do
    if [ -f "$dir/crates/dreamroot-core/src/lib.rs" ]; then echo "$dir"; return 0; fi
  done
  dir="${TMPDIR:-/tmp}/sexos-dreamroot-$rev"
  if [ ! -f "$dir/crates/dreamroot-core/src/lib.rs" ]; then
    rm -rf "$dir.tmp"
    git init -q "$dir.tmp" >&2
    git -C "$dir.tmp" fetch -q --depth 1 https://github.com/xirtus/dreamroot "$rev" >&2
    git -C "$dir.tmp" checkout -q FETCH_HEAD >&2
    rm -rf "$dir" && mv "$dir.tmp" "$dir"
  fi
  echo "$dir"
}

dreamroot_core_rlib() {
  local out="$1" src
  shift
  src="$(dreamroot_core_src)" || return 1
  mkdir -p "$out"
  rustc --edition=2021 --crate-type rlib --crate-name dreamroot_core "$@" \
    "$src/crates/dreamroot-core/src/lib.rs" -o "$out/libdreamroot_core.rlib"
}
