#!/bin/bash
# Deterministic distribution archive for NotchQuake.app.
#
#   scripts/package-artifact.sh [outdir]        # default outdir = dist/
#   NQ_APP=/tmp/nq/NotchQuake.app scripts/package-artifact.sh dist
#
# Emits NotchQuake-<version>.tar.gz (preferred: tar preserves the .app tree and
# permissions exactly), a normalized NotchQuake-<version>.zip for people who
# cannot open tar.gz, and SHA256SUMS. It refuses to finish unless BOTH the tar.gz
# and the zip are byte-identical across two archive runs, and it writes
# SHA256SUMS only AFTER those comparisons pass — a checksums file must never
# describe bytes the script failed to certify.
#
# Determinism rule: mtimes AND atimes pinned to 1980-01-01, uid/gid 0, no
# AppleDouble/.DS_Store entries, xattrs cleared, no embedded filename or
# timestamp in the gzip header, and zip entries written in sorted order with
# fixed modes. Byte identity comes from the tree, not from the clock, and it is
# PROVEN below by archiving the same staged tree twice and comparing sha256 for
# BOTH the tar.gz and the zip. The tar archive is not "sorted": bsdtar has no
# --sort option, so entry order is directory-traversal order — stable for one
# staged tree, which is exactly what the double-build check verifies. See the
# `--format ustar` note below for why that flag is load-bearing.
set -eu
cd "$(dirname "$0")/.."

OUT="${1:-dist}"
APP="${NQ_APP:-NotchQuake.app}"
[ -d "$APP" ] || { echo "no bundle at $APP — run scripts/build-bundle.sh release first" >&2; exit 1; }

VERSION=$(python3 - <<'PY'
import tomllib
print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])
PY
)

mkdir -p "$OUT"

# A staging tree with every mtime pinned. Copying (not moving) keeps the working
# bundle usable for a launch; the bundle is a build product either way.
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
# Mirror $APP into $STAGE/NotchQuake.app (the archive entry name is fixed, so a
# build into NQ_APP=/tmp/somewhere/else still ships the same paths).
find "$APP" -print0 | while IFS= read -r -d '' p; do
  dst="$STAGE/NotchQuake.app${p#"$APP"}"
  if [ -d "$p" ]; then mkdir -p "$dst"; else mkdir -p "$(dirname "$dst")"; cp -pR "$p" "$dst"; fi
done
# Pin mtimes across the whole staged tree, and drop Apple xattrs/quarantine flags
# so a re-run does not inherit metadata from the machine that built it. atime is
# pinned too (see the --format note below).
find "$STAGE" -exec touch -t 198001010000 {} +
find "$STAGE" -exec touch -a -t 198001010000 {} +
xattr -cr "$STAGE/NotchQuake.app" 2>/dev/null || true

ARCHIVE="$OUT/NotchQuake-$VERSION.tar.gz"
# COPYFILE_DISABLE keeps macOS from injecting ._ AppleDouble entries; --exclude
# catches a stray .DS_Store. gzip -n drops the embedded filename+mtime header.
#
# `--format ustar` is deliberate and VERIFIED: bsdtar's default pax format writes
# atime/ctime as extended records, so reading a file between two archives (which
# happens whenever anything inspects the tree) changes the bytes. Measured: with
# pax, an atime bump produced a different sha256; with ustar it did not. ustar's
# 100-char path limit is not a constraint here (longest entry is ~55 chars).
COPYFILE_DISABLE=1 bsdtar --uid 0 --gid 0 --format ustar -cf - \
  --exclude '.DS_Store' --exclude '._*' \
  -C "$STAGE" NotchQuake.app | gzip -n -9 > "$ARCHIVE"

# Also emit a .zip for people who cannot open tar.gz, normalized the same way
# (entries written in sorted order, epoch mtimes, fixed modes, no extra fields,
# no AppleDouble). It carries no directory entries, which unzip/Archive Utility
# handle fine; the executable bit is preserved through external_attr with
# create_system=3 (unix). Wrapped in a function so the determinism check below
# can build it a second time from the SAME staged tree.
ZIP="$OUT/NotchQuake-$VERSION.zip"
make_zip() {
  python3 - "$STAGE/NotchQuake.app" "$1" <<'PY'
import os, sys, zipfile
src, out = sys.argv[1], sys.argv[2]
items = []
for dirpath, dirnames, filenames in os.walk(src):
    dirnames.sort()
    for name in sorted(filenames):
        p = os.path.join(dirpath, name)
        items.append(os.path.relpath(p, os.path.dirname(src)))
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for rel in sorted(items):
        p = os.path.join(os.path.dirname(src), rel)
        mode = 0o100755 if os.access(p, os.X_OK) else 0o100644
        info = zipfile.ZipInfo(rel, date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.create_system = 3
        info.external_attr = mode << 16
        with open(p, "rb") as fh:
            z.writestr(info, fh.read())
PY
}
make_zip "$ZIP"

# Notices are already inside the bundle's Contents/Resources (build-bundle.sh),
# and the release workflow uploads the standalone copies too, so this script does
# not duplicate them into a second archive.

# --- prove determinism, then (and only then) write checksums ------------------
# A second run of BOTH packers over the same staged tree must produce identical
# bytes. Anything else means the archive encodes machine state (clock, xattrs,
# traversal order, AppleDouble) and the Homebrew cask's fixed sha256 would be a
# lie, so the script exits non-zero BEFORE emitting SHA256SUMS.
SECOND=$(mktemp -d)
COPYFILE_DISABLE=1 bsdtar --uid 0 --gid 0 --format ustar -cf - \
  --exclude '.DS_Store' --exclude '._*' \
  -C "$STAGE" NotchQuake.app | gzip -n -9 > "$SECOND/a.tar.gz"
make_zip "$SECOND/a.zip"

a=$(shasum -a 256 "$ARCHIVE" | cut -d' ' -f1)
b=$(shasum -a 256 "$SECOND/a.tar.gz" | cut -d' ' -f1)
[ "$a" = "$b" ] || { echo "tar.gz is NOT reproducible ($a vs $b)" >&2; rm -rf "$SECOND"; exit 1; }
z1=$(shasum -a 256 "$ZIP" | cut -d' ' -f1)
z2=$(shasum -a 256 "$SECOND/a.zip" | cut -d' ' -f1)
[ "$z1" = "$z2" ] || { echo "zip is NOT reproducible ($z1 vs $z2)" >&2; rm -rf "$SECOND"; exit 1; }
rm -rf "$SECOND"

# Only now are the bytes certified, so only now is SHA256SUMS written.
( cd "$OUT" && shasum -a 256 "$(basename "$ARCHIVE")" "$(basename "$ZIP")" > SHA256SUMS )

echo "archive  $ARCHIVE  (reproducible: two runs, same bytes)"
echo "zip      $ZIP      (reproducible: two runs, same bytes)"
echo "sha256   $a  $z1"
cat "$OUT/SHA256SUMS"
