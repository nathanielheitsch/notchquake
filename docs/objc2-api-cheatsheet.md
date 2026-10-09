# Verified objc2 / alacritty API cheat sheet (checked against the local crate sources)

All of these were confirmed to exist in the crate versions pinned in Cargo.toml
(objc2 0.6.5, objc2-app-kit / core-graphics / core-text / core-foundation /
foundation 0.3.2, alacritty_terminal 0.26.0, block2 0.6.2). Do NOT invent
other signatures — if a name is not listed here, grep the crate source before
using it.

Registry root: `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`

## Confirmed: `define_class!` CAN subclass NSView

```rust
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct NotchView;

    unsafe impl NSObjectProtocol for NotchView {}

    impl NotchView {
        #[unsafe(method(updateLayer))]
        fn update_layer(&self) { /* self.ivars() is reachable here */ }
    }
);
```
Notes:
- Method impls must be inside `impl NotchView { ... }` inside the macro.
- `#[unsafe(method_family = none)]` is NOT supported together with
  `#[unsafe(method(...))]` inside `define_class!` — omit it.
- `updateLayer` (no args) is the whole-layer draw; use it, not `drawRect:`.
- `objc2_app_kit::NSView` is `#[thread_kind = MainThreadOnly]`, so the subclass
  must be MainThreadOnly too.

## Window creation (NSPanel)

```rust
let panel = unsafe {
    NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)),
        NSWindowStyleMask::Borderless            // Borderless = 0
            | NSWindowStyleMask::NonactivatingPanel // 1<<7
            | NSWindowStyleMask::UtilityWindow      // 1<<4
            | NSWindowStyleMask::HUDWindow,         // 1<<13
        NSBackingStoreType::Buffered,
        false,
    )
};
```
Setters that exist on `Retained<NSPanel>` (all `unsafe`):
`setLevel(NSWindowLevel)`, `setCollectionBehavior(NSWindowCollectionBehavior)`,
`setFloatingPanel(bool)`, `setOpaque(bool)`, `setHasShadow(bool)`,
`setMovable(bool)`, `setReleasedWhenClosed(bool)`, `setAlphaValue(CGFloat)`,
`setContentView(Option<&NSView>)`, `setBecomesKeyOnlyIfNeeded(bool)`,
`setWorksWhenModal(bool)`, `setFrameOrigin(NSPoint)`, `setFrame(NSRect,bool)`;
non-unsafe: `frame() -> NSRect`, `alphaValue()`, `contentView() -> Option<Retained<NSView>>`,
`orderFrontRegardless()`.
Constants: `NSMainMenuWindowLevel = 24` (so level = `NSMainMenuWindowLevel + 3`).
`NSWindowCollectionBehavior::CanJoinAllSpaces | Stationary | FullScreenAuxiliary`.

NOT bound in objc2-app-kit: `animationRunLoop`, `setNeedsDisplayOnBoundsChange:`.
If needed, declare with `extern_methods!` + `#[unsafe(method_id(animationRunLoop))]`
on `NSWindow` (returns `Option<Retained<NSRunLoop>>` — NSRunLoop itself is NOT
bound in objc2-app-kit either; see below).

## CADisplayLink (objc2-quartz-core, feature-gated)

Bound: `displayLinkWithTarget_selector(target: Option<&AnyObject>, selector: &DynBlock<dyn Fn()>) -> Retained<CADisplayLink>`,
`addToRunLoop_forMode(runloop: &NSRunLoop, mode: &NSRunLoopMode)`,
`removeFromRunLoop_forMode`, `setFrameInterval`, `setPreferredFramesPerSecond`,
`invalidate`, `new() -> Retained<Self>`, `init(this: Allocated<Self>)`.
`NSRunLoop` / `NSRunLoopMode` are NOT bound in objc2-app-kit 0.3.2 → you must
declare them yourself with `extern_class!`/`extern_methods!` if you go this route.

## Simpler animation route (preferred, zero extra bindings)

Keep spring state in the view's ivars. In `updateLayer`, advance the spring using
wall-clock (`now - t0`), draw at the current height, and if not settled call
`view.setNeedsDisplay(true)` at the end of the draw so AppKit re-invokes
`updateLayer` next pass (self-sustaining ~60Hz). `NSView::setNeedsDisplay(bool)`
IS bound. If this stalls in practice, fall back to the CADisplayLink route.

## NSEvent hotkey + click

```rust
let block = block2::DynBlock<dyn Fn(std::ptr::NonNull<NSEvent>)>;  // type alias exists
// build via block2::RcBlock::new(...) then convert, or pass &Block directly
NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
    NSEventMask::KeyDown,
    &block,
) -> Option<Retained<AnyObject>>;   // keep the return alive!
NSEvent::removeMonitor(monitor)     // unsafe
```
- `NSEventMask::KeyDown = 1<<10` (bitflags).
- `NSEvent::modifierFlags() -> NSEventModifierFlags`; `Option = 1<<19`.
- `NSEvent::keyCode() -> c_ushort` (Space = 32), `isARepeat() -> bool`,
  `charactersIgnoringModifiers() -> Option<Retained<NSString>>`.
- `addLocalMonitorForEventsMatchingMask_handler` takes
  `&DynBlock<dyn Fn(NonNull<NSEvent>) -> *mut NSEvent>` (return null to keep event).
- Mouse: `NSEventMask::MouseDown`/`MouseUp`; `locationInWindow() -> NSPoint`
  (window-base coords, bottom-left origin); `buttonNumber()`, `clickCount()`.

## Geometry / drawing (objc2-core-graphics)

`CGRect`/`CGPoint`/`CGSize` come from `objc2_core_foundation` (struct with
`origin`/`size`; `CGRect::new(CGPoint, CGSize)`, `CGRect::standardize`, `min`,
`mid`, `max`). `objc2_foundation` re-exports `NSPoint/NSRect/NSSize` (same layout).

Free functions / methods (all take `Option<&CGContext>` first arg, `unsafe`):
`CGContext::save_g_state(c)`, `restore_g_state(c)`, `translate_ctm(c,x,y)`,
`scale_ctm(c,sx,sy)`, `begin_path(c)`, `add_path(c, Option<&CGPath>)`,
`add_rect(c, CGRect)`, `add_curve_to_point(c, cp1x,cp1y,cp2x,cp2y,x,y)`,
`move_to_point(c,x,y)`, `add_line_to_point(c,x,y)`, `close_path(c)`,
`clip(c)`, `clip_to_rect(c, CGRect)`, `fill_path(c)`, `stroke_path(c)`,
`fill_rect(c, CGRect)`, `set_fill_color_with_color(c, Option<&CGColor>)`,
`set_fill_color_space`, `set_line_width`, `set_text_position(c,x,y)`,
`set_text_matrix(c, CGAffineTransform)`.

`CGPath::with_rounded_rect(rect: CGRect, w: CGFloat, h: CGFloat, transform: *const CGAffineTransform) -> CFRetained<CGPath>` (unsafe).
`CGMutablePath::add_rounded_rect(...)`.
`CGColor::new_generic_rgb(r,g,b,a) -> CFRetained<CGColor>` (safe, 0..1).
`CGColor::new_generic_gray(gray, alpha)`.

## CoreText (objc2-core-text + objc2-core-foundation)

```rust
let s: CFRetained<CFString> = unsafe { CFString::with_c_string(None, c"Fira Code", CFStringEncodingUTF8) };
// CFStringEncodingUTF8 lives in objc2_core_foundation (CFString.rs, kCFStringEncodingUTF8)
let url = CFURL::with_file_system_path(None, &path_string, CFURLPathComponentStyle, false);
CTFontManagerRegisterFontsForURL(url, CTFontManagerScope::Process, &mut err_ptr) -> bool;
let font: CFRetained<CTFont> = unsafe { CTFont::with_name(&name, size, ffi::nullptr) };
let bold = unsafe { font.copy_with_symbolic_traits(size, nullptr,
    CTFontSymbolicTraits::TraitBold, CTFontSymbolicTraits::TraitBold) }; // Option<...>
let astr: CFRetained<CFMutableAttributedString> = CFAttributedString::new(None, 0); // then set string + attrs
let line: CFRetained<CTLine> = unsafe { CTLine::with_attributed_string(&astr_as_CFAttributedString) };
unsafe { CTLine::draw(&line, ctx) };  // or CTLineDraw(line, ctx) free fn
CTLine::typographic_bounds(ctx, &line, *mut CGFloat, *mut CGFloat, *mut CGFloat) // unsafe
CTLine::bounds_with_options(&line, CTLineBoundsOptions) -> CGRect
CTLine::glyph_count(&line) -> CFIndex
```
- `CFAttributedString::new(alloc, max_length)` creates a MUTABLE empty one; to
  set its text use `CFAttributedStringReplaceString` / `replace_string(...)`
  (bound at CFAttributedString.rs:754) or `new_copy` from a CFString
  (line 296/115 — check which is which before use).
- `CFAttributedStringSetAttribute` / `CFAttributedString::set_attribute` exists
  (line 799) for attaching a `kCTFontAttributeName` dictionary attribute.
- `CTFont::copy_with_attributes(size, matrix, Option<&CTFontDescriptor>)`.
- `CTFont::ascent()`, `descent()`, `leading()`, `size()` — all unsafe.
- `CTFontSymbolicTraits::TraitBold = 1<<1` (bitflags in objc2_core_text CTFontTraits).
- `CTFontManagerScope::Process = 1`.

## alacritty_terminal 0.26.0 embedding

```rust
use alacritty_terminal::event_loop::EventLoop;
use alacritty_terminal::event::WindowSize;   // {num_lines,num_cols,cell_width,cell_height}
use alacritty_terminal::tty::Options;        // {shell,working_directory,drain_on_exit,env}
use alacritty_terminal::tty::Shell;          // Shell::new(program, args)
use alacritty_terminal::tty::new as pty_new; // pty_new(&opts, window_size, id) -> Result<Pty>
use alacritty_terminal::term::Term;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::event::VoidListener; // implements EventListener

let terminal = Arc<FairMutex<Term<VoidListener>>>; // Term::new(config, &dimensions, VoidListener::new())
let loop_ = EventLoop::new(terminal, VoidListener::new(), pty, drain_on_exit, false).unwrap();
let sender = loop_.channel();      // EventLoopSender {send(Msg)}; Msg = Input|Resize|Shutdown
let joined = loop_.spawn();         // thread "PTY reader"; returns JoinHandle<(EventLoop, State)>
```
- `Dimensions` is a trait; `(usize lines, usize cols)` impl exists ONLY under
  `#[cfg(test)]` → define your own `struct Size {..} impl Dimensions`.
- `Term::new(config: Config, dimensions: &D, event_proxy: T) -> Term<T>`;
  `Config::default()` is fine.
- `Term::grid() -> &Grid<Cell>`; `Grid::display_iter()` iterates the viewport
  top-left → bottom-right; `grid.display_offset()` = scrollback offset.
- `Cell { c: char, fg: Color, bg: Color, flags: Flags, extra: Option<Arc<CellExtra>> }`
  with `Color ∈ {Named(NamedColor), Spec(Rgb), Indexed(u8)}`, `Rgb{r,g,b}`,
  `Flags` bitflags: INVERSE 1, BOLD 2, ITALIC 4, UNDERLINE 8, WIDE_CHAR 0x20,
  DIM 0x80, HIDDEN 0x100, STRIKEOUT 0x200.
- `Term::colors() -> &Colors` where `Colors = [Option<Rgb>; 269]`; palette
  indices: 0..16 ANSI, 16..232 cube, 233..256 gray, **256 = Foreground,
  257 = Background, 258 = Cursor, 259..267 dim, 267 bright fg, 268 dim bg**.
  `Term::set_options(Config)` does NOT set colors; there is no public
  `set_colors`/`resolve_color` — resolve `Named` yourself against your own
  palette table (do NOT rely on `Term.colors()` being populated; it starts
  `[None; 269]` and is only written by OSC/SGR color ops).
- `FairMutex::lock()` also takes a lease lock — copy the cells you need out
  under one short lock scope, release, then draw.
- `EventLoopSender` is `Clone` but NOT `Send`/`Sync` → keep all senders in one
  main-thread struct, never cross a thread boundary.
- `tty::new` already resolves `getpwuid(getuid()).pw_shell` and spawns it with
  `-l -i` and `TERM=xterm-256color` + IUTF8; passing `Options.shell = None`
  gives exactly the required login-shell behavior.
- `tty::Pty` owns the child; `Pty::child()` gives the `Child` (pid). SIGTERM on
  quit: use `nix`/`libc::kill` on that pid.

## AppKit runtime bootstrap (from objc2's own examples/hello_world_app.rs)

```rust
let mtm = MainThreadMarker::new().unwrap();
let app = NSApplication::sharedApplication(mtm);
app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
app.activateIgnoringOtherApps(true);   // required when launched unbundled
app.run();
```
The delegate is a `define_class!` class implementing
`unsafe impl NSApplicationDelegate for X { #[unsafe(method(applicationDidFinishLaunching:))] fn did(&self, notification: &NSNotification) {...} }`.
`NSApplicationActivationPolicy::Regular` is set via
`app.setActivationPolicy(...)` (bound on NSApplication).

## Not bound (do not use without declaring): NSRunLoop, NSRunLoopMode,
NSWindow.animationRunLoop, NSView mouseDown:/mouseUp:, setNeedsDisplayOnBoundsChange.
