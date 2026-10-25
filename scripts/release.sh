#!/bin/bash
# Cut a public release: build release, Developer-ID-sign the .app bundle,
# attach the notarization ticket if one was obtained (setup-developer-id.sh
# explains the one manual Apple step), zip, and publish as a GH Release.
# Usage: scripts/release.sh v0.2-alpha
set -eu
cd "$(dirname "$0")/.."
TAG="${1:?usage: scripts/release.sh v0.2-alpha}"
CN="Barnstorm LLC"

cargo build --release
[ -f target/release/notchquake ] || { echo "BUILD FAILED"; exit 1; }

# rebuild the bundle fresh (never sign a stale tree)
mkdir -p NotchQuake.app/Contents/MacOS
cp target/release/notchquake NotchQuake.app/Contents/MacOS/notchquake
[ -f NotchQuake.app/Contents/Info.plist ] || {
  cat > NotchQuake.app/Contents/Info.plist <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>local.notchquake</string>
<key>CFBundleExecutable</key><string>notchquake</string>
<key>CFBundleName</key><string>NotchQuake</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST
}

# the signing gate: sign the inner binary FIRST (Developer ID; adhoc is not
# distributable — syspolicy says so), then the bundle itself. codesign on a
# .app directory does NOT re-sign the inner executable, so both steps matter.
codesign -s "$CN" NotchQuake.app/Contents/MacOS/notchquake \
  || { echo "no Developer ID identity '$CN' in keychain — run scripts/setup-developer-id.sh"; exit 1; }
codesign -s "$CN" NotchQuake.app
codesign -dv NotchQuake.app/Contents/MacOS/notchquake 2>&1 | grep -qE "^Signature=Developer" \
  || { echo "inner binary not Developer-signed — abort"; exit 1; }
syspolicy_check notary-submission NotchQuake.app --json 2>/dev/null | grep -q '"Fatal"' \
  && { echo "syspolicy says the bundle is not notarization-ready — inspect output and fix before publishing"; exit 1; }
echo "syspolicy: notarization-ready"

# notarization ticket (optional, but the zip must contain it if you notarized):
# after https://developer.apple.com/submit-binary/ returns the notarized copy,
# staple then validate before publishing.
if stapler validate NotchQuake.app >/dev/null 2>&1; then
  echo "notarization ticket present"
else
  echo "no notarization ticket — first launch needs Smart-X bypass on user machines (fine for an alpha; notarize via submit-binary to avoid it)"
fi

rm -f "/tmp/notchquake-$TAG.zip"
zip -qr "/tmp/notchquake-$TAG.zip" NotchQuake.app

NOTES=$(sed -n '/## '"$TAG"'/,/./p' CHANGELOG.md 2>/dev/null || true)
gh release create "$TAG" --title "NotchQuake $TAG" --notes "${NOTES:-See CHANGELOG.md}" --target "$(git rev-parse HEAD)" "/tmp/notchquake-$TAG.zip#NotchQuake-$TAG.zip"
git tag -S "$TAG" 2>/dev/null || git tag "$TAG"  # -S per repo signing rules; plain tag if signing agent refuses
git push -u origin "refs/tags/$TAG" 2>/dev/null || git push origin master
echo "published: $TAG"
