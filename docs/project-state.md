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
`main` pushes and pull requests on the **arm64** runner label `macos-26`.
`macos-14` is retired on GitHub Actions (all macOS 14 images were removed
2026-11-02, actions/runner-images#15226) and must not be used; the Intel labels
(`macos-26-large`, `macos-26-intel`, `macos-15-large`) are wrong for this project
because there is no Intel build.

The gate also assembles the bundle and uploads a deterministic, checksummed arm64
artifact (`dist/NotchQuake-<version>.tar.gz` + `.zip` + `SHA256SUMS`). That
artifact carries only an ad-hoc local signature: it is a build output for
inspection, **not** a release, and `spctl` rejects it.

On the pushed `main` HEAD, formatting, `cargo build --locked`, and
`cargo test --locked` pass locally (**44 tests, 0 failures**). Earlier work also
reports live panel/tab/hotkey checks.

Real-user slider dragging, popup selection, widened-panel outside-click behavior,
all settings tabs, and shell-exit teardown still need a complete manual pass.
Use the throwaway suite when testing settings:
`BNT_DEFAULTS_SUITE=bnt-verify open -n NotchQuake.app`. An empty value refuses
the default domain instead of falling back; that is intentional.
The app bundle is a build product and is gitignored. `scripts/launch.sh` now
**assembles** it via `scripts/build-bundle.sh` (fresh binary + tracked
`packaging/Info.plist`), so a stale bundle can never be what you test and a
clean source-only checkout works without a pre-existing bundle.

## Release foundation — stage 1 done, stage 2 blocked

The staged plan, exact secrets, keychain discipline, and clean-Mac validation
steps live in [`docs/release-runbook.md`](./docs/release-runbook.md). Stage 1 is
implemented and verified locally; **stage 2 (Developer ID signing, notarization,
stapling, publication) is blocked because no credentials exist.** No release has
been cut and nothing in this repository has ever been notarized.

Done (stage 1, verified locally 2026-11-12):

- Supported arm64 runner label (`macos-26`) replacing retired `macos-14` in both
  workflows.
- `packaging/Info.plist` is now **tracked** bundle metadata: version
  (`CFBundleShortVersionString` from `Cargo.toml`), deterministic
  `CFBundleVersion` (`<version>+<commit>`), `LSMinimumSystemVersion` 13.0,
  `NSHumanReadableCopyright`, `LSUIElement`. No secrets, no team ID, no
  certificate names.
- `scripts/build-bundle.sh` assembles the bundle from the template plus the fresh
  binary, forces `MACOSX_DEPLOYMENT_TARGET=13.0`, builds
  `--target aarch64-apple-darwin`, and **fails if the binary's
  `LC_BUILD_VERSION minos` disagrees with `LSMinimumSystemVersion`**. Verified:
  both report `13.0`; `file` reports arm64.
- It copies Resources (notices + `licenses/`) and then re-seals the bundle with an
  **ad-hoc** signature. Verified reason: a bundle with `Contents/Resources` and no
  `_CodeSignature` fails `codesign --verify --strict` with "code has no resources
  but signature indicates they must be present" — the old bundle's plist had 4
  keys and sealed 0 files, so its checks were meaningless. The re-sealed bundle
  verifies strictly (`valid on disk`, `satisfies its Designated Requirement`) and
  still reports `Signature=adhoc`, `TeamIdentifier=not set`, `spctl: rejected`.
- `scripts/package-artifact.sh` produces a deterministic archive (sorted entries,
  epoch mtimes **and atimes**, uid/gid 0, no AppleDouble/`.DS_Store`, `gzip -n`,
  `bsdtar --format ustar`) plus `SHA256SUMS`, and **proves** reproducibility by
  archiving the same staged tree twice and comparing sha256. The ustar format is
  required: bsdtar's default pax format writes atime/ctime as extended records, so
  reading a file between two runs changed the bytes (measured). Verified: two
  independent bundle builds into different `NQ_APP` directories gave the same
  tar.gz sha256 (`ca09ef19…`) and zip sha256 (`19f5d5b9…`); bundles extracted from
  both the tar.gz and the zip pass `codesign --verify --strict`.
- `scripts/launch.sh` no longer claims a `"Barnstorm LLC"` identity check it can
  never pass; it delegates provenance to `build-bundle.sh` and reports the actual
  `open -n` error instead of asserting a stale-signature cause.
- `scripts/release.sh` is prep-only: gates → bundle → archive → checksums, asserts
  the licence artefacts **inside the archive**, warns when the tag has no
  `CHANGELOG.md` section, refuses to publish, and creates a signed annotated tag
  only with `--tag` — and only on a clean tree, never over an existing tag, with no
  unsigned fallback.
- `release.yml` splits into a credential-free `build` job and a `publish` job that
  requires `publish: true` AND all five secrets, fails closed otherwise, checks the
  tag is a **signed annotated** tag (a `-----BEGIN PGP SIGNATURE-----` block in the
  tag object — not a `gpgsig` header, which only signed *commit* objects carry),
  and runs in an ephemeral keychain with an `if: always()` cleanup sweep.
- The About row reads `env!("CARGO_PKG_VERSION")` instead of the hardcoded
  `v0.1-alpha`, so it cannot drift from the bundle metadata.

Still blocked — stage 2 (do not treat any of it as working):

1. Obtain Barnstorm LLC's **Developer ID Application** certificate with its
   private key, exported as a passphrase-protected `.p12`. A self-signed
   certificate is not a Developer ID identity; `scripts/setup-developer-id.sh`
   now checks the **Authority chain + `security find-identity -v`**, not the old
   `grep '^Signature=Developer'`, and refuses a blank passphrase.
2. `scripts/sign-notarize.sh` (new) implements sign → notarize → staple →
   re-archive, but has **never been run end to end** — no certificate, no API key.
   Read it as a plan that refuses loudly.
3. Secrets must be created by the user: `DEVID_P12_B64`, `DEVID_P12_PASS`,
   `NOTARY_API_KEY`, `NOTARY_KEY_ID`, `NOTARY_ISSUER`. Nothing in this repo has
   accessed or stored any of them; `.signing/` and `dist/` are gitignored.
4. Publication requires successful gates, a signed annotated tag, `Accepted`
   notarization, a stapled ticket, checksums, and a **clean-Mac** Gatekeeper check
   reporting `source=Notarized Developer ID`.
5. A dedicated history/secret scanner is still outstanding (see below).

There is intentionally **no tag-triggered release workflow**: publication needs an
explicit `workflow_dispatch` with `publish: true`, so an unsigned bundle can never
be published by accident.

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

Packaging now carries them: `scripts/build-bundle.sh` installs `NOTICE.md`,
`THIRD-PARTY-NOTICES.md`, `LICENSE`, and `licenses/` into
`NotchQuake.app/Contents/Resources/`, and `scripts/release.sh` plus the release
workflow assert they exist **inside the archive**, not merely in the working tree.
`scripts/gen-notices.sh check` stays a mandatory gate step on every push and on
the release commit.

Verified locally: `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`; the
only remaining warning is the expected duplicate `windows-sys` pair gated to non-shipped
Windows targets; regeneration is byte-identical; `scripts/gen-notices.sh linkage` matches the
release build. `Cargo.toml` gained `license = "MIT"`, `repository`, `description` so
`cargo deny` reports no `no-license-field` for the root crate (no dependency changes, so
`Cargo.lock` is untouched).

Open questions are recorded in `THIRD-PARTY-NOTICES.md` rather than answered there — chiefly
the objc2 family's Apple-SDK-derivation note and the fact that its published crates ship no
licence text (texts pinned from upstream instead). Not legal advice.
