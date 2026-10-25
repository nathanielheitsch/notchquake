#!/bin/bash
# One-time setup: import your Apple Developer ID Application certificate
# ("Barnstorm LLC") into the login keychain so codesign can use it everywhere
# in this pipeline.
#
# HOW TO OBTAIN THE CERT (do this once, then never again):
#   1. Log into developer.apple.com with the "Barnstorm LLC" account.
#   2. Account > Memberships: confirm you have the "App Development with
#      Distribution" entitlement (that is what unlocks Developer ID cert).
#   3. Account > Certificates, Credentials, Keys > Developer ID tab. If empty,
#      click "Create a Certificate" (no special app registration needed for
#      CLI-distributed apps). You'll set a passphrase and download a .p12.
#   4. Save it as .signing/BarnstormLLC.p12 in this repo. chmod 600.
#
# Cleanup note: if you ran scripts/make-signing-cert.sh before, the login
# keychain already holds a self-signed "Barnstorm LLC" cert and the security
# CLI has no delete verb — open Keychain.app, search "Barnstorm LLC", delete
# the cert item and its private key. The verify gate below will also catch a
# collision (self-signed → Signature=adhoc; Developer → Signature=Developer).
set -eu
cd "$(dirname "$0")/.."
P12=.signing/BarnstormLLC.p12
[ -f "$P12" ] || { echo "place your Developer ID .p12 at $P12 first (instructions above)"; exit 1; }
umask 077
read -r -p "p12 passphrase (leave blank if none): " PASS
if [ -z "$PASS" ]; then
  security import "$P12" -k ~/Library/Keychains/login.keychain-db
else
  security import "$P12" -k ~/Library/Keychains/login.keychain-db -P "$PASS"
fi
# empirical gate — sign a throwaway build copy and prove we got Developer, not adhoc
codesign -s "Barnstorm LLC" -f target/debug/notchquake
codesign -dv target/debug/notchquake 2>&1 | grep -E "^Signature=Developer" \
  || { echo "NOT Developer-signed (likely the self-signed collision or wrong identity). Delete the 'Barnstorm LLC' cert in Keychain.app and re-import."; exit 1; }
codesign -dv target/debug/notchquake 2>&1 | grep -E "^Owner" | head -1
echo "OK — Developer ID identity 'Barnstorm LLC' is live in your login keychain."
echo
echo "Now set the CI Secrets so release.yml can sign on GitHub:"
echo "  gh secret set DEVID_P12_B64 < <(base64 -A $P12)"
echo "  gh secret set DEVID_P12_PASS  # prompts; the passphrase you gave when creating the cert"
echo
echo "Optional notarization (Gatekeeper trust without a Smart-X click; do this"
echo "once per signed release you distribute):"
echo "  1. sign a throwaway copy locally with Developer ID (this script proves you can)."
echo "  2. open https://developer.apple.com/submit-binary/ and upload"
echo "     target/debug/notchquake (or the release build). Apple returns a"
echo "     notarized copy — you staple it with \`stapler staple <file>\` and"
echo "     ship that file; \`stapler validate\` proves the ticket is attached."
