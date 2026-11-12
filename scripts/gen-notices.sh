#!/usr/bin/env bash
# Third-party notice generator / checker for NotchQuake.
#
#   scripts/gen-notices.sh gen      regenerate THIRD-PARTY-NOTICES.md (default)
#   scripts/gen-notices.sh check    fail if the committed file is stale
#   scripts/gen-notices.sh deny     cargo-deny license/version/source/advisory check
#   scripts/gen-notices.sh linkage  re-verify licenses/apple-framework-linkage.txt
#   scripts/gen-notices.sh all      gen + check + deny
#
# Everything here is OFFLINE. The generator reads Cargo.lock, `cargo metadata
# --locked --offline`, the crate sources already in ~/.cargo/registry/src, and
# the committed texts in licenses/ (licenses/pins.json records where those came
# from). A release must never depend on GitHub or crates.io being reachable, so
# nothing in this script downloads. If a crate's source is missing locally it
# tells you to run `cargo fetch --locked` and stops.
set -eu
cd "$(dirname "$0")/.."
CMD="${1:-gen}"

need_registry() {
  if [ ! -d "$HOME/.cargo/registry/src" ]; then
    echo "no vendored crate sources in ~/.cargo/registry/src — run: cargo fetch --locked" >&2
    exit 1
  fi
}

case "$CMD" in
  gen)
    need_registry
    python3 scripts/gen-notices.py
    ;;

  check)
    need_registry
    python3 scripts/gen-notices.py --check
    ;;

  deny)
    if ! command -v cargo-deny >/dev/null 2>&1; then
      echo "cargo-deny not installed — run: cargo install cargo-deny" >&2
      exit 1
    fi
    cargo deny check
    ;;

  linkage)
    BIN="${NQ_LINKAGE_BIN:-target/release/notchquake}"
    [ -f "$BIN" ] || { echo "build first (cargo build --release --locked) or set NQ_LINKAGE_BIN" >&2; exit 1; }
    actual=$(otool -L "$BIN" | tail -n +2 | sed 's/ (compatibility.*//' | sed 's/^[[:space:]]*//' | sort)
    committed=$(grep -v '^#' licenses/apple-framework-linkage.txt | sed 's/^[[:space:]]*//' | grep -v '^$' | sort)
    if [ "$actual" != "$committed" ]; then
      echo "Apple framework linkage differs from licenses/apple-framework-linkage.txt:" >&2
      diff <(printf '%s\n' "$committed") <(printf '%s\n' "$actual") >&2 || true
      echo "update it with: otool -L target/release/notchquake | tail -n +2 | sed 's/ (compatibility.*//' | sed 's/^[[:space:]]*//' | sort" >&2
      exit 1
    fi
    echo "linkage ok: $(printf '%s\n' "$actual" | wc -l | tr -d ' ') Apple libraries match the committed snapshot"
    ;;

  all)
    "$0" gen
    "$0" check
    "$0" deny
    ;;

  *)
    echo "usage: scripts/gen-notices.sh [gen|check|deny|linkage|all]" >&2
    exit 2
    ;;
esac
