#!/bin/bash
# build + kill stale + relaunch (AGENTS.md: always kill before relaunch or you
# test a stale binary).
set -eu
cd "$(dirname "$0")/.."
cargo build
[ -f target/debug/notchquake ] || { echo "BUILD FAILED"; exit 1; }

# sign with the Barnstorm LLC identity (scripts/setup-developer-id.sh imports your Developer ID
# once); if the identity is missing the app still runs adhoc but says so loudly.
if codesign -s "Barnstorm LLC" -f NotchQuake.app/Contents/MacOS/notchquake 2>/dev/null; then
  codesign -dv NotchQuake.app/Contents/MacOS/notchquake 2>&1 | grep -E "^Signature|^Owner" | head -2
else
  echo "WARN: no 'Barnstorm LLC' signing identity — no Developer ID in keychain — run scripts/setup-developer-id.sh; adhoc (local runs only) link-signed"
  codesign -f -s - NotchQuake.app/Contents/MacOS/notchquake 2>/dev/null || true
fi

# kill EVERY stale holder: this bundle plus a renamed test copy — a stray
# binary steals the exclusive Carbon hotkey (AGENTS.md lesson 53). Pattern
# chosen so pkill cannot match its own command line (lesson 56).
pkill -f "NotchQuake.app/Contents/MacOS/notchquake" || true
pkill -f "notchquake-rt" || true
sleep 1
rm -f /tmp/bnt-*.txt /tmp/nq*.log
open -n NotchQuake.app || { echo "open -n refused (stale bundle signature?) — falling back to direct exec"; ./NotchQuake.app/Contents/MacOS/notchquake >/tmp/nq-launch.log 2>&1 & }
sleep 3
pgrep -f "NotchQuake.app/Contents/MacOS/notchquake" | while read p; do echo "pid: $p"; done
