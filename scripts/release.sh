#!/bin/bash
# Release PREP for NotchQuake: gates -> release bundle -> deterministic archive
# -> checksums -> signed tag. Publication (Developer ID signing, notarization,
# GitHub Release upload) is NOT performed here yet, because signing credentials
# do not exist (docs/project-state.md). This script refuses to publish and says
# exactly which stage it stopped at.
#
# Usage:
#   scripts/release.sh v0.2.0          # gates + bundle + archive + checksums, no tag (default)
#   scripts/release.sh v0.2.0 --tag    # the above, plus a SIGNED annotated tag (no push)
#
# Stage 2 (Developer ID signing + notarization, requires credentials) lives in
# scripts/sign-notarize.sh and is deliberately NOT invoked from here.
set -eu
cd "$(dirname "$0")/.."

TAG="${1:?usage: scripts/release.sh v0.2.0 [--tag]}"
MODE="${2:-gates}"
case "$TAG" in
  v[0-9]*.*) ;;
  *) echo "tag must look like v0.2.0 (got $TAG)" >&2; exit 2 ;;
esac
case "$MODE" in
  gates|--tag) ;;
  *) echo "usage: scripts/release.sh <tag> [--tag]" >&2; exit 2 ;;
esac

VERSION="${TAG#v}"
CARGO_VERSION=$(python3 - <<'PY'
import tomllib
print(tomllib.load(open("Cargo.toml", "rb"))["package"]["version"])
PY
)
[ "$VERSION" = "$CARGO_VERSION" ] || {
  echo "tag version $VERSION does not match Cargo.toml version $CARGO_VERSION — bump Cargo.toml first" >&2
  exit 1
}

# --- gates -------------------------------------------------------------------
# cargo-deny is optional (not on every machine); the notice check is mandatory
# and offline.
cargo fmt --all -- --check
cargo build --locked
cargo test --locked
scripts/gen-notices.sh check
if command -v cargo-deny >/dev/null 2>&1; then cargo deny check; else echo "note: cargo-deny not installed, skipping license/advisory check"; fi

# --- CHANGELOG section --------------------------------------------------------
# The release workflow builds its notes from `## <tag>`, so a release without a
# matching CHANGELOG section would ship with "See CHANGELOG.md" as the notes.
# Warn rather than fail: the section can be added in the same commit as the bump.
if ! grep -q "^## $TAG" CHANGELOG.md 2>/dev/null; then
  echo "warning: CHANGELOG.md has no '## $TAG' section; the release notes would fall back to a stub" >&2
fi

# --- bundle + archive --------------------------------------------------------
scripts/build-bundle.sh release
NQ_APP=NotchQuake.app scripts/package-artifact.sh dist

# The archive must carry the licence artefacts, or the release is not
# distributable. Verified against the archive itself, not the working tree.
for f in NOTICE.md THIRD-PARTY-NOTICES.md LICENSE; do
  bsdtar -tf "dist/NotchQuake-$VERSION.tar.gz" "NotchQuake.app/Contents/Resources/$f" >/dev/null \
    || { echo "release archive is missing Contents/Resources/$f" >&2; exit 1; }
done
[ -f "dist/NotchQuake-$VERSION.tar.gz" ] || {
  echo "release archive was not created" >&2
  exit 1
}
# licenses/ is an obligation, not a courtesy: notices without the pinned licence
# texts do not satisfy attribution. The old check globbed 'licenses/*', which
# bsdtar happily satisfies with the DIRECTORY ENTRY ALONE — an empty licenses/
# directory passed. That empty-directory acceptance is the real hole: under
# `set -eu` a MISSING licenses/ directory already failed, but only with the
# command's own error rather than this obligation message; an empty one passed
# cleanly. Require at least one real FILE below licenses/ and say which obligation
# was unmet.
lic_files=$(bsdtar -tf "dist/NotchQuake-$VERSION.tar.gz" 2>/dev/null \
  | grep -E '^NotchQuake\.app/Contents/Resources/licenses/.+$' \
  | grep -v '/$' \
  | head -5) || lic_files=""
[ -n "$lic_files" ] || {
  echo "release archive carries no licence-text FILE under Contents/Resources/licenses/" >&2
  echo "(notices without the pinned licence texts do not satisfy attribution)" >&2
  exit 1
}

# --- signature status (informational, no Developer ID signing) ---------------
# Honest check, not the old `grep -qE "^Signature=Developer"`, which an ad-hoc or
# self-signed identity can also satisfy. What distinguishes a distributable
# signature is the Authority chain (`Authority=Developer ID Application: …`) and
# TeamIdentifier, plus `security find-identity -v -p codesigning`.
if codesign -dvv NotchQuake.app 2>&1 | grep -qE '^Authority=Developer ID Application:'; then
  echo "bundle carries a Developer ID Application authority — stage 2 (notarize + publish) applies"
else
  echo "bundle carries only the local ad-hoc signature from build-bundle.sh: NOT distributable."
  echo "Stage 2 (Developer ID sign + notarize + staple) is blocked until Barnstorm LLC credentials exist."
fi

if [ "$MODE" != "--tag" ]; then
  echo "no tag created (pass --tag to create a signed annotated tag)"
  echo
  echo "STOPPED BEFORE PUBLICATION. Remaining stages (docs/release-runbook.md):"
  echo "  1. scripts/release.sh $TAG --tag   # signed annotated tag, then push it deliberately"
  echo "  2. scripts/sign-notarize.sh        # Developer ID sign, notarize, staple, verify"
  echo "  3. upload dist/ artifacts to a GitHub Release with SHA256SUMS"
  echo "  4. clean-Mac Gatekeeper validation of the downloaded artifact"
  exit 0
fi
# Signed annotated tag ONLY. No unsigned fallback: an unsigned tag cannot be
# attributed to a key, which is the whole point of tagging a release.
# A tag must point at a COMMITTED state, otherwise the release ships bytes that
# exist nowhere in history.
if [ -n "$(git status --porcelain)" ]; then
  echo "working tree is dirty — commit the release changes before tagging" >&2
  git status --short >&2
  exit 1
fi
if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
  echo "tag $TAG already exists — not touching it (delete it deliberately if you meant to re-cut)" >&2
  exit 1
fi
git tag -s "$TAG" -m "NotchQuake $TAG"
# Verify what was actually created. A tag object's signature is a `-----BEGIN PGP
# SIGNATURE-----` block after the message, NOT a `gpgsig` header (that form is
# only how signed *commit* objects store it).
git cat-file tag "$TAG" | grep -qE '^-----BEGIN (PGP|SSH) SIGNATURE-----' \
  || { echo "$TAG has no signature block — signing was silently skipped; delete it and retry" >&2; exit 1; }
echo "signed tag $TAG created locally. Push it deliberately: git push origin refs/tags/$TAG"
echo
echo "STOPPED BEFORE PUBLICATION. Remaining stages (docs/release-runbook.md):"
echo "  1. scripts/sign-notarize.sh        # Developer ID sign, notarize, staple, verify"
echo "  2. upload dist/ artifacts to a GitHub Release with SHA256SUMS"
echo "  3. clean-Mac Gatekeeper validation of the downloaded artifact"
