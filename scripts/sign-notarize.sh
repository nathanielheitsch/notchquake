#!/bin/bash
# STAGE 2: Developer ID sign -> notarize -> staple -> verify -> re-archive.
#
# NOT VERIFIED END TO END, BY DESIGN: this repo has no Developer ID certificate
# and no notary credentials, so nobody has run this successfully (see
# docs/project-state.md). It is written to be correct and to refuse loudly when
# its inputs are missing. Do not treat its existence as proof that signing works.
#
# Inputs (docs/release-runbook.md explains how to create each one):
#   NQ_P12        path to the Developer ID Application .p12 (optional if the
#                 machine's login keychain already holds the identity)
#   NQ_P12_PASS   passphrase for that .p12
#   NQ_NOTARY_API_KEY   notarytool API key: either a .p8 path or its contents
#   NQ_NOTARY_KEY_ID    Key ID from App Store Connect -> Integrations -> Keys
#   NQ_NOTARY_ISSUER    Issuer ID UUID
#
# It never echoes a secret, writes the API key to a private temp file only when
# it was given as contents, and always deletes both keychain and temp file
# (trap cleanup EXIT). The notarization UPLOAD zip never lands in $OUT either: it
# is staged under $RUNNER_TEMP (or a private mktemp dir locally) and removed by
# the same trap, so a failed notarization cannot leave a stray zip in dist/ for
# the BUILD job's artifact glob (dist/*.tar.gz, dist/*.zip, dist/SHA256SUMS) or
# for the publish job's `gh release create` to pick up.
set -eu
cd "$(dirname "$0")/.."

APP="${NQ_APP:-NotchQuake.app}"
IDENTITY="${NQ_SIGNING_IDENTITY:-Developer ID Application: Barnstorm LLC}"
OUT="${NQ_OUT:-dist}"

[ -d "$APP" ] || { echo "no bundle at $APP — run scripts/build-bundle.sh release first" >&2; exit 1; }

EPHEMERAL=""      # ephemeral keychain path, if we created one
OLD_KEYCHAINS=""  # user keychain list to restore on exit
KEYFILE=""        # temp .p8 when the API key was supplied as contents
STAGE=""          # private staging dir: bundle copy + notarization upload zip
cleanup() {
  [ -n "$KEYFILE" ] && rm -f "$KEYFILE"
  [ -n "$STAGE" ] && rm -rf "$STAGE"
  if [ -n "$EPHEMERAL" ]; then
    # Restore the previous keychain search list BEFORE deleting the file, so the
    # user's login keychain is not left pointing at a dead path. Unquoted on
    # purpose: it is a whitespace-separated list of paths.
    [ -n "$OLD_KEYCHAINS" ] && security list-keychains -d user -s $OLD_KEYCHAINS
    security delete-keychain "$EPHEMERAL" 2>/dev/null || true
    rm -rf "$(dirname "$EPHEMERAL")"
  fi
}
trap cleanup EXIT

# --- identity ---------------------------------------------------------------
# Prefer an ephemeral keychain when a .p12 is supplied. Never import into the
# login keychain from here — that is a one-time developer-machine action
# (scripts/setup-developer-id.sh).
if [ -n "${NQ_P12:-}" ]; then
  [ -f "$NQ_P12" ] || { echo "NQ_P12=$NQ_P12 does not exist" >&2; exit 1; }
  [ -n "${NQ_P12_PASS:-}" ] || { echo "NQ_P12_PASS is required when NQ_P12 is set" >&2; exit 1; }
  DIR=$(mktemp -d)
  EPHEMERAL="$DIR/signing.keychaindb"
  KCPASS=$(uuidgen)
  security create-keychain -p "$KCPASS" "$EPHEMERAL"
  security set-keychain-settings -lut 21600 "$EPHEMERAL"
  security unlock-keychain -p "$KCPASS" "$EPHEMERAL"
  # -A is deliberately ABSENT: security documents it as "Allow any application
  # to access the imported key without warning (insecure, not recommended!)".
  # -T /usr/bin/codesign is the scoped grant, and set-key-partition-list below
  # is what actually removes the ACL prompt a CI runner cannot answer.
  security import "$NQ_P12" -k "$EPHEMERAL" -P "$NQ_P12_PASS" -T /usr/bin/codesign
  OLD_KEYCHAINS=$(security list-keychains -d user | tr -d '"' | tr '\n' ' ')
  security list-keychains -d user -s "$EPHEMERAL" $OLD_KEYCHAINS
  # Codesign must be allowed to use the key without a GUI prompt (CI has no TTY).
  # Note the real verb is `set-key-partition-list` (singular) and it needs the
  # keychain password; without this, codesign blocks on an interactive prompt.
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KCPASS" "$EPHEMERAL" >/dev/null 2>&1 || \
    echo "WARN: set-key-partition-list failed; codesign may prompt (impossible on a CI runner)" >&2
fi

SHA=$(security find-identity -v -p codesigning | grep -F "$IDENTITY" | head -1 | awk '{print $2}')
[ -n "$SHA" ] || {
  echo "no valid codesigning identity matching '$IDENTITY'." >&2
  echo "Stage 2 cannot proceed without Barnstorm LLC's Developer ID Application certificate." >&2
  security find-identity -v -p codesigning >&2 || true
  exit 1
}
echo "signing identity: $IDENTITY ($SHA)"

# --- sign --------------------------------------------------------------------
# Inner executable first, then the bundle. Hardened runtime + secure timestamp
# are prerequisites for notarization. NotchQuake needs no entitlements: the
# Carbon hotkey and the PTY require no entitlement and no sandbox. If a future
# feature ever trips a hardened-runtime violation, the notary log names it; fix
# the code rather than disabling protections with com.apple.security.cs.* — and
# re-verify that the hotkey still registers afterwards.
codesign --force --options runtime --timestamp --sign "$SHA" "$APP/Contents/MacOS/notchquake"
codesign --force --options runtime --timestamp --sign "$SHA" "$APP"

codesign --verify --strict --verbose=2 "$APP"
# The real identity check is the Authority chain, not `Signature=`: an ad-hoc or
# self-signed identity can satisfy a `Signature=` grep but has no
# "Developer ID Application" authority and cannot be notarized.
codesign -dvv "$APP" 2>&1 | grep -E '^(Authority=|TeamIdentifier=)' | sed 's/^/  /'
codesign -dvv "$APP" 2>&1 | grep -q 'Authority=Developer ID Application:' || {
  echo "authority chain is not 'Developer ID Application:' — not distributable" >&2
  exit 1
}
# `spctl` here is only a local pre-check: before the ticket is stapled, Gatekeeper
# has nothing to validate against, so a rejected result at this point is expected.
spctl -a -vv -t exec "$APP" 2>&1 | sed 's/^/  spctl: /' || true

# --- notarize ----------------------------------------------------------------
# `xcrun notarytool` is the supported CLI (the earlier "web portal only" claim
# was wrong). API key: App Store Connect -> Users and Access -> Integrations ->
# Keys -> Mobile Account Secrets, "Notary Service" permission.
[ -n "${NQ_NOTARY_KEY_ID:-}" ] && [ -n "${NQ_NOTARY_ISSUER:-}" ] || {
  echo "NQ_NOTARY_KEY_ID and NQ_NOTARY_ISSUER are required" >&2; exit 1
}
[ -n "${NQ_NOTARY_API_KEY:-}" ] || { echo "NQ_NOTARY_API_KEY missing — cannot notarize" >&2; exit 1; }
command -v jq >/dev/null 2>&1 || { echo "jq is required to read the notarytool JSON result" >&2; exit 1; }
command -v ditto >/dev/null 2>&1 || { echo "ditto is required to build the notarization upload" >&2; exit 1; }
command -v xcrun >/dev/null 2>&1 || { echo "xcrun (Xcode CLT) is required for notarytool/stapler" >&2; exit 1; }
if [ -f "$NQ_NOTARY_API_KEY" ]; then
  KEY="$NQ_NOTARY_API_KEY"
else
  KEYFILE=$(mktemp -t nq-notary-key)
  chmod 600 "$KEYFILE"
  printf '%s' "$NQ_NOTARY_API_KEY" > "$KEYFILE"
  KEY="$KEYFILE"
fi

# The upload zip is staged OUTSIDE $OUT on purpose: $OUT is dist/, the directory
# the build job uploads by glob (dist/*.tar.gz, dist/*.zip, dist/SHA256SUMS) and
# that the publish job reads for its named release assets.
# A failed notarization must leave nothing there, so this file lives under
# $RUNNER_TEMP on CI (else a private mktemp dir) and the EXIT trap removes it.
if [ -n "${RUNNER_TEMP:-}" ]; then
  STAGE="$RUNNER_TEMP/nq-notarize-stage"
  mkdir -p "$STAGE"
else
  STAGE=$(mktemp -d)
fi
cp -R "$APP" "$STAGE/"
# --keepParent is required: notarization expects the .app inside a zip with its
# own directory entry, and ditto preserves the bundle's symlinks/xattrs.
ZIP="$STAGE/notarize-input.zip"
ditto -c -k --keepParent "$STAGE/$(basename "$APP")" "$ZIP"
mkdir -p "$OUT"
SUBMIT_JSON=$(xcrun notarytool submit "$ZIP" \
  --key "$KEY" --key-id "$NQ_NOTARY_KEY_ID" --issuer "$NQ_NOTARY_ISSUER" \
  --wait --json)
STATUS=$(printf '%s' "$SUBMIT_JSON" | jq -r '.status')
echo "notary status: $STATUS"
[ "$STATUS" = "Accepted" ] || {
  printf '%s' "$SUBMIT_JSON" | jq -r '.logFileUrl // empty' | sed 's/^/  log: /'
  echo "notarization was not Accepted — do not staple, do not publish" >&2
  exit 1
}

xcrun stapler staple "$APP"
xcrun stapler validate "$APP"
# The upload zip is not part of the artifact set; drop it here as well as in the
# trap, so a successful run leaves $OUT holding only the archives.
rm -f "$ZIP"

# --- final artifact ----------------------------------------------------------
# Re-package AFTER stapling so the ticket is inside what users download.
scripts/package-artifact.sh "$OUT"
echo
echo "stage 2 complete. Remaining (docs/release-runbook.md stage 3):"
echo "  upload $OUT (tar.gz, zip, SHA256SUMS) to the GitHub Release"
echo "  validate on a clean Mac:  xcrun stapler validate NotchQuake.app && spctl -a -vv -t exec NotchQuake.app"
