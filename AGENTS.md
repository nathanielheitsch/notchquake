# AGENTS.md — Rust Quake-Notch Terminal

**Official product name: NotchQuake** (Barnstorm LLC). The shipped bundle is
`NotchQuake.app`, bundle id `local.notchquake`, binary `notchquake`. Historical text
below still says `BNT` / `local.bnt` in places: that is the OLD prefs domain — renaming
the product was a one-time prefs reset, so old settings do not carry over. The internal
prefs KEY prefix `BNT.` and the env var `BNT_DEFAULTS_SUITE` are UNCHANGED and must stay
exactly as written in the lessons.

Development notes for agents working in this repo (`rust-notch-terminal`,
shipped as `NotchQuake.app`). macOS-only, arm64. This is a **standalone Rust app**,
not part of `boring.notch` — the plan lives in the sibling worktree at
`../emdash/worktrees/boring.notch-*/issue-quake-terminal-k605g/docs/rust-notch-terminal-plan.md`.

## Build / run / verify

```bash
cargo build                                    # the gate — there is no LSP
./scripts/launch.sh                            # build + kill stale + relaunch NotchQuake.app
# NotchQuake.app is a signed bundle at ./NotchQuake.app; launch it with `open -n NotchQuake.app`
```

`launch.sh` kills any running instance first (a stale binary reuses the
running process via `open`), rebuilds, copies the binary into `NotchQuake.app`,
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
| Tab bar (iTerm style): 3 tabs + `+` button, selected DARKER, dividers | ✅ | window-local alpha capture (open h=420, commit `8f23b1c`): square tabs, inactive fill `1A1A1A`, **active fill `141414` (darker, per iTerm's dark theme)**, separators/active borders `373737`, labels `FFFFFF`/`969696`, `⌘<n>` hint in iTerm blue accent `4372E0` on EVERY tab. Strip spans the FULL silhouette width (x 0–639.5 = silhouette alpha extent), chin corners cropped by the clip; `+` button square at `TAB_HEIGHT` (28pt, x 612–639.5). Titles start at the same offset on all tabs (35.0/35.3/35.2pt) so they do NOT shift on hover. Chrome text is San Francisco (`ui_font`), ink-centered on `cap_height`. |
| Click tab → switch PTY | ✅ verified | `mouseDown:` on the open panel selects the tab under the cursor; synthetic click on the middle tab moved selection from tab 0 (sel 64–435) to tab 1 (sel 434–805) |
| Option+Space global hotkey | ✅ verified | Carbon `RegisterEventHotKey` (no TCC grant); synthetic Option+Space toggles open then close |
| Click closed pill → open | ✅ verified | synthetic click on the pill toggles open (via `mouseDown:` for real clicks / global monitor fallback) |
| Scroll wheel → history | ✅ wired | ONE global `ScrollWheel` monitor → `scroll_wheel()`; alacritty `Scroll::Delta`, `BNT_SCROLLBACK` (default 10000). Wheel events proven to reach the handler (40× dy=-1 logged). Grid visibly moves only when the session has scrollback history — a fresh login shell has none, so `display_offset` clamps to 0. |
| Click outside panel → collapse | ✅ built | `click_at_screen_point` collapses an open panel on an outside click; idempotent `expand`/`collapse` so both click paths land once. |
| Header bar (notch height, open state) | ✅ verified | desktop capture: grid text starts at y≈50pt = notch 33 + GRID_TOP_PAD 22; the notch-height block above it is reserved. **Restyled 2026-10-12**: the header fill IS `settings.background` (seamless with the terminal), the hard LINE separator is gone, and `draw_header_shadow` fades a 32pt band of terminal-bg color over the top grid rows AFTER the grid (verified by pixel measurement: top row max lum 181/216 ≈ the gradient alpha at that depth, full brightness by 65pt). See lesson 46. |
| Dynamic tabs: `+` spawns | ✅ verified | synthetic click on the `+` grew the strip from 3 to 4 tabs (labels reflowed, tab_w shrank). `sessions` is a live `Cell<Vec<Session>>`; `tab_strip(h,w,count)` is the single source of truth for renderer + hover + click. |
| Tab close via `×` (hover only) | ✅ verified (by construction) | click on the `×` fires `close_tab`. **Changed 2026-10-12**: the `×` draws ONLY on a HOVERED tab (not the active one) and the hit-test matches (`hovered.get() == Some(index)` only) — see lesson 47. Verified lifecycle (before the change): 1 tab → `+` → 2 → ⌘T → 3 → `×` → 2 tabs; closing the LAST tab collapses to the 33pt pill. Closing a live tab no longer hangs the app (see lesson 35). |
| Settings **window** (gear -> separate native `NSWindow`) | ✅ verified | gear click opens a real titled layer-0 window (`BNT Settings` 560x652; `computer_use_list_windows` sees it — the first window tool ever to see anything for this app) with 16 native click-driven controls in one `NSGridView`: 4 `NSSlider`, 6 `NSPopUpButton`, 2 `NSSwitch`, 1 `NSSegmentedControl`, 3 `NSColorWell` (+ 16 display `NSBox` ANSI swatches + About text). Verified: a real pixel click on the Ligatures `NSSwitch` flipped it (`BNT.ligatures` true->false in the plist, knob moved right->left); a color well opened the native `NSColorPanel` popover; the panel stayed open (461pt) through every in-window click (collapse guard works). The in-panel overlay is DELETED (-572 lines). See lessons 48-51. |
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

iTerm-style: tabs are square rectangles flush with the silhouette chin line, 1px
dividers between tabs, and a `+` button at the right end (a real click target —
sessions are dynamic). Geometry lives in **`tab_strip(h, w, count)`**, the single
source of truth shared by the renderer, the `+` rect, the `×` rect, hover, and
`select_at_screen_point`, so draw and click hit-testing cannot drift.

The strip insets by **`TAB_INSET` only (0.0)** — NOT by `chin_radius(h)`. It is
drawn under the silhouette clip, so the tapered chin corners crop the outermost
tabs (intended) and the leftmost tab / `+` button touch the true silhouette
edges. Insetting by the chin radius instead left an 18pt black gutter each side.

Chrome text (titles, `⌘<n>` hints) uses **`ui_font`** (San Francisco,
`.AppleSystemUIFont`) — never the grid's `monospace`. The title band is
`x + TAB_CLOSE_W + TAB_TITLE_PAD .. x + tab_w - TAB_HINT_W`, with the `×`
footprint and the hint reservation applied to **every** tab whether or not that
tab currently draws a `×`, so the centered title never re-centres on hover.
Vertical centering uses `CTFont::cap_height()`, not `ascent + descent`.

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

16. **A display change must be handled with `NSApplicationDidChangeScreen-
    ParametersNotification`, NOT a `ScrollWheel`/`ScreenChanged` monitor.**
    The panel is placed once at startup and AppKit does not re-centre a
    borderless panel on a resize, so it keeps its old frame origin and
    drifts off-centre — horizontally, because the y offset stays put and can
    coincidentally still look "at the top".
    The original code watched for a `ScrollWheel` event with subtype
    `ScreenChanged`. **That never fires**: reloaded and measured, on a
    1920→2048 resize the panel origin stayed exactly `(640, 623)` —
    `reposition` was never called, and the old "verified" note below was a
    false positive (a no-op run whose panel happened to still be centred).
    Fix: register one observer in `applicationDidFinishLaunching:` via
    `NSNotificationCenter::defaultCenter().addObserverForName_object_queue_
    usingBlock(...NSApplicationDidChangeScreenParametersNotification...)`
    (queue `None` = synchronous on the posting thread) and **hold the
    returned observer token in an ivar** — the center does not retain it, so
    dropping the token silently unregisters the block. The ScrollWheel
    monitor now unconditionally forwards `scroll_wheel`.
    Verified end-to-end with `screenresolution set` + CoreGraphics mode
    restore: 2560→x=960, 1920→x=640, 2048→x=704, all PASS. Note the user's
    real mode (1920×1243) is a HiDPI mode `screenresolution` cannot set —
    restore it via `CGConfigureDisplayWithDisplayMode` with
    `kCGDisplayShowDuplicateLowResolutionModes` (1920×1243 px=3840×2486).

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
    actual special keys. Plain printable chars fall through via
    `characters()`.

    CORRECTION (2026-10-11): the replacement table in this lesson was ALSO
    wrong — `8/13/3/26` are not Backspace/Return/Enter/Tab; in Carbon's
    virtual-key table (Events.h) they are the letters **C/W/F/7**. So typing
    `w` sent CR (a newline), `c` sent Backspace, `f` sent Enter, `7` sent
    Tab. The verified table (kbd "Show Virtual Keys" / Events.h): **Esc=53,
    Backspace(Delete)=51, Return=36, keypad Enter=76, Tab=48**, arrows
    123–126 (sent as ANSI `\x1b[A..D`, NOT the `\u{F700}+` NSFunctionKey
    chars `characters()` returns), ForwardDelete=117 (`\x1b[3~`).
    Verified end-to-end: typing `wcf7` now produces `wcf7`.

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
    overpaints the left-aligned `×`. The `×` footprint (`TAB_CLOSE_W` = `CLOSE_INSET
    + CLOSE_R + CLOSE_BUTTON_PAD`, i.e. the drawn BUTTON square, not just the
    glyph) plus `TAB_TITLE_PAD` is reserved on EVERY tab — see lesson 25 for why
    it must not be conditional on hover.

25. **Reserve the `×` gutter on EVERY tab, not just the active/hovered one.**
    Making `left_pad` conditional (`TAB_CLOSE_W + PAD` when active/hovered, else
    `PAD`) means the band widens the moment the pointer enters the strip, and a
    centered title then JUMPS sideways and retruncates. Symptom: titles visibly
    shift on hover. The reservation must be a per-tab constant so all tabs share
    one band; the `×` is drawn on top of empty space on the tabs that show it.

26. **`CGContextFillPath` CONSUMES the current path — `CGContext::clip` right
    after a `fill_path` clips an EMPTY path (a no-op).** The silhouette's rounded
    chin corners came from the FILL, not the clip, so nothing looked wrong as long
    as every child rect stayed inside the silhouette. Once the tab strip spanned
    the full width it painted straight over the corners and SQUARED them. Fix:
    `trace_silhouette(...)` again immediately before `CGContext::clip(ctx)`. This
    latent bug also left the grid and header unclipped during the open animation.
    Verify with a window-local alpha capture: the bottom row must inset the SAME
    amount on both sides and the corner alpha must staircase (`0,0,0,63,255…`).

27. **Center chrome text on `CTFont::cap_height()`, never `ascent + descent`.**
    San Francisco's `ascent` carries accent/diacritic headroom ~4pt above cap
    height, so centering the EM box rides every glyph ~6pt high in the tab —
    titles looked top-heavy while the vector `×` (drawn from rects) sat correctly
    centered. `baseline = y + (TAB_HEIGHT - cap) / 2.0` puts ink at panel-y
    208.5–218.5 (center 213.5) against the `×` center 214.2. Same expression in
    `draw_tab_label` and `draw_tab_hint` so title and hint cannot drift.

28. **`CTLineCreateTruncatedLine` with a NULL token draws NO ellipsis** — it
    just cuts the line. Pass a real `…` `CTLine` token shaped with the **SAME
    font** as the label: a mismatched-font token can be wider than `w`, and the
    call returns NULL then (which the code treats as "draw nothing"), silently
    blanking the title.
29. **alacritty 0.26's `Term` silently DROPS unknown OSCs.** `Term` implements
    `vte::ansi::Handler`, but its `osc_dispatch` only matches the codes it
    knows (0/1/2 title, clipboard, colors, …) and `return`s for the rest —
    logged at `debug!`, never surfaced. So OSC 133 (FinalTerm shell
    integration), OSC 7 / 1337 (cwd), OSC 6 (tab color), OSC 9;4 (progress)
    and OSC 7501 (program status) are invisible to the app no matter what the
    shell emits. `grep`ping the vendored crate for them returns nothing.

30. **Observe those sequences with a byte-transparent TEE, not a second read.**
    `EventLoop` owns the PTY read loop in the reader thread and cannot be
    borrowed from the main thread, and a `dup`'d master read STEALS bytes (the
    kernel hands each byte to exactly one reader). The sound shape (shipped in
    `TeePty`/`OscReader`): wrap `Pty`, and in `reader().read(buf)` perform the
    SAME single `libc::read` on the SAME master fd the inner `Pty` would have
    used, hand `buf[..n]` to a side `vte::ansi::Processor` (its `advance` is
    infallible and the `Mutex<OscState>` lock is taken briefly, NEVER across
    the read), then return `n` untouched. `EventLoop::pty_read` then feeds the
    identical slice to the real `Term`, so both parsers stay byte-synced.
    Guard it with a pipe round-trip test (`reader_is_byte_transparent_and_parses`)
    — drop/duplicate/reorder of a single byte corrupts the grid.

31. **`polling` must be a DIRECT dependency to implement `EventedReadWrite`.**
    The trait's method signatures name `polling::{Event, PollMode, Poller}` and
    alacritty does NOT re-export `polling` (it does re-export `vte`). Declaring
    `polling = "3.11"` against alacritty's `3.8.0` is safe: Cargo resolves ONE
    semver-compatible copy (verify in `Cargo.lock` — two copies would make the
    `Poller` types distinct and `register()` would silently never fire).

32. **Stock zsh emits only OSC 0/2 — OSC 133/1337 need an emitter.** The
    `ZDOTDIR` shim (`install_zsh_integration`) points `ZDOTDIR` at a
    pid-scoped temp dir whose `.zshrc`/`.zshenv` source the REAL user file
    (`$BNT_USER_ZDOTDIR`, restoring `ZDOTDIR` around the source) and then
    APPEND the hooks (`add-zsh-hook precmd/preexec`) — never overwrite the
    user's `precmd_functions`/`preexec_functions`. Removed on `Session::drop`;
    `kill -9` leaks the dir, so the app should tolerate a stale one.
    `133;D;<status>` on precmd + `1337;CurrentDir=$PWD` is what makes
    `~`-collapsed cwd titles and the nonzero-exit marker possible.

33. **A login-shell `#[test]` HANGS FOREVER.** `Session::new(..)` in a unit
    test spawns a real PTY; teardown (`Session::destroy` → reader returns →
    `Pty::Drop`) SIGHUPs the child AND WAITS, so the test never finishes and
    `cargo test` blocks indefinitely (burned ~25 min here across kills). Keep
    tests PURE (parser + `shorten_cwd` + the pipe transparency test); verify
    rendering with launch.sh + a window-local capture, never a PTY test.

34. **A fresh login shell paints at the BOTTOM of the grid, not the top.**
    Measuring ink in the upper band (panel_y 235–420) reads ZERO on a
    brand-new session and looks exactly like "the grid is blank". The prompt
    lands near panel_y 512–568 in a 420pt-tall panel. Measure the WHOLE
    panel_y range before concluding a grid regression — and re-check the
    window id via `swift scripts/bntw.swift` first, since a relaunch changes
    it (a stale id silently captures nothing).

35. **Closing a tab hung the whole app: `Session::Drop` joined the reader
    thread on the MAIN thread.** `Session.reader` is a `JoinHandle` whose
    payload owns `EventLoop` -> `TeePty` -> `Pty`, and `Pty::Drop` (alacritty)
    sends `SIGHUP` then BLOCKS in `Child::wait()`. `JoinHandle::Drop` JOINS —
    it runs that teardown on whichever thread drops it, and the `Session` vec
    lives in the view, so that is the main thread inside `close_tab`. Stack
    was `close_tab -> drop<Session> -> drop<JoinHandle> -> Pty::drop ->
    Child::wait` with every `sample` stuck there. Fix: make `reader` an
    `Option<JoinHandle>`, `take()` it in `Drop`, and `std::mem::forget` it —
    NEVER drop it. Both teardown paths (`close_tab`, `reap_dead_sessions`)
    already sent `Msg::Shutdown` BEFORE dropping, so the reader thread returns
    on its own and tears the PTY down on ITS OWN thread. Forgetting the handle
    is therefore non-blocking AND still correct teardown. (Two wrong turns to
    avoid: `let _ = spawn(...)` and `let x = spawn(...)` both DROP the spawned
    thread's handle = JOIN = block again; and `sample`'s "main-thread" block
    also shows the detached thread's `Child::wait`, so a `sample` leaf is NOT
    proof of a hang — trust the eprintln ordering instead.)

36. **Closing the LAST tab left the panel open (never collapsed).** `draw`
    spawns sessions whenever `sessions.is_empty()` and `h > CLOSED_HEIGHT + 1`,
    but the COLLAPSE animation passes through frames with `h > CLOSED_HEIGHT`,
    so `draw` respawned a tab mid-collapse and the panel never settled. Fix:
    gate the spawn sub-branch on the spring TARGET too —
    `ivars.spring.target_value() > CLOSED_HEIGHT + 1.0` — so a frame whose
    target is CLOSED_HEIGHT does not respawn. Keep the outer `h > CLOSED_HEIGHT`
    block intact (the grid must still paint during collapse); gate ONLY the
    `sessions.is_empty()` spawn.

37. **`INITIAL_TABS` is 1, not 3.** A real user opens one session; more tabs
    come from the `+` button or the Cmd+T hotkey. `MAX_TABS` caps the strip.

38. **Cmd+T spawns a tab via a SECOND Carbon hotkey, same handler.** `hotkey.rs`
    registers two `RegisterEventHotKey` chords (Option+Space id=1 toggle, Cmd+T
    id=2 new-tab) against ONE `InstallEventHandler`; the handler reads the
    `EventHotKeyID` and dispatches on `id`. Cmd+T keyCode is `kVK_ANSI_T = 0x11`
    (17) from Events.h — NOT the letter's position (lesson 20's table). Cmd+T is
    inert while closed because `spawn_tab` no-ops when the panel is closed.

39. **A layer-27 notch panel does NOT composite into `screencapture -l <id>`,**
    so a window-local capture of the CLOSED pill reads "fully transparent" even
    when the pill is painted correctly. The DESKTOP capture (`screencapture -x`
    full screen, or `computer_use_get_desktop_state`) is the user-visible truth:
    measure the dark run down the screen-center column (33pt = closed pill).
    Do not conclude "panel blank" from a transparent `-l` capture alone.

40. **Settings live in ONE `Settings` struct (`src/settings.rs`) carried as
    `ivars.settings: Cell<Settings>`** (`#[derive(Clone, Copy)]`, plain data —
    `Cell` needs `Copy`, so `font_family` is `&'static str` from a preset list,
    NOT a `String`; enum-ish knobs persist as an INDEX via `setInteger:forKey:`,
    the only safe typed setter in objc2-foundation 0.3.2). Read once per frame
    (`ivars.settings.get()`) and threaded into the silhouette fill, `draw_grid`,
    and each `Session::new` palette. The palette is threaded through `Session::new`
    (Term built on the main thread, moved into the reader thread) — NOT a
    `static Cell` (not Sync) or `thread_local!` (reader thread gets a different
    value).

41. **`NSUserDefaults` persistence gotchas.** (a) The plist is named after the
    **bundle id** (`local.bnt` → `~/Library/Preferences/local.bnt.plist`), NOT
    the app name or key prefix — grep the right file before concluding a write
    failed. (b) Writes are buffered and flushed on process EXIT, so a still-running
    app has no plist yet. (c) The defaults domain is keyed by the running
    **process name**, so a renamed copy (`/tmp/bnt-rt`) starts at DEFAULTS — use
    that to prove a true cross-process round-trip, since the SAME-name relaunch
    reads a stale in-memory cache, not the plist. (d) `key_name` applies the
    `BNT.` prefix EXACTLY once; `channel_key` returns a bare SUFFIX (`bg.r`).
    Double-prefixing (prefix in both) wrote `BNT.BNT.bg.r` and left chord keys
    single-prefixed, so `load` read a different key than `save` wrote. (e) The
    store is resolved by `defaults_store()`, which honours `BNT_DEFAULTS_SUITE`
    and falls back to `standardUserDefaults()`; verification runs MUST set it so
    a click pass cannot write real preferences (see lesson 52).

42. **The settings overlay is a click-driven UI, no text entry.** The panel
    can't take key focus while another app is frontmost, so values are edited
    with preset/stepper controls, not typed: `< value >` chevrons (a cycle row
    steps through a preset ring; a step row nudges by a unit), a toggle switch,
    and color wells. `settings_rows(settings)` is the SINGLE source of truth
    consumed by BOTH `draw_settings` and the click hit-test, so draw and hit-test
    cannot drift. `settings_region()` returns `(left,right,top,bottom)`; the
    bottom clamp is `bottom.min(top)` — `max` collapses the region to zero height
    and silently drops every row AND every click route.

43. **Chevron direction = which chevron was hit.** `apply_settings_row` computes
    `left_cx`/`right_cx` and steps `-1`/`+1` by `cx <= left_cx`. Comparing against
    `x_right - SETTINGS_CHEVRON` makes BOTH chevrons step back (the right chevron
    center is itself below that threshold). And a click that lands ON a row must
    `return` (keep the overlay open) — falling through to the "click off the rows
    closes settings" branch closed the overlay after EVERY control click.

44. **A row with NO wells must not index `wells`.** `wells.len() - 1` on an empty
    vec underflows (a chevron/toggle row has `wells == []`); guard with
    `row.wells.is_empty() || well >= row.wells.len() ? 0`. This was a hard panic
    (`attempt to subtract with overflow`) on the FIRST chevron click.

45. **Hotkey config = Carbon re-registration, not a new subsystem.** `hotkey.rs`
    keeps the two `EventHotKeyRef`s in a `static HOT_REFS: (AtomicUsize,AtomicUsize)`;
    `reregister_hotkeys` unregisters the old refs and registers the two chords
    from `settings` against the SAME `GetApplicationEventTarget()` (the delegate
    handler stays installed). `chord_mask` uses Carbon modifier bits (cmd=1<<8,
    shift=1<<9, option=1<<11, ctrl=1<<12, plus a 1<<0 sentinel when any modifier
    is set) so Option+Space=2049 and Cmd+Space=257 are DISTINCT. `CHORD_PRESETS`
    is the cycle ring; a chord change persists and takes effect at the next launch
    (verified: Cmd+Space toggle persisted, Option+Space inert on relaunch).

46. **The header bar is seamless with the terminal, not a separate block.** It
    is filled with `settings.background.rgb()` (NOT `TAB_FILL`) and there is no
    hard separator stroke. The visual separation is `draw_header_shadow`: a
    `SHADOW_FADE` (32pt) band of the SAME terminal-bg color, a two-stop
    `CGGradient` (opaque → alpha-0), drawn AFTER the grid so the top text rows
    fade away under the bar instead of meeting a line. CGGradient in objc2:
    `CGGradient::with_color_components(space, components.as_ptr(), stops.as_ptr(),
    n)` (device-RGB components, 4 floats/stop) — no CFArray needed; `CGColorSpace::
    new_device_rgb()` returns `Option<CFRetained<…>>` and the C wrappers want
    `Option<&T>`, so bind the CFRetained in a `match`/`as_ref` and pass `&*` —
    a `.map(|s| &*s)` closure returns a dangling borrow (E0515/E0597). Clip the
    band with `begin_path` + `add_rect` + `clip` (NOT `set_clip_path`), then
    `draw_linear_gradient` start=header-edge (opaque) end=below (clear) with
    `DrawsAfterEndLocation`. The band lives in the GRID branch only (settings
    replaces the grid, so no fade there).

47. **The `×` affordance is hover-only now** (the active tab no longer shows it):
    draw `if hovered == Some(index)`, hit-test `hovered.get() == Some(index)` —
    draw and hit-test must stay the same predicate or a click can hit an
    invisible `×`. The attention dot keeps `looking = selected || hovered` so a
    selected tab still never shows a dot in the `×` gutter. Hover itself is
    still not driver-verifiable (NSTrackingArea `mouseMoved:` never fires for
    synthetic pointer moves — same limitation as keystrokes), so the `×` path is
    verified by construction, not by a synthetic click.

48. **A separate native settings window beats the hand-drawn overlay** for every
    setting: the notch panel is a non-activating layer-27 `NSPanel` and can never
    take key focus, but a normal `NSWindow` is a different object entirely —
    `computer_use_list_windows` sees it, `get_window_state` returns a real AX tree,
    and native AppKit controls (`NSSlider`/`NSPopUpButton`/`NSSwitch`/
    `NSColorWell`/`NSSegmentedControl`) come free from `objc2-app-kit`, already a
    dependency. The overlay (~572 lines of hand-drawn chevrons/wells/rows) is
    deleted; the gear glyph + `gear_rect` stay as the affordance and now only
    open/refocus the window.

49. **`sel!(applyKnob:)` with a trailing colon PANICS at window build.**
    `objc2`'s `define_class!` asserts at runtime that the selector's argument
    count matches the Rust method's arity:
    `assertion failed: selector applyKnob: accepts 1 arguments, but function
    accepts 0`. The panic fires when the first control is built, killing the
    whole app — which looks like "the window never appears" rather than an
    error. A zero-argument handler needs the colon-FREE selector:
    `#[unsafe(method(applyKnob))]` + `sel!(applyKnob)`, in BOTH the method
    declaration and every `setTarget:`/`setAction:` site.

50. **A click inside the settings window COLLAPSED the open notch.** The global
    `LeftMouseDown` monitor receives clicks delivered to the frontmost app, and
    `click_at_screen_point` hit-tests them as OUTSIDE the panel frame ->
    `collapse()` (measured: 461pt -> 33pt pill). Fix: before EVERY dismissal
    branch, bail when the click point is inside
    `settings_window::window_frame(&ivars.settings_owner)`. The guard reads the
    LIVE frame, so a user drag of the movable window keeps it correct; a click
    in the window's titlebar is swallowed too (still our window — intended).

51. **`Accessory` activation policy does NOT block the settings window.**
    `bring_to_front` succeeds on it (the notch panel refuses), and a real
    foreground PIXEL click flips an `NSSwitch` (knob x 888-948 -> 924-984).
    What IS blocked: background synthetic input is refused
    (`minimized_or_hidden_window`) because the app is hidden, and `AXPress`
    does nothing — test these controls with pixel clicks, never AX. Typing is
    still unproven, so the UI is click-driven only: sliders, popup menus,
    switches, color wells, segmented controls, static labels — no editable
    `NSTextField`, no typed chord entry.

52. **A verification click pass wrote REAL preferences and bricked the hotkey.**
    Driving the settings window persists every change through `Settings::save`
    — so a pass that cycles the toggle chord wrote `BNT.toggle.mods = 257`
    (Cmd+Space) into the user's actual `local.bnt.plist`, and it was never
    reverted. Result: Option+Space no longer matched the registered chord, and
    Cmd+Space IS registered but is swallowed by Spotlight before the app sees
    it, so the app became unreachable by hotkey. Fix: `Settings::load`/`save`
    now resolve their store through `defaults_store()`, which honours
    `BNT_DEFAULTS_SUITE` — run verification as
    `BNT_DEFAULTS_SUITE=bnt-verify open -n BNT.app` so every read AND write
    lands in a throwaway domain. Unset (normal use) it is
    `standardUserDefaults()`. **Rule: never drive the settings window against
    the default suite.**

53. **A stale renamed binary steals the exclusive Carbon hotkey.** A
    `RegisterEventHotKey` chord is exclusive per process, so a second instance
    holding Option+Space wins it and the real app is silent. `launch.sh` only
    killed `BNT.app/Contents/MacOS/BNT`, so a `/tmp/bnt-rt` left over from the
    lesson-41c cross-process test kept running for HOURS and ate the chord
    (symptom: the hotkey "stopped working", Option+Space just typed a space).
    Fix: `launch.sh` now also `pkill -f bnt-rt`. Check for strays with
    `ps ax | grep -iE 'BNT|bnt-rt'` whenever the hotkey goes dead.

54. **Open width is a canvas problem, not a slider problem.** `WIDTH_MAX` was
    `PANEL_WIDTH` AND the panel window was created once at 640×620 and never
    resized — two ceilings numerically equal, so maxing the slider still painted
    a 640pt panel on a 1710pt display. Fix: the canvas = the notch display's
    logical width, sized ONCE at startup (`window.rs::canvas_size` threaded into
    `panel_origin`, `initWithContentRect`, `setFrame_display`, the view frame in
    `main.rs`, and a new `canvas: Cell<CGFloat>` ivar read everywhere geometry is
    drawn); `PANEL_WIDTH` survives ONLY as the `open_width` default and the
    `PANEL_WIDTH_FALLBACK`. Three traps found while landing it: (a) `grid_dims`
    must bake columns from `settings.open_width`, NEVER the live spring `w` —
    spawn runs on the first open frame while `w` is still the ~212pt notch width;
    (b) `click_at_screen_point` gained an x-gate from `ivars.width.get()` so a
    click beside the terminal INSIDE the widened canvas still collapses (the old
    code hit-tested the panel-frame rect, which is now full-screen); (c) the
    tracking-area ivar CANNOT be `Cell<Option<Retained<…>>>` (`get()`/`update`
    need `T: Copy`) — use the lesson-17 move-out/move-back `Cell<Vec<…>>`
    idiom with a `tracking_canvas` sentinel to reinstall on canvas change. The
    settings-window x centering must switch with `panel_origin`: with the canvas
    as its left edge, `(PANEL_WIDTH - WINDOW_W)/2` pins the window to the screen
    edge — it is `(frame.size.width - WINDOW_W)/2` now. Verified live by seeding
    `bnt-verify.plist` `BNT.openWidth` as a plist `<real>` (the `defaults` CLI
    stores strings; `get_sized` needs numbers): silhouette 1600pt, window-local
    alpha capture shows tab titles out to ~1746pt, closed pill unchanged at
    189pt/33pt. Live x-gate click test NOT run (see notes).

57. **Signing/release correction:** the earlier pipeline claims were incorrect.
    `xcrun --find notarytool` locates Xcode's CLI; notarization is automatable.
    `security import -T /usr/bin/codesign` is supported, and certificate/key
    deletion commands exist. A self-signed certificate is not an ad-hoc signature,
    but neither provides Apple Developer ID distribution trust. The existing
    signing scripts are unverified scaffolding with incorrect identity checks
    and must not publish releases yet. See `docs/project-state.md` for the
    authoritative current status and release blockers. The release workflow
    is manual/build-only until these are resolved.

56. **Renaming the bundle broke execution in three ways** (v0.1-alpha rename
    BNT.app→NotchQuake.app, id `local.bnt`→`local.notchquake`, binary `BNT`→
    `notchquake`): (a) `codesign --remove-signature NotchQuake.app` strips
    recursively — the INNER binary went unsigned and unsigned arm64 gets
    SIGKILL'd (Killed: 9) at exec; fix is just re-running launch.sh, which
    copies a fresh linker-signed (adhoc) build over. (b) `open -n` fails
    (launchd spawn 162/163) while the bundle-level signature is inconsistent
    with its contents — same root cause. (c) `pkill -f "MacOS/BNT"`-style
    patterns in a shell self-match the killer command; use a pattern that
    cannot match the pkill line (`grep -iE 'macos/(bnt|notchquake)$'` to
    inspect, explicit `kill <pid>` to act). The renamed app launches (launch.
    sh pid verified) and the Carbon hotkey still registers (Option+Space
    toggles) — process-name squatting (lesson 53) stays fine because launch.
    sh now kills `notchquake`. Old prefs under `local.bnt.plist` are orphaned
    ONE TIME by the bundle-id change; keys keep the `BNT.` prefix so a future
    prefs-domain migration is a simple file copy, and prefs reset is the
    accepted v0.1 cost.

55. **Slider max ≠ canvas had TWO sources, and the open grid never reflowed.**
    (a) `settings.rs::display_width()` took a MAX over every `NSScreen`
    (secondary display 1920) while the canvas is the NOTCH display
    (`window.rs::canvas_size` via `notch_screen`) — so the Open width rail could
    exceed the canvas and the silhouette got clipped at the screen edge. Fix:
    `display_width()` delegates to `crate::window::canvas_size(mtm)` — ONE
    source of truth feeding the slider rail, the `apply_knob_value` clamp AND
    the `load` clamp. The notch display can even CHANGE between launches
    (canvas measured 1710 one session, 1920 the next) so no constant is valid.
    (b) An OPEN panel did not reflow when the size knobs changed: `grid_dims`
    bakes rows/cols at spawn. Fix at `apply_settings` (`notch_view.rs`):
    while the spring TARGET is open, call `Session::resize(rows, cols, cell)`
    per live session with dims from the SAME `grid_dims` formula; `Session::
    resize` locks the `FairMutex<Term>` and calls alacritty's public
    `Term::resize(NotchSize{…})` (re-wraps lines, moves the vi cursor) then
    pushes `Msg::Resize(WindowSize)` through the SAME `EventLoopSender` wire
    `send_input` uses, so the reader thread's `EventLoop` runs `Pty::on_resize`
    → `TIOCSWINSZ`/SIGWINCH — never touch the PTY from the main thread (lesson
    35). Closed panel: skip (mid-collapse); it re-spawns on next open.
    Live drag of the width slider needs a REAL user click on the gear (menu-bar
    strip swallows synthetic clicks); verified by code audit + build gate.

## Repo / process rules

- macOS-only, arm64. `cargo build` is the gate; no LSP.
- `BNT.app/` is gitignored (built binary bundle — never commit). `/tmp/bnt-*.txt`
  debug writes were all stripped at ship time; none remain in `src/`.
- The plan doc (sibling worktree) is the spec; this AGENTS.md is the
  machine-specific how-to.