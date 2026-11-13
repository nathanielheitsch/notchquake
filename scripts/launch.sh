#!/bin/bash
# build + assemble bundle + kill stale + relaunch (AGENTS.md: always kill before
# relaunch or you test a stale binary).
#
# Provenance: the bundle is assembled by scripts/build-bundle.sh from the
# freshly built binary plus the tracked packaging/Info.plist, so this script can
# never test a stale gitignored bundle and never has to guess whether the plist
# is current. No signing here: nothing in this repo claims a Developer ID
# identity check, because no valid signing identity exists locally yet
# (docs/project-state.md). An unsigned/ad-hoc binary runs locally; it is not
# distributable.
set -eu
cd "$(dirname "$0")/.."

scripts/build-bundle.sh debug

# kill EVERY stale holder: this bundle plus a renamed test copy — a stray
# binary steals the exclusive Carbon hotkey (AGENTS.md lesson 53). Pattern
# chosen so pkill cannot match its own command line (lesson 56).
pkill -f "NotchQuake.app/Contents/MacOS/notchquake" || true
pkill -f "notchquake-rt" || true
sleep 1
rm -f /tmp/bnt-*.txt /tmp/nq*.log

# `open -n` can fail for reasons that have nothing to do with signatures — most
# often a regular FILE at NotchQuake.app (launchd spawn error 162/163, which is
# what the old "stale bundle signature" message was actually reporting).
# build-bundle.sh removes a non-directory there and re-seals the bundle, so this
# reports the real error instead of asserting a cause.
if ! open -n NotchQuake.app 2>/tmp/nq-open.err; then
  echo "open -n refused: $(head -3 /tmp/nq-open.err)" >&2
  echo "falling back to direct exec (unbundled process: no bundle Info.plist, hotkey still registers)" >&2
  ./NotchQuake.app/Contents/MacOS/notchquake >/tmp/nq-launch.log 2>&1 &
fi
sleep 3
pgrep -f "NotchQuake.app/Contents/MacOS/notchquake" | while read p; do echo "pid: $p"; done
