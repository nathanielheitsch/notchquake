# AGENTS.md — Rust Quake-Notch Terminal

Development notes for agents working in this repo (`rust-notch-terminal`,
shipped as `BNT.app`). macOS-only, arm64. This is a **standalone Rust app**,
not part of `boring.notch` — the plan lives in the sibling worktree at
`../emdash/worktrees/boring.notch-*/issue-quake-terminal-k605g/docs/rust-notch-terminal-plan.md`.

## Build / run / verify

```bash
cargo build                                    # the gate — there is no LSP
./scripts/launch.sh                            # build + kill stale + relaunch BNT.app
# BNT.app is a signed bundle at ./BNT.app; launch it with `open -n BNT.app`
```

`launch.sh` kills any running instance first (a stale binary reuses the
running process via `open`), rebuilds, copies the binary into `BNT.app`,
and relaunches. **Always kill before relaunch** or you test a stale binary.

The app writes debug files to `/tmp/bnt-*.txt` (notch geometry, shape, grid,
key/click/any event dumps). Remove these before shipping. `launch.sh` does
`rm -f /tmp/bnt-*.txt` itself.

## Window architecture (the single most important fact)

The notch panel is a **non-activating `NSPanel` at `kCGWindowLayer == 27`**,
NOT a normal layer-0 window. Consequences, all verified:

- **It cannot take key focus** while another app is frontmost, so the
  Option+Space hotkey is registered system-wide with the Carbon Event Manager
  (`RegisterEventHotKey` in `src/hotkey.rs`), which delivers a dedicated
  hotkey event to this app on the main run loop with **no Accessibility /
  Input Monitoring grant** and without being frontmost. (The old
  `NSEvent::addGlobalMonitorForEvents` path could NOT see key events — Apple
  gates global key monitoring behind an accessibility trust our adhoc-signed
  dev bundle never gets, so it silently received zero callbacks. Carbon sees
  only this chord, never other keystrokes.) To type into the open terminal,
  a human must click inside the grid first to claim key focus (the local
  `KeyDown` monitor then routes keystrokes to the PTY).
- **Clicks** are handled two ways that do NOT double-fire: a real click ON the
  opaque silhouette goes to the view's `mouseDown:` override (the window
  server routes it to the panel, which sits above the menu bar); a click on
  a transparent region — or a synthetic/test click delivered to the
  frontmost app — is observed by the global `LeftMouseDown` monitor and
  hit-tested via `NSEvent::mouseLocation()`. Both paths call the shared
  `select_at_screen_point` (toggle when closed, switch PTY tab when open).

## Observation: use `computer_use_get_desktop_state`, NOT window tools

The pi-computer-use extension **is installed and works** for observing this
app, but only one of its tools reaches the panel:

| Tool | Reaches the panel? | Why |
| --- | --- | --- |
| `computer_use_get_desktop_state` | ✅ YES | Full composited desktop capture. Shows the panel exactly as the user sees it. Use this. |
| `computer_use_list_windows` | ❌ NO | Returns **0 windows** for the BNT pid — only lists layer-0 top-level windows; the panel is layer 27. |
| `computer_use_get_window_state` | ❌ NO | Needs a layer-0 window + AX tree; the panel has no AX tree (`ax_window_unresolved`). |
| `computer_use_bring_to_front` | ❌ NO | Refuses a non-activating panel. |
| `computer_use_zoom` | ❌ NO | Requires a prior `get_window_state` snapshot, which can't target the panel. |
| `computer_use_hotkey` / `computer_use_click` (scope=desktop) | ✅ works | Option+Space fires the Carbon hotkey (verified: toggles open/close); a click on the pill/tab fires `mouseDown:` or the global monitor (verified: opens + switches tabs). Note: synthetic clicks in the menu-bar strip (y<24pt) are intercepted by the menu bar and only reach the global-monitor fallback, not `mouseDown:` — real clicks hit the panel directly. |

For **alpha/pixel measurement** of the panel's own pixels (transparency vs
painted, silhouette taper), the complementary tool is the shell:

```bash
swift scripts/bntw.swift   # print BNT window id + layer (CGWindowListCopyWindowInfo)
screencapture -x -o -l <id> out.png   # window-local capture WITH alpha
magick out.png -resize 640x620! s.png && magick s.png RGBA:- | python3 -c '…'  # sample pixels
```

`screencapture -x -o -l` is the only way to test transparency — a full-screen
capture composites the desktop behind any alpha-0 region, so it CANNOT
distinguish "transparent" from "painted with the wallpaper colour". Use the
window capture for alpha, the desktop capture for the composited result.

## What's built and verified (2026-10-09)

| Gate | Status | Evidence |
| --- | --- | --- |
| `cargo build` (arm64) | ✅ | clean |
| Panel top-center, above menu bar, layer 27 | ✅ | id, bounds (535,0) 640×620 |
| Closed pill ~33pt at startup | ✅ | shape dump h=33.5 w=189 |
| Expand/collapse round-trip via `toggle()` | ✅ | 33.5 → 420 → 33 |
| Rounded silhouette, bottom taper ~20pt/side | ✅ | bottom row 599 vs 639 wide |
| Live shell grid (text visible) | ✅ verified | window-local alpha capture (open h=420): 704 light text pixels; row 0 reads `Waiting for another flutter command to release the startup lock…` |
| Tab bar (iTerm style): 3 tabs + `+` button, selected lighter, dividers | ✅ | window-local alpha capture (open h=420): selected x 33–216, unselected 218–607, painted through y=420 (flush with chin line), straight tab top at y=404 with corners curving in from 399, `+` at x≈588–608. **Restyled 2026-10-10** (commit `4c2f172`): square tabs (no rounded corners), all tabs filled `0D0C0C`, only the ACTIVE tab has 1px LINE (`3E3D3D`) side borders (no top/bottom), separators `3E3D3D`, labels `⌘1/⌘2/⌘3` (active `FBFAFC`, inactive `828383`), header block filled `0D0C0C` with a `3E3D3D` separator under it. |
| Click tab → switch PTY | ✅ verified | `mouseDown:` on the open panel selects the tab under the cursor; synthetic click on the middle tab moved selection from tab 0 (sel 64–435) to tab 1 (sel 434–805) |
| Option+Space global hotkey | ✅ verified | Carbon `RegisterEventHotKey` (no TCC grant); synthetic Option+Space toggles open then close |
| Click closed pill → open | ✅ verified | synthetic click on the pill toggles open (via `mouseDown:` for real clicks / global monitor fallback) |
| Scroll wheel → history | ✅ wired | ONE global `ScrollWheel` monitor → `scroll_wheel()`; alacritty `Scroll::Delta`, `BNT_SCROLLBACK` (default 10000). Wheel events proven to reach the handler (40× dy=-1 logged). Grid visibly moves only when the session has scrollback history — a fresh login shell has none, so `display_offset` clamps to 0. |
| Click outside panel → collapse | ✅ built | `click_at_screen_point` collapses an open panel on an outside click; idempotent `expand`/`collapse` so both click paths land once. |
| Header bar (notch height, open state) | ✅ verified | desktop capture: grid text starts at y≈50pt = notch 33 + GRID_TOP_PAD 22; the notch-height block above it is reserved (empty for now, buttons later). `row_rect`/`draw_grid` take a `header` param; `grid_dims` reserves the same inset. |
| Dynamic tabs: `+` spawns | ✅ verified | synthetic click on the `+` grew the strip from 3 to 4 tabs (labels reflowed, tab_w shrank). `sessions` is a live `Cell<Vec<Session>>`; `tab_strip(h,w,count)` is the single source of truth for renderer + hover + click. |
| Tab close via `×` on hover | ⚠️ built, not driver-verifiable | hover needs a REAL mouse move (NSTrackingArea `mouseMoved:` never fires for synthetic pointer moves — same synthetic-event limitation as keystrokes). `×` drawn only on `hovered` tab; click on it → `close_tab` (selects next, collapses on last). |
| Shell exit self-destruct | ⚠️ built, not driver-verifiable | `Event::Exit` (post-drain) sets `Arc<AtomicBool>` dead flag; `tick` → `reap_dead_sessions()` tears down via `Msg::Shutdown` (reader thread returns → `Pty::Drop` SIGHUPs the child). Needs a real `exit` keystroke to exercise. |

## The tab-bar defect — root cause (fixed 2026-10-09)

The outlined pills set the **fill** color to gray but never set the **stroke
color**, so `stroke_path` painted black-on-black (CGContext's default stroke
is opaque black). `begin_path` was not the problem. The iTerm restyle fills
every tab (selected 0.22 gray, unselected 0.10) so no stroke color is needed
for the tabs at all; the only stroke left is the 1px divider, which now sets
`set_gray_stroke_color` explicitly. `trace_capsule` was replaced by
`trace_tab_rect` (square bottom, rounded top corners).

## The grid-freeze + top-clip bugs (fixed 2026-10-09)

Two more bugs made the terminal look empty even though the grid was populated:

1. **`tick` only redrew while the spring was animating.** Once the open
   animation settled, `drawRect:` was never called again, so PTY output
   (processed on the reader thread) was never painted — the grid stayed
   frozen at whatever it held on the last animation frame. Fix: `tick`
   marks the view dirty and calls `update`/`displayIfNeeded` whenever the
   spring is flying **OR** the panel is open (`height > CLOSED_HEIGHT`). A
   terminal redraws continuously while open; the 60Hz pump is the frame
   driver.
2. **Row 0 was clipped by the silhouette top.** The grid hung flush against
   the panel top (`y = PANEL_HEIGHT`), so the first row's cap-height sat above
   the top edge and inside the rounded-corner taper. Fix: `GRID_TOP_PAD`
   (22pt) vertical inset in `row_rect` so row 0 clears the top edge and taper.

Also fixed en route: PTY control responses (DSR/DA queries from the shell
prompt) are now routed back to the PTY via `PtyWrite`, so the shell stops
waiting on unanswered terminal queries.

## The shipped tab strip

iTerm-style: tabs are rounded-TOP-corner rectangles flush with the silhouette
chin line (square bottom edge, rounded part facing up into the panel),
selected tab lighter, unselected dimmed, 1px dividers between tabs, a `+`
button at the right end (decorative — sessions are a fixed `TAB_COUNT`, so it
is not a click target). Geometry lives in **`tab_strip(h, w)`**, the single
source of truth shared by the renderer and `select_at_screen_point`, so draw
and click hit-testing cannot drift. Tabs inset by `chin_radius(h) + TAB_INSET
from the silhouette sides so they stay inside the clip at every animation
height.

## Lessons learned this session

1. **`kCTFontFeatureSettingsAttribute` does NOT work as a text attribute for
   ligature shaping.** CoreText/HarfBuzz applies `calt` from the *font*, not
   the attributed string. The ligature test (`debug_ligature_check` in
   `font.rs`) reports 2 glyphs for `->` either way because **Fira Code draws
   `->` as two `.seq` half-glyphs that compose the arrow** — `glyph_count == 2`
   is correct, the glyph *names* (`hyphen_start.seq`+`greater_hyphen_end.seq`)
   prove the feature applied. Don't gate on glyph count; gate on glyph names.

2. **Fira Code's GSUB has no named `calt`/`liga` feature tags** — its
   ligatures live in the DFLT/latn script's default lookups, applied by
   default. `hb-shape` confirms it. CoreText applies them by default too; the
   `kCTFontFeatureSettingsAttribute` text-attribute approach is a dead end.

3. **`CFRange { length: 0 }` is the INSERT range** for
   `CFAttributedStringReplaceString`; using the text length there crashes
   (out-of-range). Insert with length 0, then set attributes over the
   inserted text's full range.

4. **`CFMutableArray<CFString>::empty()` + `.append(&calt)`** is the clean
   way to build the feature-settings array in objc2-core-foundation. Requires
   the `CFArray` feature on `objc2-core-foundation`.

5. **Non-activating panel + global event monitor** (corrected): the old
   `addGlobalMonitorForEvents` monitor installed (`ok`) but received zero
   key callbacks because the app lacked an accessibility trust (see lesson 10),
   NOT because of synthetic events. The autoToggle NSTimer scaffolds that
   worked around this are now gone — replaced by the Carbon hotkey +
   `mouseDown:`, both verified with synthetic input.

6. **Verification scaffolds were removed.** The `autoToggle` timers and
   `auto_timer`/`auto_timer2` ivars are deleted; all `/tmp/bnt-*.txt` debug
   writes are gone; `debug_ligature_check`/`measure` are gone. The hotkey +
   click paths are now driven (and verified) with the computer-use tools.

7. **The view is NOT flipped** (`setIsFlipped:` is unbound in objc2-app-kit),
   so the drawing space is the window's own **y-up** space: the panel top
   (menu bar) is at `y = PANEL_HEIGHT`, the chin at `y = PANEL_HEIGHT - h`.
   `select_at_screen_point` must NOT invert the click point — the original
   code did (`y = height - y_up`), which put tab clicks ~2× the strip height
   off. Hit-test the strip as `y ∈ [PANEL_HEIGHT - h, PANEL_HEIGHT - h +
   TAB_HEIGHT]` directly.

8. **CGContext fill and stroke colors are separate state.** A `stroke_path`
   with only `set_fill_color_with_color` set paints default-black. Always
   `set_gray_stroke_color`/`set_stroke_color_with_color` before stroking.

9. **`NSEvent.keyCode()` returns the macOS virtual key code**, not ASCII or
   X11. Space is **49** (`0x31`), not 6 (which is Z). The old
   `SPACE_KEY_CODE = 6` silently rejected every Option+Space.

10. **`NSEvent::addGlobalMonitorForEvents` cannot see key events without an
   accessibility trust** (Apple's own doc: key events "may only be monitored
   if accessibility is enabled"). An adhoc-signed, no-entitlement dev bundle
   gets neither Accessibility nor Input Monitoring, so the monitor installs
   (non-nil) but receives **zero** callbacks — for real *and* synthetic
   input. Mouse events are NOT gated this way, so the global `LeftMouseDown`
   monitor works. The zero-permission fix for a global hotkey is Carbon
   `RegisterEventHotKey` (`src/hotkey.rs`): it delivers a dedicated hotkey
   event to the app on the main run loop, needs no grant, and works without
   being frontmost. CGEventTap is the Input-Monitoring-gated alternative
   (returns NULL loudly on denial).

11. **A non-activating panel's view DOES receive `mouseDown:`** for real
   clicks on its opaque regions — that is the reliable click path. The
   global `LeftMouseDown` monitor is only the fallback for clicks that pass
   through transparent regions or synthetic clicks delivered to the
   frontmost app. `handle_click` must use `NSEvent::mouseLocation()` (absolute
   screen point), NOT `window.frame.origin + locationInWindow()` — a global
   event's `window` is nil, so the old code early-returned and swallowed
   every click.

12. **`CTLineDraw` paints BLACK unless the attributed string opts in.** Setting
    only the CGContext fill color does nothing for a `CTLine` — CoreText uses
    the string's own `kCTForegroundColorAttributeName`, defaulting to black
    (black-on-black = invisible). Set
    `kCTForegroundColorFromContextAttributeName = YES` (a `CFBoolean`, via
    `kCFBooleanTrue`) over the full range so the line reads the per-run
    `set_fill_color` the caller sets. The tab labels looked fine because they
    happened to be drawn where black-on-black was still legible against the
    dim tab fill; the grid was not.
13. **Claim key focus on open, not just on click.** `set_target` calls
    `window.makeKeyWindow()` when opening, so a hotkey-open lets the local
    key monitor fire immediately — no follow-up click needed before typing.
    Legal because `canBecomeKeyWindow -> true`; the app is never activated,
    so the previous frontmost app stays visually frontmost.

14. **`trace_silhouette` chin arcs must use the SAME control-point convention
    as the tab rects.** The right chin corner's second control point was
    `max_x - bottom` (should be `max_x`), inverting the arc so the bottom-right
    corner bulged the wrong way. Symptom: bottom-right corner looked clipped
    / inverted while the left was correct. Verify with a window-local alpha
    capture: the bottom row must inset the SAME amount on both sides.

15. **`addGlobalMonitorForEventsMatchingMask` keeps only ONE handler per event
    type.** Registering a second global monitor on the same mask (e.g. two
    `ScrollWheel` monitors — one for the wheel, one for display changes)
    silently DROPS the first; the wheel stops working. Combine into one
    monitor and dispatch on `event.subtype()` inside the handler.

16. **A display change (monitor unplugged / resolution changed) arrives as a
    `ScrollWheel` event with subtype `ScreenChanged`** carrying a bogus
    `deltaY`. Handle it in the single ScrollWheel monitor: call `reposition()`
    (re-pin the panel to the notch display's top-centre + refresh `set_notch`),
    never `scroll_wheel`. The panel was placed once at startup, so an
    unplug/resolution change left it off-centre. Verified: desktop 1920×1243
    pts, panel bounds x=640 w=640 → center 960 = exact screen center.

17. **Dynamic tabs (2026-10-10).** `sessions` is a live `Cell<Vec<Session>>` with
    `spawned: Cell<bool>` (so closing the last tab doesn't auto-resurrect) and
    `selected: Cell<Option<usize>>`. `tab_strip(h, w, count)` takes the live
    count — renderer, hover, and click all pass it, and the `+` rect and the
    `×` rect derive from its outputs in BOTH draw and hit-test. `tab_count`
    mirrors `sessions.len()` for the hover/click path. `Cell<Vec<T>>::update`
    does NOT compile (needs `T: Copy`); use the move-out/move-back idiom
    (`ivars.sessions.replace(vec![])` … `replace(v)`) with short panic-free
    pairs. `Mut<T>` is unavailable in this toolchain.

18. **Shell-exit teardown is `Msg::Shutdown`, not a raw kill.** `Session::destroy()`
    sends `Msg::Shutdown` → `drain_recv_channel` returns false → the reader
    thread returns → `EventLoop`/`Pty` drop → `Pty::Drop` SIGHUPs the child and
    unregisters the SIGCHLD hook. Dropping the `JoinHandle` alone detaches the
    thread and kills nothing. The dead signal is `Event::Exit` (sent by
    `Term::exit()` AFTER the `drain_on_exit` final read, so the last output
    lands in the grid first) — NOT `ChildExit` (pre-drain) and never `Wakeup`
    (fires continuously while alive). Main thread reaps in `tick` via
    `reap_dead_sessions()`.

19. **First-open must own the selection.** `selected` starts `None` and collapse
    sets `None`; the spawn branch in `draw` sets `Some(count-1)` — otherwise
    `draw_grid` matches `_ => {}` and the terminal renders blank on open.

20. **NSEvent keyCodes are NOT letters.** The old `keyCode - 1` Ctrl mapping
    and the `2 => BS / 4 => EOT / 10 => CR` arms broke plain typing: keyCode
    4 is **H** (sent Ctrl-D = EOT, killing the line), 2 is D (sent BS), 13 is
    W (sent CR = early submit). Fix: Ctrl chords derive from
    `event.characters()` (`b & 0x1f`), and keyCode arms are reserved for
    actual special keys (53=Esc 8=Backspace 13=Return 3=Enter 26=Tab). Plain
    printable chars already fall through via `characters()`.

21. **Draw order in the tab strip = z-order.** Fills → LINE separators/borders
    → labels+hint → `×` LAST, so a long shell title can never overpaint the
    `×` or the right-aligned `⌘<n>` hint (title width is truncated by
    `tab_w - hint_w`).

22. **`CTLineDraw` defaults to BLACK unless the attributed string opts into
    the CGContext fill color.** Any shaper must set
    `kCTForegroundColorFromContextAttributeName = YES` over the full range
    (see `render.rs::run` and `notch_view.rs::shaped_line`); without it
    `set_fill_color_*` has no effect on the text and it paints black-on-black
    (invisible on the dark panel). This bit twice: the grid, then the tab
    labels/hints.

23. **Compare against the guidance image with a real PNG decoder, not a
    vision model.** Inline vision descriptions hallucinate freely (they
    "saw" text that pixel data proves absent). The only trustworthy check is
    a filter-correct PNG decode (`/tmp/png.py` handles per-scanline filter
    bytes — a naive `zlib.decompress` without unfiltering returns garbage)
    plus numeric region checks: per-tab color counts, glyph x-runs, edge
    columns. Verify one settled frame, not motion frames.

24. **The `×` gutter must be reserved in the title band.** A long shell
    title centered in the full tab width starts at the tab's left edge and
    overpaints the left-aligned `×`. The active/hovered tab's label band
    starts at `TAB_CLOSE_W` (`CLOSE_INSET + CLOSE_R`) from the tab's left;
    other tabs keep the full width.

## Repo / process rules

- macOS-only, arm64. `cargo build` is the gate; no LSP.
- `BNT.app/` is gitignored (built binary bundle — never commit). `/tmp/bnt-*.txt`
  debug writes were all stripped at ship time; none remain in `src/`.
- The plan doc (sibling worktree) is the spec; this AGENTS.md is the
  machine-specific how-to.