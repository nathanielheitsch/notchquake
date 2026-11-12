# UI core contracts (as actually implemented)

Audience: the next UI worker (native settings toolbar, precision controls, ANSI
2x8 grid, live preview, shortcut recorder). Everything below exists in the tree
today; anything aspirational is explicitly listed under *Limitations*.

Files: `src/settings.rs`, `src/effects.rs`, `src/input.rs`, `src/hotkey.rs`,
`src/font.rs`, `src/terminal.rs`, `src/render.rs`, `src/notch_view.rs`,
`src/settings_window.rs`. `src/window.rs` is untouched (panel canvas).

## 1. Settings: proposal → validation → registration → accept → coalesce → save

`Settings` is plain data that is `Clone` (not `Copy`; `font_family: String`).
`SettingsSnapshot = Rc<Settings>` is the only type the view hands out.

| Step | API | Notes |
| --- | --- | --- |
| validate fields | `Settings::validate(canvas) -> Result<(), SettingsError>` | Pure. Finite-and-ranged `open_height`, `font_size` 8–32, `finished_notify_secs` 0–3600; `open_width` must be finite and at least `NOTCH_WIDTH.min(canvas)` **only** — the ceiling is applied when painting, so a stored width above a shrunk display is not a settings error; non-empty family without NUL; `scrollback_lines <= 100_000`; every color channel finite in `0..=1`; then `validate_chords` on the **effective** pair. Font *availability* is **not** validated here. |
| validate chord policy | `validate_toggle_chord`, `validate_local_chord`, `validate_chords(toggle, local)`, `validate_chord_change(chord, changed, effective_toggle)`, `chord_status(chord, ChordKind, effective_toggle)` | `local_reserved()` covers Cmd+native/local chords (0, 6–9, 12, 13, 18–23, 25, 26, 28, 43, 48, 49, 50, 53), Cmd+T, Ctrl+Space. Toggle needs Option/Cmd/Control. `validate_chord_change` constrains only a chord the proposal actually changes — a merely *stored* unusable value is exempt — and rejects a duplicate of the effective toggle. `chord_status` is the user-facing report; the load-time guards decide when it applies. |
| raw vs effective | `Settings::effective_chords() -> (Chord, Chord)`, `Settings::effective_open_width(canvas) -> CGFloat` | What the runtime registers and paints, derived from the raw stored values without changing them. Unusable stored toggle → `DEFAULT_TOGGLE`; unusable stored new-tab chord, or one equal to the effective toggle → the first usable `LOCAL_CHORD_FALLBACKS` entry. Stored width above the canvas → the canvas. `preferences()` always re-encodes the RAW values. |
| phase gate | `SettingsState::validate_proposal(proposal, phase, canvas)` | Field validation plus the chord policy: a **chord change is only acceptable in `EditPhase::Commit`** (previews must not re-register); a changed toggle must pass `validate_toggle_chord`, and a changed new-tab chord must pass `validate_chord_change` against the proposal's effective toggle. A chord that is merely stored and unusable rejects nothing, so a legacy value cannot lock out font/color/history edits. |
| font resolve | `font::resolve_monospace(family, size) -> Result<ResolvedFont, String>` | Strict: resolved `family_name()` must equal the request (case-insensitive) and carry `TraitMonoSpace`; cell metrics must be positive and finite. Called by `NotchView::apply_settings`, which rewrites `after.font_family` to the resolved family. |
| hotkey | `hotkey::change_toggle(mtm, chord)` | Runs **before** acceptance; failure → `SettingsError::Hotkey` and nothing is stored. See §3. |
| accept | `SettingsState::accept(proposal, phase, now) -> bool` | Sets `current = Rc::new(proposal)`, arms a coalesced resize (`now+0.085`) and save (`now+0.300`); `Commit` also `flush`es. Returns whether anything changed. |
| due checks | `resize_due(now, open)`, `save_due(now) -> Option<SettingsSnapshot>`, `require_resize(now)`, `flush(now)`, `saved(snapshot)` | Latest-value coalescing, not one resize per slider tick. Resize is deferred while the panel is closed (`open == false` keeps the deadline pending). |
| persist | `Settings::save() -> Result<(), PersistenceError>` | Submission only — `NSUserDefaults` reports no durable/atomic result, so success means "submitted". Reached only from `NotchView::pump_settings`, which `apply_settings` calls with `flush = phase == Commit`: a **commit** therefore arms the deadline and submits synchronously inside that call, while continuous controls (slider drags, previews) defer to the `now+0.300` deadline drained by `tick` and `flush_settings`. Never called from a control callback. |
| load | `Settings::load()`, `Settings::load_with_status() -> (Settings, Vec<String>)` | Per-field decode with warnings; malformed values fall back individually (never NaN, never a hard failure). Chords decode **raw** and are never rewritten: an unusable stored chord is reported here and handled at runtime by `effective_chords`. `openWidth` decodes raw too (floor-clamped only) and reports an above-canvas value instead of persisting the clamp. |

Rejection ordering in `NotchView::apply_settings(after, phase) ->
Result<SettingsSnapshot, SettingsError>`: validate → strict font resolve →
toggle re-register → `accept` → font cache invalidation / geometry retarget →
`pump_settings(phase == Commit)`. A rejected proposal therefore produces **no
preference write, no resize, no chord change**. `flush_settings()` is the
explicit commit path (used on settings-window close and
`applicationWillTerminate:`).

### Persistence compatibility contract (unchanged, do not rename)

- Keys are `BNT.<name>` via `key_name()`, applied **exactly once**; `channel_key()`
  returns a bare suffix (`bg.r`, `toggle.key`, `ansi.3.g`). `defaults_store()`
  honours `BNT_DEFAULTS_SUITE` (empty value = refuse, non-UTF-8 = error) and
  otherwise `standardUserDefaults()`. Never verify against the default suite.
- Written: `BNT.openHeight|openWidth|fontSize|notifySecs` (doubles),
  `BNT.ligatures|bellEnabled` (bools), `BNT.fontFamily` (index, 0 when the name
  is custom), `BNT.fontFamilyName` (text, authoritative on read),
  `BNT.cursorShape`, `BNT.scrollbackLines` (preset index), `BNT.historyLines`
  (exact lines, authoritative on read, capped 100_000), `BNT.bg/fg/cursor.{r,g,b}`,
  `BNT.ansi.<i>.{r,g,b}`, `BNT.toggle.{key,mods}`, `BNT.newTab.{key,mods}`.
- Chord masks are Carbon bits (`chord_mask`: cmd 1<<8, shift 1<<9, option 1<<11,
  ctrl 1<<12, plus a 1<<0 sentinel) and `chord_of()` is their exact inverse.
- `Settings::preferences()` is the single encoder, shared by save, load and the
  pure decode test (`Settings::from_preferences(&prefs, canvas)`).

## 2. Palette, font metrics, grid geometry, session seams

- `Settings::palette() -> Palette { background, foreground, cursor_color, ansi[16] }`.
  `Session::snapshot(&Palette)` resolves colors per frame, so palette edits apply
  to **live** sessions. `palette_or()` keeps an OSC/SGR-written entry in
  `Term::colors()` ahead of the settings palette; `default_color()` supplies the
  16 named colors, named defaults 256–268, the 6x6 cube (16–231) and the
  grayscale ramp (232–255, `8 + (i-232)*10`).
- `font::ResolvedFont { font, family, cell }`, `font::metrics(&CTFont) ->
  (advance, ascent+descent+leading)`, `font::installed_monospace_families()`
  (CoreText-only enumeration, filtered by the strict resolver, lower-case sorted).
- `NotchView::ensure_font` caches font + cell in ivars; `font_invalidate` drops
  both when a font knob changes, and the grid resize is deferred to a settings
  deadline — so a wheel event can arrive with an empty cache. `scroll_wheel`
  re-derives the metric through `ensure_font(&current_settings())` (the same call
  `draw` makes) and the pure helper `scroll_lines(cell_height, pts)`, which
  returns `(0, pts)` for a non-finite or non-positive metric instead of dividing
  by zero; nothing on that path unwraps the cache, and the next draw still
  resolves the selected font. `grid_dims(cell, header, &settings, canvas)` is the
  single spawn/resize formula (columns from
  `settings.effective_open_width(canvas) - 2*GRID_PAD`, never from the live spring
  width; rows from `open_height - header - GRID_TOP_PAD - TAB_HEIGHT
  - 2*TAB_INSET`; scrollback from settings).
- `cell_pixels(cell, backing_scale)` converts points to device pixels;
  `terminal::GridSize::new(rows, columns, width, height)` (ceil, clamped to
  `u16`) is the value compared for resize equality — so a pixel-only change
  (font size, scale) still reaches the PTY. `Session::resize` re-wraps `Term`
  under the `FairMutex` and sends `Msg::Resize` through the existing channel;
  the main thread never touches the PTY. `Session::set_scrollback` applies
  `Config.scrolling_history` live via `set_history_options`, which suppresses
  only that call's synchronous title echo (`Event::Title`/`ResetTitle`) with an
  RAII reset, so history edits cannot clear a shell-set tab title.
- Repaint signal: `Session::take_dirty()` — set by `Event::Wakeup` (which the
  reader thread emits only when bytes were parsed or on an empty poll cycle, so
  it is never a 60Hz repaint by itself) and by title/history edits. `tick`
  repaints only when a spring is flying or the open panel's sessions/titles
  changed. `Event::Wakeup` deliberately does not touch `activity`/`dead`.
- Attention/notification flags (`take_bell`, `take_finished`, `has_activity`,
  `take_attention`) are unchanged; `drain_notifications` suppresses banners only
  for a tab that is both selected **and** visible (`is_open()`).

## 3. Shortcut / input routing

- **Global**: exactly one Carbon chord — the **effective** toggle. `main.rs` and
  `NotchView::apply_settings` register `Settings::effective_chords().0`, never the
  raw stored chord. `hotkey::register_hotkeys(mtm, view, chord) -> Vec<String>`
  installs the handler once and returns warnings; a chord that fails to register
  falls back to `Option+Space` at runtime and **never rewrites the stored
  preference**. `hotkey::change_toggle` re-validates with `validate_toggle_chord`,
  so an unusable replacement can never become active.
- `hotkey::Registration<B>` (backend-abstracted, `Backend::register/unregister`)
  registers the candidate *before* releasing the old handle, keeps the old
  registration on any failure, and retains handles whose cleanup failed so their
  events are ignored. Each registration carries a monotonic `generation`; the
  Carbon handler dispatches only when `accepts_event(signature, generation,
  active)` matches, so a stale or rolled-back registration can never toggle.
- **New tab is terminal-local**, never global: `input::classify` maps the
  **effective** local chord (`effective_chords().1`, default `Cmd+T`) inside the
  panel, so a stored chord that cannot be routed does not silently disable the
  fallback.
- `input::classify(context, key_code, text, modifiers, new_tab) -> Route` takes
  the effective local chord from `NotchView::handle_key`:
  `Route::PassThrough` unless `terminal_open && panel_is_key && event_is_panel`.
  Command chords (`Cmd+W` close tab, `Cmd+V` paste, `Cmd+C` copy-unavailable,
  `Cmd+1…8` select tab — Carbon key codes 18,19,20,21,23,22,26,28 — modifier-heavy
  chords, `Cmd+,`/`Cmd+Q` and unknown chords) never produce bytes; `Cmd+,`/`Cmd+Q`
  pass through to AppKit/menu routing. Control chords become `b & 0x1f`;
  specials by key code (Esc 53, Delete 51, Return 36 / keypad 76, Tab 48, arrows
  123–126 as ANSI, ForwardDelete 117); NSFunctionKey chars and empty text pass
  through; a single ASCII char with Option becomes `ESC c`.
- `input::paste_bytes(text, bracketed)` normalizes CR/CRLF and strips `ESC` from
  bracketed content so pasted data cannot close the `200~…201~` wrapper. The
  bracket state comes from `Session::bracketed_paste()`.
- `NotchView::handle_key(&NSEvent) -> bool` reports consumption; the local
  monitor in `main.rs` returns `null_mut()` only when consumed and otherwise
  returns the original event, so native windows, menus and the rest of the
  desktop keep their keystrokes.
- Click routing: `click_at_screen_point` bails on any visible, non-miniaturized
  window of this app (settings window, `NSColorPanel`, About) before the
  settings-window frame guard and before every dismissal branch; the panel's own
  frame/silhouette gates are unchanged. `SettingsOwner::window_frame()` reports
  `None` for a hidden or miniaturized window.
- Menu routing: `main.rs::install_menus` installs a main menu — app menu (About,
  `Settings…` = `Cmd+,` targeting the delegate's `showSettings:`, Quit) and an
  Edit menu (Undo/Redo/Cut/Copy/Paste/Select All/Close Window). This is what
  makes native text editing and clipboard work in the settings window, and what
  `Route::PassThrough` chords (`Cmd+,`, `Cmd+Q`, `Cmd+W` when a native window is
  key) land on. The panel's own `Cmd+C`/`Cmd+V` are consumed first only while the
  panel is key, so the Edit menu is unaffected there.
- `NotchView::select_tab(index)` ignores an out-of-range index against
  `tab_count`; `effects::selection_after_removal(selected, &removed)` is the one
  identity-preserving remap used by both `close_tab` and `reap_dead_sessions`
  (`None` ⇒ collapse).

## 4. Geometry, motion, Reduce Motion

- `effects::displayed(value, min, max)` clamps for display and maps non-finite to
  `min`; physics may overshoot, the painted silhouette cannot.
- `effects::motion_frame(height, width, delta, reduce_motion, notch, bounds) ->
  (h, w)` — one pure decision for both springs; `reduce_motion` snaps both to the
  clamped targets before advancing. `animation_active(h_settled, w_settled)` is
  the only "keep redrawing" predicate, used by `drawRect:` and `tick`.
- `NotchView::is_open()` reads the **spring target** (`> notch_height + 1`), so
  open/closed decisions do not depend on the current frame height. `set_target`
  retargets both springs without resetting position or velocity, clamps cached
  height/width, snaps when Reduce Motion is on, and only claims key focus on a
  closed→open transition (`makeKeyWindow`, never `activate`).
- `set_notch` re-reads `backingScaleFactor` and retargets to the open height when
  already open. `reposition` updates canvas, window frame and notch, re-targets
  the width spring at `effective_open_width(new_canvas)`, and records a status
  line when that clamps the stored preference — a display change never writes
  preferences, and the raw width survives for a later larger display.
- `update_reduce_motion` is driven by
  `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification` (observer token
  retained in `AppDelegateIvars.motion_observer`).
- `last_time` is reset to 0 when both springs settle, so the next frame after
  idle contributes a zero delta instead of a giant one.

## 5. Effect scheduler (`src/effects.rs`)

`SettingsState { current, saved, resize_at, save_at, status }` plus the free
functions above: pure latest-value deadlines with no AppKit types, no callbacks,
no timers. `NotchView::pump_settings(flush)` is the only consumer; it runs from
`tick`, from `set_target` on open, and from `apply_settings`. Deadlines are
absolute `CFAbsoluteTimeGetCurrent()` values; a rejected proposal touches none.

## 6. Rendering seams available to the UI worker

- `render::draw_grid(ctx, grid, cursor, font, cell_w, cell_h, height, header,
  settings, x0, usable)` — unchanged, panel-space (rows hang from `PANEL_HEIGHT`).
- `render::paint_grid_at(ctx, bounds, grid, cursor, &ResolvedFont, &Settings)` —
  origin-independent adapter that clips, fills the background and re-anchors the
  same grid renderer inside `bounds`. **Currently no caller**: it exists so a
  native preview `NSView` can paint a real grid without a PTY.
- `render::sample_grid(&Settings) -> Vec<Vec<CellSnapshot>>` — representative
  lines plus the ANSI 16 ramp for that preview. Also currently uncalled.
- Both are pure (no PTY, no main-thread requirement) and consume the same shaped
  grid path as live sessions.

## 7. Settings window (temporary client)

`src/settings_window.rs` remains a valid client of the seams above and is the
UI to replace, not to extend:

- Control callbacks call `NotchView::apply_settings(value, phase)` and on
  rejection `record_status`, `readback_control(knob, control, &current)` and
  `show_error`. Phase is `Preview` while a slider drag event is not
  `LeftMouseUp`, otherwise `Commit` (so chords can only change on commit).
- Popups use `setPullsDown(false)` so the accepted selection stays visible;
  `NSColorWell` (Minimal style) now has the cell as `target:` with
  `sel!(applyKnob)` as `action:`, so wells actually submit their color.
- `KNOB_FONT_FAMILY` is populated from `font::installed_monospace_families()`
  (`family_names()` thread-local) instead of a fixed list.
- `open_or_focus`, `window_frame`, `show_error`, `apply_knob_value`,
  `knob_value_from_sender` and `SettingsOwner` are the public surface;
  `width_max()`, `srgb_color()` and `chord_index()` are exported helper
  functions, not part of the client surface. `family_names()`, `readback_control`,
  `about_rows`, `ansi_slot_titles` and the `SettingsCell` target are **private
  helpers — do not call them as public API**. `SettingsCell` targets are kept
  alive by `SettingsOwnerIvars::cells`; controls are retained by the grid tree;
  raw pointers are main-thread-only by construction.
- `show_error` uses `NSAlert::runModal`. **Do not call it during
  `applicationDidFinishLaunching:`** — it blocks before `app.run()`. Startup
  warnings are `eprintln`-logged and recorded in `SettingsState::status`
  (`NotchView::settings_status()`); surfacing them in the window is open work.

## 8. Known limitations / not implemented here

- No native settings toolbar, precision steppers, ANSI 2×8 grid editor, live
  preview view, or shortcut recorder. `paint_grid_at`/`sample_grid` are the only
  preview plumbing that exists.
- `CHORD_PRESETS` is still the chord ring (`[Chord; 7]`: Option+Space, Option+`,
  Ctrl+`, Option+Return, Cmd+Return, Option+Backspace, Cmd+T), and a chord
  changes only on commit. Chord names for a non-preset stored chord still render
  as `Key <n>` and `chord_index` falls back to 0, so a legacy stored chord reads
  back as the first popup preset while `effective_chords` — not the popup —
  decides what is registered; `about_rows` reports the effective pair. Arbitrary
  chord recording is not implemented (needs an editable recorder plus
  validation/re-register; no Input Monitoring grant is required for a *local*
  chord while the panel is key).
- `show_error` uses `NSAlert::runModal`, so a rejected knob change in the current
  window blocks that callback until dismissed. A replacement UI should surface
  rejection non-modally (inline status) rather than per-click.
- `SettingsState::status` is append-only and is not displayed anywhere yet.
- `render::paint_grid_at`/`sample_grid` are unused today, and `render::draw_grid`
  still exceeds clippy's argument-count limit (pre-existing).
- Clipboard paste only reads `NSPasteboardTypeString`; there is no selection model
  in the panel, so `Cmd+C` is consumed as `CopyUnavailable`.
- Strict Clippy is **not** green: 50 pre-existing lint errors remain (14
  `collapsible_if`, 21 needless deref/auto-deref, 6 `single_match`, 1
  `draw_grid` argument count, 1 `let_and_return`, 1 doc-comment spacing, 1
  deprecated `msg_send!` comma form, and 5 `window.rs` unused-import/dead-code
  items). This lane fixed the only lint its own new code introduced and removed
  35 baseline lints as a side effect of the rewritten modules; no unrelated
  cleanup was applied.
- Tests are pure: parser/pref decode/font resolve/grid dims/selection/spring/
  input routing. No `Session::new`, no PTY, no default-preferences I/O.
- No live UI verification was performed in this lane (no launch, no desktop
  capture); everything above is established by code inspection plus the CLI gates.
