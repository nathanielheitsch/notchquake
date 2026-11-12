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
Use a throwaway `BNT_DEFAULTS_SUITE` when testing settings, never real preferences.
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
   artifact on a clean Mac; generate checksums and include dependency notices.

The release workflow is temporarily manual/build-only to prevent accidental
unsigned publication. Fix its signing path before restoring tag-triggered releases.

## Audit limitations

Earlier history checks found no tracked agent-run artifacts and no obvious secret
patterns. That was a heuristic scan, not a comprehensive secret/license audit.
Author names/emails and signed-commit identities are public Git metadata. Complete
a dedicated history scanner and dependency-license/notice review before release.
