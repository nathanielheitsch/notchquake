# NotchQuake

**NotchQuake** — a pre-public alpha from Barnstorm LLC.

An iTerm-style terminal that lives *inside the macOS display notch*. At rest it
is a small pill parked in the notch; the Option+Space hotkey (or a click on the
pill) expands it into a full panel: a live PTY grid, an iTerm-style tab strip
(square tabs, a `+` button that spawns sessions, a hover-only `×` that closes
one, `⌘<n>` hints), shell-integration metadata captured over a byte-transparent
PTY tee (OSC 0/2 titles, OSC 133/1337 cwd + exit status → tinted titles and
activity dots), and a real native settings window with sliders, popup menus,
switches, colour wells and ANSI swatches. Everything is drawn with CoreGraphics
over a non-activating layer-27 panel, and the hotkeys come from the Carbon
Event Manager, so they work without Accessibility or Input-Monitoring grants.

## Requirements

- macOS only, **arm64**, **macOS 13 or later**. There is no Intel, Linux, or Windows
  build. The deployment target is 13.0 (the notch geometry needs
  `NSScreen.safeAreaInsets` / `auxiliaryTop*Area`); `scripts/build-bundle.sh` fails
  if the compiled binary and `LSMinimumSystemVersion` disagree.
- A notch display is ideal. Without one, the panel parks at the top-centre of the
  lowest screen using a 33pt fallback pill.

## Build

```bash
cargo build                            # the gate — there is no LSP in this repo
./scripts/launch.sh                    # build + assemble bundle + kill stale + relaunch
scripts/build-bundle.sh release        # bundle only, into ./NotchQuake.app
NQ_APP=/tmp/nq/NotchQuake.app scripts/build-bundle.sh release   # bundle elsewhere
```

`launch.sh` **assembles the bundle from the freshly built binary plus the tracked
`packaging/Info.plist`**, so a stale gitignored `NotchQuake.app/` can never be what
you test. It then **kills every running instance before relaunching** (including
renamed debug copies): a stale binary reuses the running process via `open`, and a
stale Carbon registration silently steals the exclusive Option+Space chord.

The bundle it produces carries only an **ad-hoc local signature**. That is enough to
run on your own Mac; it is not distributable (`spctl` rejects it, notarization will
never accept it). See [Signing & releases](#signing--releases-barnstorm-llc-developer-id).

## Validation gates

```bash
cargo fmt --all --check
cargo build
cargo test           # pure parser/geometry tests only — never spawn a PTY in a test
scripts/gen-notices.sh check     # third-party notices are current (offline)
scripts/release.sh v0.2.0        # all of the above + bundle + checksummed archive
```

## Hotkeys

| Chord | Effect |
| --- | --- |
| Option+Space | Toggle the notch panel open / closed |
| Cmd+T | Spawn a new PTY tab — terminal-local, only while the panel is open and holds key focus |

Chords are configurable. The global toggle accepts any unreserved Option/Cmd/Control
chord; the new-tab chord is terminal-local and only runs while the panel has key focus.
macOS-reserved or conflicting chords are rejected. If a chord cannot be registered,
the runtime falls back to the default without rewriting the stored preference.

## Settings persistence

Settings live in `~/Library/Preferences/local.notchquake.plist`, named after the
bundle id. The BNT → NotchQuake bundle-id change reset preferences once;
ordinary version updates do not change the preferences domain.

## License

MIT — see [LICENSE](./LICENSE), Copyright 2026 Barnstorm LLC.

Third-party attribution lives in [NOTICE.md](./NOTICE.md) (short form) and
[THIRD-PARTY-NOTICES.md](./THIRD-PARTY-NOTICES.md) (per-dependency, with the licence texts
and their pinned provenance in [`licenses/`](./licenses/)). NotchQuake embeds
[`alacritty_terminal`](https://github.com/alacritty/alacritty) (Apache-2.0) for the terminal
grid and PTY plumbing, and the
[`objc2`](https://github.com/madsmtm/objc2) family of bindings for the AppKit/Core surface it
is written against. Regenerate and check them with `scripts/gen-notices.sh gen` /
`scripts/gen-notices.sh check`; `deny.toml` plus `scripts/gen-notices.sh deny` (cargo-deny)
fail on any licence outside the allow-list or any dependency source other than crates.io.
Include `NOTICE.md`, `THIRD-PARTY-NOTICES.md`, `LICENSE`, and `licenses/` in a release archive.
`scripts/build-bundle.sh` installs them into `NotchQuake.app/Contents/Resources/`, and
`scripts/release.sh` asserts they are present **inside** the archive.

## Signing & releases (Barnstorm LLC Developer ID)

**Signing is blocked until credentials exist.** Stage 1 — CI, bundle metadata, a
deterministic artifact, and checksums — is implemented; stage 2 — Developer ID
signing, notarization, stapling, and publication — has never been run, because
there is no Developer ID certificate and no notary API key. Nothing here has ever
been notarized, and no release has been published.

Read [docs/release-runbook.md](./docs/release-runbook.md) for the staged process,
the exact GitHub Actions secrets (`DEVID_P12_B64`, `DEVID_P12_PASS`,
`NOTARY_API_KEY`, `NOTARY_KEY_ID`, `NOTARY_ISSUER`), ephemeral-keychain discipline,
signed-tag and clean-Mac Gatekeeper validation, and the future Homebrew cask
prerequisites. [docs/project-state.md](./docs/project-state.md) is the authoritative
blocker list.
```bash
scripts/release.sh v0.2.0        # gates -> bundle -> archive -> checksums; refuses to publish
scripts/release.sh v0.2.0 --tag  # plus a signed annotated tag (never an unsigned one)
```

Notarization is automatable with `xcrun notarytool` and an App Store Connect API
key; the earlier claim that it required a web-only submission was incorrect. The
identity check that matters is the **Authority chain**
(`Authority=Developer ID Application: …`) plus `TeamIdentifier` and
`security find-identity -v -p codesigning` — not a `Signature=` grep, which an
ad-hoc signature also satisfies.

CI checks `main` pushes and pull requests on the arm64 runner label `macos-26`
(`macos-14` is retired), and uploads a deterministic but **ad-hoc-signed** build
artifact. Release publication requires an explicit `workflow_dispatch` with
`publish: true` and fails closed when any signing secret is missing; there is no
tag-triggered workflow.
