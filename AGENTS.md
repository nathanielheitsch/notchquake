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
| Tab bar (iTerm style): 3 tabs + `+` button, selected DARKER, dividers | ✅ | window-local alpha capture (open h=420, commit `8f23b1c`): square tabs, inactive fill `1A1A1A`, **active fill `141414` (darker, per iTerm's dark theme)**, separators/active borders `373737`, labels `FFFFFF`/`969696`, `⌘<n>` hint in iTerm blue accent `4372E0` on EVERY tab. Strip spans the FULL silhouette width (x 0–639.5 = silhouette alpha extent), chin corners cropped by the clip; `+` button square at `TAB_HEIGHT` (28pt, x 612–639.5). Titles start at the same offset on all tabs (35.0/35.3/35.2pt) so they do NOT shift on hover. Chrome text is San Francisco (`ui_font`), ink-centered on `cap_height`. |
| Click tab → switch PTY | ✅ verified | `mouseDown:` on the open panel selects the tab under the cursor; synthetic click on the middle tab moved selection from tab 0 (sel 64–435) to tab 1 (sel 434–805) |
| Option+Space global hotkey | ✅ verified | Carbon `RegisterEventHotKey` (no TCC grant); synthetic Option+Space toggles open then close |
| Click closed pill → open | ✅ verified | synthetic click on the pill toggles open (via `mouseDown:` for real clicks / global monitor fallback) |
| Scroll wheel → history | ✅ wired | ONE global `ScrollWheel` monitor → `scroll_wheel()`; alacritty `Scroll::Delta`, `BNT_SCROLLBACK` (default 10000). Wheel events proven to reach the handler (40× dy=-1 logged). Grid visibly moves only when the session has scrollback history — a fresh login shell has none, so `display_offset` clamps to 0. |
| Click outside panel → collapse | ✅ built | `click_at_screen_point` collapses an open panel on an outside click; idempotent `expand`/`collapse` so both click paths land once. |
| Header bar (notch height, open state) | ✅ verified | desktop capture: grid text starts at y≈50pt = notch 33 + GRID_TOP_PAD 22; the notch-height block above it is reserved (empty for now, buttons later). `row_rect`/`draw_grid` take a `header` param; `grid_dims` reserves the same inset. |
| Dynamic tabs: `+` spawns | ✅ verified | synthetic click on the `+` grew the strip from 3 to 4 tabs (labels reflowed, tab_w shrank). `sessions` is a live `Cell<Vec<Session>>`; `tab_strip(h,w,count)` is the single source of truth for renderer + hover + click. |
| Tab close via `×` (selected or hovered) | ✅ verified | click on the `×` of the SELECTED tab fires `close_tab` (the affordance is drawn on hovered OR selected, and the hit-test accepts either). Verified lifecycle: 1 tab → `+` → 2 → ⌘T → 3 → `×` → 2 tabs; closing the LAST tab collapses to the 33pt pill. Closing a live tab no longer hangs the app (see lesson 35). |
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

## Repo / process rules

- macOS-only, arm64. `cargo build` is the gate; no LSP.
- `BNT.app/` is gitignored (built binary bundle — never commit). `/tmp/bnt-*.txt`
  debug writes were all stripped at ship time; none remain in `src/`.
- The plan doc (sibling worktree) is the spec; this AGENTS.md is the
  machine-specific how-to.