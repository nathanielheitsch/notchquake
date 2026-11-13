#!/bin/bash
# LOCAL, ONE-TIME developer setup for Developer ID signing. Requires real Apple
# Developer ID credentials, which this project does not have yet
# (docs/project-state.md). Nothing here creates a signing identity: a
# self-signed certificate is NOT a Developer ID certificate and no command can
# conjure one. If you do not have the certificate, stop and read
# docs/release-runbook.md instead.
#
# What this does:
#   1. import a Developer ID Application .p12 into the LOCAL login keychain, and
#   2. prove the imported identity really is a valid Developer ID chain —
#      `security find-identity -v -p codesigning` plus the Authority chain and
#      TeamIdentifier from `codesign -dvv`, NOT the old `grep "^Signature=Developer"`
#      which a self-signed identity can also satisfy.
#
# CI does NOT use this script. CI builds an EPHEMERAL keychain per job and deletes
# it (see .github/workflows/release.yml and docs/release-runbook.md). Do not put a
# .p12 in this repo: .signing/ is gitignored for that reason.
set -eu
cd "$(dirname "$0")/.."

P12="${1:-.signing/BarnstormLLC.p12}"
IDENTITY="${NQ_SIGNING_IDENTITY:-Developer ID Application: Barnstorm LLC}"

[ -f "$P12" ] || {
  cat >&2 <<EOF
No Developer ID .p12 at $P12.

To obtain one (once, from an Apple Developer account with the "App Development
with Distribution" membership capability):
  1. developer.apple.com -> Account -> Certificates, IDs & Profiles.
  2. Create a Developer ID Application certificate (CSR from Keychain Access,
     or a customer code-signing request); install it in your login keychain so
     its private key lives next to it.
  3. Export that identity + private key as a password-protected .p12 and place
     it at $P12 (chmod 600). It stays local and gitignored; CI gets its own copy
     via the DEVID_P12_B64 secret.
There is no shortcut and no substitute: an ad-hoc or self-signed identity cannot
be notarized and will not pass Gatekeeper on someone else's Mac.
EOF
  exit 1
}

umask 077
read -r -p "p12 passphrase: " PASS
[ -n "$PASS" ] || { echo "a Developer ID .p12 export always carries a passphrase; refusing a blank import" >&2; exit 1; }
security import "$P12" -k ~/Library/Keychains/login.keychain-db -P "$PASS" -T /usr/bin/codesign

# --- identity gate -----------------------------------------------------------
# find-identity -v already validates the trust chain; a self-signed or revoked
# cert does not appear as a valid codesigning identity.
line=$(security find-identity -v -p codesigning | grep -F "$IDENTITY" | head -1)
[ -n "$line" ] || {
  echo "no valid codesigning identity matching '$IDENTITY'. Check:" >&2
  security find-identity -v -p codesigning >&2
  # Correction: the security CLI DOES have delete verbs — `delete-certificate`
  # (-c name | -Z hash [-t]) and `delete-identity`. Keychain.app works too, but
  # the CLI is not the dead end the earlier comment claimed. Deleting an
  # identity also needs the keychain that holds it unlocked.
  echo "To remove a stale self-signed identity from an earlier experiment:" >&2
  echo "  security delete-certificate -c '<common name>' ~/Library/Keychains/login.keychain-db" >&2
  echo "  security delete-identity  -c '<common name>' ~/Library/Keychains/login.keychain-db" >&2
  exit 1
}
SHA=$(printf '%s' "$line" | awk '{print $2}')
echo "valid identity: $line"

# Prove the chain and team, which is what notarization actually keys on.
TMP=$(mktemp -d); trap 'rm -rf "$TMP"' EXIT
cp packaging/Info.plist "$TMP/Info.plist"   # any file works; we sign a throwaway copy
codesign --force --sign "$SHA" --timestamp=none "$TMP/Info.plist" >/dev/null 2>&1 || true
codesign -dvv "$TMP/Info.plist" 2>&1 | grep -E '^(Authority|TeamIdentifier|Signature)' | sed 's/^/  /' || true
codesign -dvv "$TMP/Info.plist" 2>&1 | grep -q 'Authority=Developer ID Application' || {
  echo "Authority chain is not 'Developer ID Application' — this is not a distributable identity." >&2
  exit 1
}

echo
echo "OK — a valid Developer ID identity is in your LOCAL login keychain."
echo "Next: docs/release-runbook.md (stage 2) for the CI secrets and the"
echo "sign -> notarize -> staple -> verify -> publish order."
