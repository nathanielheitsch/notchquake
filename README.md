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

- macOS only, **arm64**. There is no Intel, Linux, or Windows build.
- A display with a notch (or a notch display) — the panel is positioned from the
  notch geometry and sized to the notch display's logical width at startup.

## Build

```bash
cargo build              # the gate — there is no LSP in this repo
./scripts/launch.sh      # build + kill stale instances + relaunch NotchQuake.app
```

`launch.sh` **kills every running instance before relaunching** (including
renamed debug copies): a stale binary reuses the running process via `open`, and
a stale Carbon registration silently steals the exclusive Option+Space chord.

## Validation gates

```bash
cargo fmt --all --check
cargo build
cargo test           # pure parser/geometry tests only — never spawn a PTY in a test
```

## Hotkeys

| Chord | Effect |
| --- | --- |
| Option+Space | Toggle the notch panel open / closed |
| Cmd+T | Spawn a new PTY tab (inert while the panel is closed) |

Chords are configurable, but only within the registered Carbon presets — the
Carbon Event Manager can deliver chords it registered at launch, and recording
an arbitrary chord needs an Input Monitoring grant this app does not have.

## Settings persistence

Settings live in `~/Library/Preferences/local.notchquake.plist`, named after the
bundle id. The BNT → NotchQuake bundle-id change reset preferences once;
ordinary version updates do not change the preferences domain.

## License

MIT — see [LICENSE](./LICENSE), Copyright 2026 Barnstorm LLC.

Attribution: NotchQuake embeds [`alacritty_terminal`](https://github.com/alacritty/alacritty_terminal)
(Apache-2.0) for the terminal grid and PTY plumbing, and the
[`objc2`](https://github.com/kacperix/objc2) family of bindings for the AppKit/Core
surface it is written against.

## Signing & releases (Barnstorm LLC Developer ID)

**Not production-ready yet.** See [the current project state](docs/project-state.md)
for implemented features, validation, and signing/release blockers. The existing
setup and release scripts are scaffolding, not a verified publishing pipeline.
Notarization can be automated with `xcrun notarytool`; the earlier claim that
it required a web-only submission was incorrect.

CI checks `main` pushes and pull requests. The release workflow is manual and
build-only until Developer ID signing, notarization, and stapling are verified.
