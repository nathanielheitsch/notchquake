# NotchQuake — current project state

## Baseline

- Pre-release alpha from Barnstorm LLC; Cargo version `0.1.0`.
- Canonical branch: `main` (formerly `master`); all existing development history retained.
- Repository: https://github.com/nathanielheitsch/notchquake (public).
- macOS Apple Silicon only; shipped name `NotchQuake.app`, executable `notchquake`,
  bundle ID `local.notchquake`. MIT license for project source.

## Implemented

- Animated notch panel and Carbon global toggle hotkey with transactional registration
  and stale-event generation guards; new-tab routing is terminal-local.
- Live PTY sessions, dynamic tabs, terminal key routing, scrollback, paste sanitization,
  and byte-transparent OSC observation with zsh shell integration for tab metadata.
- Separate tabbed native settings window; persisted fonts, colors, geometry, cursor,
  hotkeys, notifications, and scrollback preferences.
- Settings proposal/validation/effective-value pipeline with coalesced resize/save effects.
- Strict CoreText monospace resolution, Reduce Motion support, main-menu integration,
  and app-wide click gating for native windows.
- Display-width canvas and live terminal/PTY resize when open dimensions change.
- Internal `BNT.` preference keys and `BNT_DEFAULTS_SUITE` remain intentional.
  The bundle-ID rename reset preferences once; version bumps do not reset them.

## Validation and remaining UI checks

Run `cargo fmt --all --check`, `cargo build --locked`, and `cargo test --locked`.
Tests must not spawn login-shell PTYs (see AGENTS.md). CI runs these checks on
`main` pushes and pull requests using a macOS ARM64 runner.

On the pushed `main` HEAD, formatting, `cargo build --locked`, and
`cargo test --locked` pass locally (**44 tests, 0 failures**). Earlier work also
reports live panel/tab/hotkey checks.

Real-user slider dragging, popup selection, widened-panel outside-click behavior,
all settings tabs, and shell-exit teardown still need a complete manual pass.
Use the throwaway suite when testing settings:
`BNT_DEFAULTS_SUITE=bnt-verify open -n NotchQuake.app`. An empty value refuses
the default domain instead of falling back; that is intentional.
The app bundle is ignored; `launch.sh` expects the bundle directory to exist and
is intended for the shipped checkout rather than a clean source-only checkout.

## Signing and releases — blocked, not verified

The previous signing documentation overclaimed completion. Do not use the
existing `scripts/setup-developer-id.sh` or `scripts/release.sh` as release-ready
automation. No release is cut by this branch consolidation.

Local inspection found zero valid code-signing identities. Xcode's notarization
CLI is available through `xcrun --find notarytool`; it is not web-only.

Required follow-up:

1. Obtain Barnstorm LLC's **Developer ID Application** certificate with its
   private key. Apple issues a certificate; export the installed identity and
   private key as a password-protected `.p12` from Keychain Access for CI.
2. Verify the full identity and Team ID, not `Signature=Developer` (the existing
   scripts' check is wrong). Use `security find-identity -v -p codesigning`,
   `codesign --verify --strict --verbose=2`, and inspect Authority/TeamIdentifier.
3. Sign with hardened runtime and secure timestamp; submit the signed app archive
   with `xcrun notarytool submit ... --wait`, require Accepted, then staple and
   validate the ticket on the app before creating the final distribution archive.
4. Add protected GitHub release credentials, an ephemeral CI keychain and cleanup;
   never print or commit certificates' private keys, passwords, or notary API keys.
5. Require successful gates, signed annotated tags (no unsigned fallback), and
   verified notarization before publishing a GitHub release. Test a downloaded
   artifact on a clean Mac; generate checksums, and bundle the dependency notices —
   `NOTICE.md`, `THIRD-PARTY-NOTICES.md`, `LICENSE`, and `licenses/` — in the release
   archive. The notice lane is DONE (see below); packaging them into the archive is the
   remaining step, and `scripts/gen-notices.sh check` must pass on the release commit.

The release workflow is temporarily manual/build-only to prevent accidental
unsigned publication. Fix its signing path before restoring tag-triggered releases.

## Audit limitations

Earlier history checks found no tracked agent-run artifacts and no obvious secret
patterns. That was a heuristic scan, not a comprehensive secret/license audit.
Author names/emails and signed-commit identities are public Git metadata. Complete
a dedicated history scanner before release.

## Dependency licences and notices (done 2026-11-12)

The dependency-license/notice review is complete; the history scanner above is not.

- `NOTICE.md` — short attribution summary.
- `THIRD-PARTY-NOTICES.md` — generated per-dependency audit: direct / transitive runtime /
  compile-time-only (proc-macro closure) / out-of-scope-for-this-target, each crate's SPDX
  expression, crates.io + repository links, the licence file each crate actually shipped, and
  the copyright lines as written.
- `licenses/` — the canonical texts plus pinned objc2 texts, with sha256 and the exact commit
  each came from in `licenses/pins.json`, and an `otool -L` snapshot of the Apple frameworks.
- `scripts/gen-notices.py` + `scripts/gen-notices.sh` (`gen` / `check` / `deny` / `linkage`) —
  offline and deterministic (no timestamps, sorted output); a release never fetches anything.
- `deny.toml` — cargo-deny allow-list (MIT, Apache-2.0, Apache-2.0 WITH LLVM-exception, Zlib,
  Unlicense, Unicode-3.0), crates.io-only sources, RustSec advisories.

Verified locally: `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`; the
only remaining warning is the expected duplicate `windows-sys` pair gated to non-shipped
Windows targets; regeneration is byte-identical; `scripts/gen-notices.sh linkage` matches the
release build. `Cargo.toml` gained `license = "MIT"`, `repository`, `description` so
`cargo deny` reports no `no-license-field` for the root crate (no dependency changes, so
`Cargo.lock` is untouched).

Open questions are recorded in `THIRD-PARTY-NOTICES.md` rather than answered there — chiefly
the objc2 family's Apple-SDK-derivation note and the fact that its published crates ship no
licence text (texts pinned from upstream instead). Not legal advice.
