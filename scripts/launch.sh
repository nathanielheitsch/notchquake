#!/bin/bash
# Build + relaunch the BNT.app prototype, killing the stale instance first.
cd "$(dirname "$0")/.." || exit 1
cargo build 2>&1 | grep -E "^error" -A6 | head -20
[ -f target/debug/rust-notch-terminal ] || { echo "BUILD FAILED"; exit 1; }
cp target/debug/rust-notch-terminal BNT.app/Contents/MacOS/BNT
# Kill every process holding the toggle hotkey, not just the bundle path. A
# renamed copy (e.g. /tmp/bnt-rt from the cross-process persistence test) or a
# stale debug build registers the SAME exclusive Carbon chord and silently
# steals Option+Space, leaving the real app unreachable by hotkey.
pkill -f "BNT.app/Contents/MacOS/BNT"; pkill -f "bnt-rt"; sleep 1
rm -f /tmp/bnt-*.txt
open -n BNT.app
sleep 3
echo "pid: $(pgrep -f 'BNT.app/Contents/MacOS/BNT')"
