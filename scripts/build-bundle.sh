#!/bin/bash
# Assemble a NotchQuake.app bundle from source: fresh binary + tracked metadata
# template, sealed with an AD-HOC signature only.
#
#   scripts/build-bundle.sh            # debug build into ./NotchQuake.app
#   scripts/build-bundle.sh release    # release build into ./NotchQuake.app
#   NQ_APP=/tmp/nq/NotchQuake.app scripts/build-bundle.sh release
#
# What this fixes over the old launch.sh path:
#   * the bundle is ASSEMBLED here from the tracked template + a fresh binary, so
#     a stale gitignored bundle can never be the thing you test or ship;
#   * Info.plist is generated from packaging/Info.plist with the version taken
#     from Cargo.toml, so version/minimum-OS/copyright are never hand-maintained;
#   * the bundle path must be a directory (launchd refuses a regular file named
#     NotchQuake.app with spawn error 162/163 — that was the real cause of the
#     old "open -n refused (stale bundle signature?)" message);
#   * Resources (notices + licenses/) are sealed into the bundle, which the old
#     bundle never did — its signature sealed 0 files, so every check against it
#     was vacuous;
#   * no Developer ID claims. The only signature applied here is ad-hoc, and the
#     script says so. Developer ID signing and notarization are a separate,
#     credential-gated step (scripts/sign-notarize.sh, docs/release-runbook.md).
set -eu
cd "$(dirname "$0")/.."

PROFILE="${1:-debug}"
case "$PROFILE" in
  debug)   CARGO_PROFILE=debug ;;
  release) CARGO_PROFILE=release ;;
  *) echo "usage: scripts/build-bundle.sh [debug|release]" >&2; exit 2 ;;
esac

APP="${NQ_APP:-NotchQuake.app}"
TEMPLATE=packaging/Info.plist
# Single source of truth for the deployment target: it is what the binary is
# compiled with AND what LSMinimumSystemVersion states. A mismatch means a
# too-old Mac gets a "damaged app" refusal instead of a clean version error.
MINOS="${NQ_MIN_OS:-13.0}"
CARGO_TARGET="${NQ_CARGO_TARGET:-aarch64-apple-darwin}"

VERSION=$(python3 - <<'PY'
import tomllib
print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])
PY
)
# Deterministic build number: version + commit. Same commit -> same CFBundleVersion.
COMMIT=$(git rev-parse --short=8 HEAD 2>/dev/null || echo 00000000)
BUILD="${VERSION}+${COMMIT}"

# --- build -------------------------------------------------------------------
export MACOSX_DEPLOYMENT_TARGET="$MINOS"
# --target makes the arm64 target explicit rather than inherited from the host,
# so a CI runner cannot silently produce a different architecture.
cargo build --profile "$CARGO_PROFILE" --locked --target "$CARGO_TARGET"
# --target puts the artifact under target/<triple>/<profile>/, not target/<profile>/.
BIN=target/"$CARGO_TARGET"/"$CARGO_PROFILE"/notchquake
[ -f "$BIN" ] || { echo "build produced no binary at $BIN" >&2; exit 1; }
file "$BIN" | grep -q 'arm64' || { echo "$BIN is not arm64: $(file -c "$BIN")" >&2; exit 1; }
minos=$(otool -l "$BIN" | awk '/LC_BUILD_VERSION/{f=1} f && /minos/{print $2; exit}')
[ -n "$minos" ] || { echo "no LC_BUILD_VERSION minos in $BIN" >&2; exit 1; }
[ "$minos" = "$MINOS" ] || { echo "binary minos=$minos does not match LSMinimumSystemVersion=$MINOS" >&2; exit 1; }

# --- assemble the bundle -----------------------------------------------------
# Assemble FROM SCRATCH: remove whatever sits at $APP (a stale bundle, a partial
# bundle, or a regular file — the launchd spawn-162/163 case) so no member of an
# old bundle can survive into the new one. Overwriting in place left stale
# Contents/ entries behind whenever the template or Resources changed shape.
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
python3 - "$TEMPLATE" "$APP/Contents/Info.plist" "$VERSION" "$BUILD" "$MINOS" <<'PY'
import sys, pathlib
tpl, out, version, build, minos = sys.argv[1:6]
text = pathlib.Path(tpl).read_text()
for k, v in (("@VERSION@", version), ("@BUILD@", build), ("@MINOS@", minos)):
    if k not in text:
        sys.exit(f"{tpl} is missing placeholder {k}")
    text = text.replace(k, v)
if "@" in text:
    sys.exit(f"unsubstituted placeholder left in {out}")
pathlib.Path(out).write_text(text)
PY
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# The binary is copied fresh and made executable; a stale bundle binary is
# overwritten, never reused. `install` does copy+chmod in one step.
install -m 755 "$BIN" "$APP/Contents/MacOS/notchquake"

# Belt-and-braces: the rm -rf above already removed the whole bundle, but a stale
# Contents/_CodeSignature no longer matches the new binary and must never be
# reused — it is removed here too, then re-sealed below.
rm -rf "$APP/Contents/_CodeSignature"

# Third-party notices travel with the bundle: a distribution archive must carry
# them (NOTICE.md), and putting them in Resources means the shipping artifact is
# licence-complete before any signing step exists.
for f in NOTICE.md THIRD-PARTY-NOTICES.md LICENSE; do
  if [ -f "$f" ]; then install -m 644 "$f" "$APP/Contents/Resources/$f"; fi
done
if [ -d licenses ]; then
  rm -rf "$APP/Contents/Resources/licenses"
  cp -R licenses "$APP/Contents/Resources/licenses"
fi

# Seal the bundle with an AD-HOC signature after Resources are populated.
# Verified locally: a bundle with Contents/Resources and no _CodeSignature fails
# `codesign --verify --strict` with "code has no resources but signature indicates
# they must be present", because the linker's implicit ad-hoc signature seals the
# Mach-O only. Ad-hoc is a LOCAL signature: `spctl` rejects it, it cannot be
# notarized, and it is not distributable. Stage 2 replaces it with Developer ID +
# hardened runtime (scripts/sign-notarize.sh, docs/release-runbook.md).
codesign --force --sign - "$APP/Contents/MacOS/notchquake"
codesign --force --sign - "$APP"
codesign --verify --strict --verbose=1 "$APP"

# Report the signature honestly: ad-hoc, no team, no Developer ID authority.
sig=$(codesign -dvv "$APP" 2>&1 | grep -E '^(Signature|TeamIdentifier|CodeDirectory)' | tr '\n' ' ')
echo "bundle: $APP"
echo "  version  CFBundleShortVersionString=$VERSION  CFBundleVersion=$BUILD"
echo "  minos    LSMinimumSystemVersion=$MINOS (binary minos=$minos, target=$CARGO_TARGET)"
echo "  binary   $BIN -> $APP/Contents/MacOS/notchquake ($(shasum -a 256 "$APP/Contents/MacOS/notchquake" | cut -c1-16)…)"
echo "  signing  ad-hoc local signature only: $sig"
echo "           NOT distributable — Developer ID signing and notarization require"
echo "           credentials that do not exist yet (docs/release-runbook.md)."
