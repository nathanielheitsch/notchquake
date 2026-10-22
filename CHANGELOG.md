# Changelog

## v0.1-alpha — 2026-10-22

First pre-public alpha. Shipped as `NotchQuake.app` (bundle id `local.notchquake`,
binary `notchquake`).

- **Key routing fixed.** NSEvent keyCodes are Carbon virtual-key codes, not
  letters: the old table sent CR/BS/Tab while typing `w/c/f/7`. Ctrl chords now
  derive from `event.characters()`, and only the verified special-key codes
  (Esc 53, Backspace 51, Return 36, Tab 48, arrows 123–126, ForwardDelete 117)
  are special-cased. Typing `wcf7` now produces `wcf7`.
- **Tab close and last-tab collapse.** The `×` affordance is hover-only, and
  closing the last tab settles the panel back to the 33pt pill instead of
  respawning mid-collapse. Closing a tab no longer hangs the app: the reader
  thread's `JoinHandle` is dropped on the main thread by way of `Pty::Drop`, so
  it is taken and `mem::forget`-ed after `Msg::Shutdown` is sent.
- **Real native settings window.** The hand-drawn in-panel overlay was replaced
  with a titled layer-0 `NSWindow` of native AppKit controls (sliders, popup
  menus, switches, colour wells, a segmented control, 16 ANSI swatches), so the
  settings surface takes real clicks and can be inspected by window tools.
- **Seamless header.** The header bar is filled with the terminal background
  rather than a separate chrome colour, and a 32pt shadow fade replaces the hard
  separator so the top text rows fade under the bar.
- **Open-width canvas + reflow.** Panel width is no longer capped by a fixed
  640pt canvas: the canvas is the notch display's logical width, sized once at
  startup, and the slider rail, clamp and open panel share that single source
  of truth. Changing size knobs while open re-wraps the grid via
  `Term::resize` plus a `Msg::Resize` on the existing PTY wire.
- **OSc/PTY integration.** A byte-transparent PTY tee parses OSC sequences
  alacritty drops (133, 1337, 7, 6, 9;4) without stealing bytes from the real
  read loop, and a pid-scoped `ZDOTDIR` shim emits the hooks stock zsh never
  sends — enabling cwd-collapsed titles, tinted tabs and nonzero-exit markers.
- **Product renamed** from `rust-notch-terminal` / `BNT.app` to **NotchQuake** /
  `NotchQuake.app`. Internal prefs key prefix `BNT.` and `BNT_DEFAULTS_SUITE`
  are unchanged; the prefs domain changed, so this is a one-time settings reset.
