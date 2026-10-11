//! One terminal session: an `alacritty_terminal` state machine driven by a
//! login-shell PTY, plus a snapshot of its cell grid the renderer can paint.
//!
//! The PTY is spawned exactly once, at startup. The grid size is fixed for
//! the whole process — it is computed from the OPEN panel height and the font
//! cell metrics — so the reveal/collapse animation never resizes the PTY and
//! nothing reflows (the panel's clip wipes the grid instead).
//!
//! Shell resolution: the required `getpwuid(getuid()).pw_shell` rule is
//! implemented here and handed to `alacritty_terminal` as
//! `Options.shell = Shell::new(pw_shell, ["-l", "-i"])`. (`alacritty_terminal`
//! would resolve `pw_shell` itself when `shell` is `None`, but it spawns
//! `login -flp … /bin/zsh -fc …`, which is not the documented `-l -i` argv.)

use libc::getpwuid;
use libc::getuid;
use libc::passwd;
use libc::c_void;
use libc::pid_t;
use libc::proc_name;
use libc::proc_pidpath;
use libc::tcgetpgrp;

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::collections::HashMap;
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event, EventListener, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, State};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::{Colors};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, new as pty_new, Options, Pty, Shell};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb as AnsiRgb};
use alacritty_terminal::vte::{Parser as VteParser, Perform};
use polling::{Event as PollingEvent, PollMode, Poller};

/// Per-session OSC state harvested by the PTY tee's side-channel parser.
///
/// `alacritty_terminal`'s `Term` does NOT override `vte::ansi::Handler::osc_dispatch`,
/// so the vte default DROPS every OSC it does not know (verified: the default
/// arm handles only 0/2/4/8/10/11/12/22/50/52/104/110/111/112). That means
/// OSC 6 (tab color), 133 (command lifecycle), 1337 (cwd/host), 9;4 (progress)
/// and 7501 (program status) never reach us from `Term`. The tee parses the
/// same PTY bytes with our own tiny `Handler` whose only override is
/// `osc_dispatch`.
#[derive(Default)]
pub struct OscState {
    /// `OSC 1337;CurrentDir=<path>` (and the zsh hook's `$PWD`): the shell's
    /// own, prompt-accurate cwd. Preferred over the OSC title for the tab
    /// label because it is short and never carries the `user@host` prefix.
    cwd: Option<String>,
    /// `OSC 133;D;<status>`: the just-finished command's exit status. A
    /// nonzero status tints the tab title red (T2a).
    exit_status: Option<i32>,
    /// `OSC 6;1;bg;<r|g|b>;brightness;<0-255>` accumulated across the three
    /// per-channel sequences into one RGB; `None` after `*;default` (T1b).
    tint: Option<(f64, f64, f64)>,
    /// Partial tint while only some of R/G/B have arrived (OSC 6 sends one
    /// channel per sequence). Cleared once the trio is complete.
    tint_parts: (Option<u8>, Option<u8>, Option<u8>),
    /// `OSC 7501;state=<s>:app=<name>:msg=<base64>`: the latest program
    /// status. `state` is one of idle|working|done|blocked|error (T2b).
    program: Option<(String, String, String)>,
    /// `OSC 9;4;<st>;<pr>`: progress. st 0 clear, 1 set %, 2 error,
    /// 3 indeterminate, 4 paused; pr is 0-100 (required only for st=1) (T2c).
    progress: Option<(i32, i32)>,
    /// Wall clock at `OSC 133;C` (command start), used to measure how long the
    /// finished command ran; `133;D` takes it (T2a).
    command_started: Option<std::time::Instant>,
    /// A just-finished command not yet reported: `(exit_status, seconds)`.
    /// Taken exactly once by the view's notifier, so a long-running command
    /// notifies its OWNER at most once (T2a).
    pending_finish: Option<(i32, f64)>,
}

impl OscState {
    /// Fold one OSC 6 color channel into the tab tint (T1b).
    fn set_tint_channel(&mut self, channel: &[u8], value: Option<u8>) {
        match channel {
            b"red" => self.tint_parts.0 = value,
            b"green" => self.tint_parts.1 = value,
            b"blue" => self.tint_parts.2 = value,
            _ => return,
        }
        // iTerm's OSC 6 sends one channel per sequence, so only fold into a
        // final color once all three have landed; a partial mix would flash
        // the tab an arbitrary color between sequences.
        if let (Some(r), Some(g), Some(b)) = self.tint_parts {
            self.tint = Some((r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0));
            self.tint_parts = (None, None, None);
        }
    }
}

/// The tee's OSC-only perform target. Every other `Perform` method keeps
/// vte's default no-op body: the real `Term` (fed the SAME bytes by the
/// EventLoop) owns the grid, so this parser must never mutate terminal state
/// — it only copies out the OSCs `Term` drops.
///
/// NOTE: raw OSC is only reachable through the LOW-LEVEL `vte::Parser` +
/// `vte::Perform` pair. `vte::ansi::Handler` (what `Term` implements) has NO
/// `osc_dispatch` of its own — `vte::ansi::Processor` funnels OSC through its
/// internal `Perform` impl, which only understands a fixed set of codes and
/// `debug!`s the rest. So the tee drives `vte::Parser` directly.
struct OscHandler<'a> {
    state: &'a Mutex<OscState>,
    /// The same flag `Event::Title` sets. `133;D` sets it too, so the blue
    /// activity dot works even for an rc that never sets an OSC title (T2a).
    /// Two atomic stores of `true` are idempotent, so there is no
    /// double-fire.
    activity: &'a AtomicBool,
}

impl Perform for OscHandler<'_> {
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() || params[0].is_empty() {
            return
        }
        // A poisoned lock here would mean a panic on the render thread; the
        // parse path must never panic or block, so treat poisoning as "drop
        // this event" rather than unwrapping.
        let mut state = match self.state.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match params[0] {
            // Command lifecycle. A/B/C are prompt/line markers we do not need
            // to render; D carries the exit status (T2a).
            b"133" => {
                if let Some(kind) = params.get(1) {
                    // C = a command started (preexec); D = it finished
                    // (precmd), carrying the exit status. The gap between B
                    // and C is the user typing, so timing C..D measures the
                    // COMMAND's runtime, which is what the >5s notification
                    // rule wants.
                    if *kind == b"C" {
                        // A new command is running: drop the previous command's
                        // failed marker so a stale red does not persist across
                        // commands in the same tab.
                        state.exit_status = None;
                        state.command_started = Some(std::time::Instant::now());
                    } else if *kind == b"D" {
                        let status = params.get(2)
                            .and_then(|s| std::str::from_utf8(s).ok())
                            .and_then(|s| s.trim().parse::<i32>().ok())
                            .unwrap_or(0);
                        state.exit_status = Some(status);
                        // Only queue a finish notification when a command
                        // actually STARTED (a preceding `133;C`). Every prompt
                        // emits `133;D` (via precmd), so without this guard the
                        // first prompt of each freshly spawned tab would post a
                        // spurious "completed" banner.
                        if let Some(started) = state.command_started.take() {
                            state.pending_finish = Some((status, started.elapsed().as_secs_f64()));
                        }
                        self.activity.store(true, Ordering::SeqCst);
                    }
                }
            }
            // iTerm2 shell-integration cwd (T1c), e.g.
            // `OSC 1337;CurrentDir=/Users/me/projects`.
            b"1337" => {
                if let Some(param) = params.get(1) {
                    if let Some(rest) = param.strip_prefix(b"CurrentDir=") {
                        if let Ok(path) = std::str::from_utf8(rest) {
                            if !path.is_empty() {
                                state.cwd = Some(path.to_string());
                            }
                        }
                    }
                }
            }
            // Tab color (T1b): `OSC 6;1;bg;<r|g|b>;brightness;<N>` or the
            // reset `OSC 6;1;bg;*;default`.
            b"6" => {
                if params.len() >= 5 && params[2] == b"bg" {
                    let channel = params[3];
                    if params[4] == b"default" || channel == b"*" {
                        state.tint = None;
                        state.tint_parts = (None, None, None);
                    } else if params.len() >= 6 && params[4] == b"brightness" {
                        let value = std::str::from_utf8(params[5]).ok()
                            .and_then(|s| s.trim().parse::<u8>().ok());
                        state.set_tint_channel(channel, value);
                    }
                }
            }
            // Program status (T2b): `OSC 7501;state=<s>:app=<name>:msg=<b64>`.
            b"7501" => {
                if let Some(body) = params.get(1) {
                    if let Ok(text) = std::str::from_utf8(body) {
                        let (mut state_s, mut app, mut msg) = (String::new(), String::new(), String::new());
                        for field in text.split(':') {
                            if let Some(v) = field.strip_prefix("state=") {
                                state_s = v.to_string();
                            } else if let Some(v) = field.strip_prefix("app=") {
                                app = v.to_string();
                            } else if let Some(v) = field.strip_prefix("msg=") {
                                msg = v.to_string();
                            }
                        }
                        if !state_s.is_empty() {
                            state.program = Some((state_s, app, msg));
                        }
                    }
                }
            }
            // Progress (T2c): `OSC 9;4;<st>;<pr>`.
            b"9" if params.get(1) == Some(&b"4".as_slice()) => {
                let st = params.get(2).and_then(|s| std::str::from_utf8(s).ok())
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(-1);
                let pr = params.get(3).and_then(|s| std::str::from_utf8(s).ok())
                    .and_then(|s| s.trim().parse::<i32>().ok())
                    .unwrap_or(-1);
                state.progress = if st < 0 { None } else { Some((st, pr)) };
            }
            _ => {}
        }
    }
}

/// A byte-transparent tee around the PTY's read half.
///
/// `EventLoop` owns the single read loop and reads through
/// `EventedReadWrite::reader()`; a second `read()` on a dup'd master would
/// STEAL bytes from it (the kernel hands each byte to exactly one reader).
/// So the wrapper keeps the ONLY reader and forwards each byte to our OSC
/// parser on the way to the caller's buffer.
///
/// Byte-transparency argument: `read` performs the same `libc::read` on the
/// SAME master fd the inner `Pty` would have used (`OscReader.fd ==
/// inner.file().as_raw_fd()`), returns the kernel's byte count untouched, and
/// tees an exact copy of `buf[..n]` into the parser BEFORE returning. The tee
/// can neither drop, duplicate, reorder nor block: `advance` is infallible
/// (no `Result`, no re-read of the fd), and the only shared mutable state is
/// the `Mutex<OscState>` lock, which the parser takes briefly and NEVER holds
/// across the `read` — a poisoned lock just drops that OSC, it never unwinds
/// into the read path. Nothing else reads the master fd, so no byte is lost.
pub(crate) struct TeePty {
    inner: Pty,
    reader: OscReader,
}

impl TeePty {
    fn new(inner: Pty, state: Arc<Mutex<OscState>>, activity: Arc<AtomicBool>) -> Self {
        let fd = inner.file().as_raw_fd();
        Self { inner, reader: OscReader { fd, parser: VteParser::new(), state, activity } }
    }
}

/// The one thing `EventLoop` reads through: a raw-fd read that also feeds the
/// OSC parser. Holds the master fd directly (not a `File`) so it does not
/// need to borrow the inner `Pty`'s file — the trait's `reader()` returns a
/// `&mut Self::Reader` stored in the wrapper, which a self-referential borrow
/// could not satisfy.
pub(crate) struct OscReader {
    fd: RawFd,
    parser: VteParser,
    state: Arc<Mutex<OscState>>,
    activity: Arc<AtomicBool>,
}

impl Read for OscReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // SAFETY: `buf` is a valid writable slice of `buf.len()` bytes and
        // `fd` is the live master descriptor captured from the inner `Pty`
        // (which outlives this reader). `read(2)` writes at most `len` bytes
        // and returns the count (or -1). This is byte-for-byte what the inner
        // `Pty`'s own `File::read` would do, so the terminal sees no change.
        let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut c_void, buf.len()) };
        if n < 0 {
            return Err(io::Error::last_os_error())
        }
        let n = n as usize;
        if n > 0 {
            // Hand the EXACT bytes the caller is about to receive to the vte
            // processor; the real `Term` gets the same slice via the EventLoop
            // afterwards, so both parsers stay byte-synchronized.
            let mut handler = OscHandler { state: &self.state, activity: &self.activity };
            self.parser.advance(&mut handler, &buf[..n]);

        }
        Ok(n)
    }
}

impl EventedReadWrite for TeePty {
    type Reader = OscReader;
    type Writer = File;

    unsafe fn register(
        &mut self,
        poll: &Arc<Poller>,
        interest: PollingEvent,
        poll_opts: PollMode,
    ) -> io::Result<()> {
        // SAFETY: delegated verbatim to the inner `Pty`'s own impl, with the
        // same arguments; the wrapper does not change the registration.
        unsafe { self.inner.register(poll, interest, poll_opts) }
    }

    fn reregister(
        &mut self,
        poll: &Arc<Poller>,
        interest: PollingEvent,
        poll_opts: PollMode,
    ) -> io::Result<()> {
        self.inner.reregister(poll, interest, poll_opts)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.inner.deregister(poll)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut Self::Writer {
        self.inner.writer()
    }
}

impl EventedPty for TeePty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.inner.next_child_event()
    }
}

impl OnResize for TeePty {
    fn on_resize(&mut self, window_size: WindowSize) {
        self.inner.on_resize(window_size);
    }
}

/// A minimal `EventListener` that keeps only the OSC (`ESC ] 0 ; …`) title of
// the session, so the tab strip can label each PTY with what is running in
// it. Crucially, it also routes `Event::PtyWrite` (the terminal's responses
// to shell queries like `ESC [ 6 n` cursor-position reports) back into the
// PTY by sending them as `Msg::Input`. Without this, the shell blocks
// forever waiting for a response and never draws its prompt (the grid stays
// blank even though the shell sets a title). `Arc<Mutex<...>>` is
// `Send + Sync`, satisfying the `Send + 'static` bound `EventLoop` requires.
#[derive(Clone)]
pub struct TitleListener {
    title: Arc<std::sync::Mutex<Option<String>>>,
    sender: Arc<std::sync::Mutex<Option<EventLoopSender>>>,
    /// Set when the shell re-sets its OSC title (`Event::Title`), i.e. a
    /// command just finished, so the tab strip can mark a tab whose command
    /// finished while the user was NOT looking at it (the blue activity dot).
    /// Cleared when that tab is selected.
    activity: Arc<AtomicBool>,
    /// Set when the child shell exits. Shared (via `Clone`) with the copies
    /// held by `Term` and the reader thread's `EventLoop`, so the main thread
    /// can observe the exit through any clone (`Session::dead()`).
    dead: Arc<AtomicBool>,
    /// Set by `Event::Bell` (a raw BEL byte, `Term::bell`). Taken once by the
    /// view's notifier so one bell posts one notification (T1a).
    bell: Arc<AtomicBool>,
}

impl TitleListener {
    pub fn new() -> Self {
        Self {
            title: Arc::new(std::sync::Mutex::new(None)),
            sender: Arc::new(std::sync::Mutex::new(None)),
            dead: Arc::new(AtomicBool::new(false)),
            activity: Arc::new(AtomicBool::new(false)),
            bell: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Wire the PTY writer channel once the `EventLoop` exists. Called after
    /// `EventLoop::new`; later `PtyWrite` events are forwarded through it.
    pub fn set_sender(&self, sender: EventLoopSender) {
        if let Ok(mut g) = self.sender.lock() {
            *g = Some(sender);
        }
    }

    pub fn title(&self) -> Option<String> {
        self.title.lock().ok().and_then(|g| g.clone())
    }

    /// The shared activity flag, so the tee's `133;D` arm sets the SAME flag
    /// `Event::Title` sets (one place the view reads).
    pub fn activity_handle(&self) -> Arc<AtomicBool> {
        self.activity.clone()
    }
}

impl EventListener for TitleListener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(t) => {
                if let Ok(mut g) = self.title.lock() {
                    *g = Some(t);
                }
                // A title change means the shell just redrew its prompt —
                // zsh's precmd sets the OSC title once per prompt, so this
                // fires once per COMMAND, which is iTerm's "command finished
                // while you weren't looking" rule (and the source of the
                // `~ (-zsh)` / `π - projects (pi)` labels). NOT `Wakeup`: the
                // event loop also sends `Wakeup` on every EMPTY poll cycle
                // (`event_loop.rs`: `events.is_empty() && rx.peek().is_none()`
                // → `send_event(Event::Wakeup)`), i.e. 60Hz while a session
                // lives, which would light every idle tab's dot permanently.
                self.activity.store(true, Ordering::SeqCst);
            }
            Event::ResetTitle => {
                if let Ok(mut g) = self.title.lock() {
                    *g = None;
                }
            }
            // A raw BEL from the shell/program (`Term::bell`). Set the flag;
            // the view's tick takes it and posts at most one notification per
            // bell, and only for a tab the user is NOT looking at (T1a).
            Event::Bell => {
                self.bell.store(true, Ordering::SeqCst);
            }
            // Route terminal responses (DSR/DA cursor-position reports, etc.)
            // back to the PTY so the shell's startup queries are answered and
            // the prompt actually renders.
            Event::PtyWrite(text) => {
                if let Ok(g) = self.sender.lock() {
                    if let Some(sender) = g.as_ref() {
                        let _ = sender.send(Msg::Input(Cow::from(text.into_bytes())));
                    }
                }
            }
            // The child shell exited. `Event::Exit` (sent by `Term::exit()`
            // AFTER the `drain_on_exit` final pty_read) is the death signal;
            // gating on it instead of `ChildExit` (which the event loop
            // sends BEFORE the drain, `event_loop.rs:263-267`) lets the
            // session's last bytes land in the grid before the reap. Never
            // treat `Wakeup` as death — it fires continuously while a
            // session lives.
            Event::Exit => {
                self.dead.store(true, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

/// Grid dimensions for the notch terminal.
///
/// `alacritty_terminal`'s `Dimensions` trait is what `Term::new` consumes;
/// the crate's own `(usize, usize)` impl is `#[cfg(test)]`-only, so we
/// provide our own.
pub struct NotchSize {
    screen_lines: usize,
    columns: usize,
}

impl Dimensions for NotchSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// One cell of the rendered grid, copied out under the terminal lock.
pub struct CellSnapshot {
    pub character: char,
    pub foreground: (f64, f64, f64),
    pub background: (f64, f64, f64),
}

/// A live session: the terminal state machine, the input channel to the PTY,
/// and the PTY reader thread.
pub struct Session {
    terminal: Arc<FairMutex<Term<TitleListener>>>,
    listener: TitleListener,
    shell: String,
    /// The PTY's master descriptor and its child pid, captured at spawn
    /// before the `Pty` moves into the reader thread's `EventLoop` (which
    /// the main thread cannot borrow). Both feed `foreground_program`.
    master_fd: RawFd,
    child_pid: pid_t,
    sender: EventLoopSender,
    reader: JoinHandle<(EventLoop<TeePty, TitleListener>, State)>,
    /// Oscillator state harvested by the tee (cwd, exit status, tint, program
    /// status, progress). Shared with the reader thread's parser, so every
    /// accessor locks briefly.
    osc: Arc<Mutex<OscState>>,
    /// The per-session zsh shim dir (`ZDOTDIR`), removed on drop so it never
    /// outlives the session. `None` for a non-zsh shell.
    shim_dir: Option<PathBuf>,
    rows: usize,
    columns: usize,
}

/// Monotonic id mixed into the shim-dir name so parallel sessions in one
/// process cannot collide (the process id alone is shared).
static NEXT_SHIM_ID: AtomicUsize = AtomicUsize::new(0);

/// Install the zsh shell-integration EMITTER and return
/// `(ZDOTDIR, real-zdotdir)`. Stock zsh emits no OSC 133/1337, so the tee has
/// nothing to parse without this; the snippet is iTerm2's own approach.
///
/// `ZDOTDIR` relocates ALL four zsh startup files, so the shim dir carries a
/// loader for each (`.zshenv`/`.zprofile`/`.zshrc`/`.zlogin`) that sources the
/// user's REAL file from `$BNT_USER_ZDOTDIR`. That preserves login-shell
/// behaviour exactly (alacritty's argv is still `-l -i`); only the hooks in
/// the `.zshrc` loader are new. Returns `None` for a non-zsh shell or when no
/// temp dir can be written — the session then runs exactly as before.
fn install_zsh_integration(shell: &str) -> Option<(String, String)> {
    let base = Path::new(shell).file_name()?.to_str()?;
    if base != "zsh" {
        return None
    }
    // The dir zsh WOULD have used: ZDOTDIR when set, else HOME.
    let real = std::env::var("ZDOTDIR").ok()
        .filter(|d| !d.is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|d| !d.is_empty()))?;
    let id = NEXT_SHIM_ID.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("bnt-zdotdir-{}-{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).ok()?;

    // The hook emitter, sourced at the TOP of the `.zshrc` loader so it
    // wraps whatever PROMPT/hooks the user's rc installs. `add-zsh-hook`
    // APPENDS to `precmd_functions`/`preexec_functions` (it never overwrites
    // them), and the `__BNT_SHELL_INTEGRATION` guard makes re-sourcing a
    // no-op. Sequence syntax (iTerm2's, verified): OSC 133;D;<status>,
    // OSC 1337;RemoteHost=<user>@<host>, OSC 1337;CurrentDir=<pwd>,
    // OSC 133;A (prompt start), OSC 133;C (command start), OSC 133;B
    // (prompt end, appended zero-width to PROMPT).
    let integration = "\
        if [[ -o interactive ]] && [[ -z \"$__BNT_SHELL_INTEGRATION\" ]]; then\n\
        \x20 __BNT_SHELL_INTEGRATION=1\n\
        \x20 autoload -Uz add-zsh-hook\n\
        \x20 __bnt_precmd() {\n\
        \x20   local __bnt_status=$?\n\
        \x20   printf '\\033]133;D;%s\\007' \"$__bnt_status\"\n\
        \x20   printf '\\033]1337;RemoteHost=%s@%s\\007' \"$USER\" \"$HOST\"\n\
        \x20   printf '\\033]1337;CurrentDir=%s\\007' \"$PWD\"\n\
        \x20   printf '\\033]133;A\\007'\n\
        \x20   return $__bnt_status\n\
        \x20 }\n\
        \x20 __bnt_preexec() { printf '\\033]133;C\\007'; }\n\
        \x20 add-zsh-hook precmd __bnt_precmd\n\
        \x20 add-zsh-hook preexec __bnt_preexec\n\
        \x20 PROMPT=\"${PROMPT}%{\"$'\\033]133;B\\007'\"%}\"\n\
        fi\n";

    // One loader per startup file, so relocating ZDOTDIR hides none of the
    // user's config. The loader saves $ZDOTDIR, points it at the REAL dir
    // while sourcing, then restores it — so user configs that read $ZDOTDIR
    // see the value they would have seen without us. A readable check means a
    // missing real file is simply skipped. `.zshrc` also loads the hooks.
    for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
        // The hooks go AFTER the real rc, so the user's PROMPT/hooks are
        // already installed and our `%{…133;B…%}` append wraps them instead
        // of being overwritten by a theme (`add-zsh-hook` appends either
        // way, so hook order is unaffected).
        let hooks = if name == ".zshrc" { integration } else { "" };
        let loader = format!(
            "# BNT shell-integration shim: load the real {name} from $BNT_USER_ZDOTDIR.\n\
             if [[ -r \"$BNT_USER_ZDOTDIR/{name}\" ]]; then\n\
             \x20 __bnt_zdotdir=$ZDOTDIR\n\
             \x20 ZDOTDIR=$BNT_USER_ZDOTDIR\n\
             \x20 source \"$BNT_USER_ZDOTDIR/{name}\"\n\
             \x20 ZDOTDIR=$__bnt_zdotdir\n\
             \x20 unset __bnt_zdotdir\n\
             fi\n\
             {hooks}"
        );
        if std::fs::write(dir.join(name), loader).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return None
        }
    }

    Some((dir.to_string_lossy().into_owned(), real))
}

impl Drop for Session {
    /// Remove the per-session zsh shim dir. The `ZDOTDIR` dir is private to
    /// this session (its name carries the shim id), so nothing else reads it;
    /// leaving it behind would accumulate one dir per tab in `/tmp`.
    fn drop(&mut self) {
        if let Some(dir) = self.shim_dir.as_ref() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

impl Session {
    /// Spawn the PTY once and start the PTY reader thread. `scrollback` is
    /// the history cutoff in lines (0 disables scrolling history).
    pub fn new(rows: usize, columns: usize, cell_width: f64, cell_height: f64, scrollback: usize) -> Self {
        let shell = login_shell().expect("the current user has a login shell");
        let listener = TitleListener::new();

        // Shell integration (the EMITTER): alacritty's own shell argv stays
        // exactly `-l -i`; we only point zsh's startup-file lookup at a
        // per-session shim dir via `ZDOTDIR`. Stock zsh emits no OSC 133/1337,
        // so without this the tee parses nothing and T1c/T2a cannot fire.
        let mut env = HashMap::new();
        let shim_dir = match install_zsh_integration(&shell) {
          Some((zdotdir, real)) => {
              env.insert("ZDOTDIR".to_string(), zdotdir.clone());
              env.insert("BNT_USER_ZDOTDIR".to_string(), real);
              Some(PathBuf::from(zdotdir))
          }
          None => None,
        };

        let options = Options {
            shell: Some(Shell::new(shell.clone(), ["-l", "-i"].map(|arg| arg.to_string()).to_vec())),
            working_directory: None,
            drain_on_exit: true,
            env,
        };

        let window_size = WindowSize {
            num_lines: rows as u16,
            num_cols: columns as u16,
            cell_width: cell_width.ceil() as u16,
            cell_height: cell_height.ceil() as u16,
        };

        let pty = pty_new(&options, window_size, 1).expect("the PTY could not be spawned");

        // Grab the pty's raw handles BEFORE `pty` is moved into the event
        // loop: the `EventLoop` owns the `Pty` inside the reader thread, so
        // the main (render) thread could never reach them afterwards. The
        // descriptors stay valid for the whole session lifetime — `Pty`'s
        // `Drop` closes them, and a dead session is reaped.
        let master_fd = pty.file().as_raw_fd();
        let child_pid = pty.child().id() as pid_t;

        // Tee the PTY: the single read loop reads through `TeePty`, whose
        // reader parses the same bytes into `osc` as they pass through. See
        // `TeePty` for why this is the only sound way to see the OSCs that
        // `Term`'s handler drops.
        let osc = Arc::new(Mutex::new(OscState::default()));
        let tee = TeePty::new(pty, osc.clone(), listener.activity_handle());

        // The scrollback cutoff: alacritty's `Config.scrolling_history` is the
        // history size in lines; the grid keeps total = screen + history.
        let mut config = Config::default();
        config.scrolling_history = scrollback;

        let terminal = Arc::new(FairMutex::new(Term::new(
            config,
            &NotchSize { screen_lines: rows, columns },
            listener.clone(),
        )));

        let loop_ = EventLoop::new(terminal.clone(), listener.clone(), tee, true, false)
            .expect("the PTY event loop could not be created");

        // Wire the PTY writer so terminal responses (cursor-position reports,
        // device-status/attribute replies) are routed back to the shell. The
        // EventLoop owns the channel; hand the listener a clone of it.
        listener.set_sender(loop_.channel());

        Session {
            terminal,
            listener,
            shell,
            master_fd,
            child_pid,
            sender: loop_.channel(),
            reader: loop_.spawn(),
            osc,
            shim_dir,
            rows,
            columns,
        }
    }

    /// Write user input (a keystroke, pasted text, …) to the PTY.
    pub fn send_input(&self, bytes: Vec<u8>) {
        let _ = self.sender.send(Msg::Input(Cow::from(bytes)));
    }

    /// Scroll the display through the scrollback history. `lines` > 0 shows
    /// older content (scroll up), < 0 returns toward the live viewport. The
    /// grid clamps to the history size, so the cutoff is the `scrollback`
    /// passed to `new`.
    pub fn scroll_display(&self, lines: i32) {
        let mut terminal = self.terminal.lock();
        terminal.scroll_display(Scroll::Delta(lines));
    }

    /// Copy the visible grid out under one short lock.
    ///
    /// Rows are returned top to bottom; each row has exactly `columns` cells.
    pub fn snapshot(&self) -> Vec<Vec<CellSnapshot>> {
    let terminal = self.terminal.lock();
    let grid = terminal.grid();
    let offset = grid.display_offset();
    let colors = terminal.colors();

    let rows = (0..self.rows).map(|line| {
        (0..self.columns).map(|column| {
                let cell = &grid[Line((offset + line) as i32)][Column(column)];
                let inverse = cell.flags.intersects(Flags::INVERSE);
                let (fg, bg) = (resolve(colors, cell.fg), resolve(colors, cell.bg));
                CellSnapshot {
                    character: cell.c,
                    foreground: if inverse { bg } else { fg },
                    background: if inverse { fg } else { bg },
                }
            }).collect()
        }).collect();

        rows
    }

    /// The cursor position in viewport coordinates, or `None` when it is
    /// hidden (a scrolled-back display offset).
    pub fn cursor(&self) -> Option<(usize, usize)> {
        let terminal = self.terminal.lock();
        let grid = terminal.grid();
        if grid.display_offset() != 0 {
            return None
        }
        Some((grid.cursor.point.line.0 as usize, grid.cursor.point.column.0))
    }

    /// Whether the child shell has exited (`Event::ChildExit` /
    /// `Event::Exit` landed on the shared listener flag).
    pub fn dead(&self) -> bool {
        self.listener.dead.load(Ordering::SeqCst)
    }

    /// Ask the reader thread to exit: `Msg::Shutdown` makes
    /// `drain_recv_channel` return false, the event loop breaks, and the
    /// `EventLoop` (which owns the `Pty`) drops inside the thread —
    /// `Pty`'s `Drop` impl SIGHUPs the child and waits for it. Harmless on
    /// an already-dead session.
    pub fn destroy(&self) {
        let _ = self.sender.send(Msg::Shutdown);
    }

    /// The tab label, in iTerm's `<directory> (<program>)` form (measured off
    /// the user's iTerm screenshot: `~ (-zsh)`, `π - projects (pi)`). The
    /// first half is the OSC title the shell sets; the parenthesised half is
    /// the PTY's CURRENT foreground program, re-read every call, so the label
    /// tracks k9s/cat/wget while they run — it is not a one-time decision.
    pub fn title(&self) -> String {
        let base = self.base_title();
        match self.foreground_program() {
          Some(prog) if !prog.is_empty() && base != prog => format!("{base} ({prog})"),
          _ => base
        }
    }

    /// The OSC title alone (the first half of the label), falling back to the
    /// shell's basename (`zsh`) when the shell never set one.
    pub fn base_title(&self) -> String {
        // The shell-integration cwd (OSC 1337;CurrentDir, T1c) is preferred:
        // it is prompt-accurate and short, whereas the stock OSC title is the
        // raw `user@host:/full/path` that must be ellipsized past the `(prog)`
        // suffix. `~`-collapse home and keep at most the last two components.
        if let Some(cwd) = self.cwd() {
            if let Some(short) = shorten_cwd(&cwd) {
                return short
            }
        }
        if let Some(t) = self.listener.title() {
            if !t.trim().is_empty() {
                return t
            }
        }
        std::path::Path::new(&self.shell).file_name().and_then(|n| n.to_str())
            .map(|s| s.to_string()).unwrap_or_else(|| "shell".to_string())
    }

    /// The shell-integration cwd for this session (OSC 1337;CurrentDir), or
    /// `None` when the shell has not reported one yet.
    pub fn cwd(&self) -> Option<String> {
        self.osc.lock().ok().and_then(|g| g.cwd.clone())
    }

    /// The full OSC state snapshot for the renderer (tint, exit status,
    /// program status, progress). Cheap: one short lock and a clone of the
    /// small strings.
    pub fn osc(&self) -> OscSnapshot {
        match self.osc.lock() {
          Ok(g) => OscSnapshot {
              tint: g.tint,
              exit_status: g.exit_status,
              program: g.program.clone(),
              progress: g.progress,
          },
          Err(_) => OscSnapshot::default(),
        }
    }

    /// Clear the "command finished" markers the user has just looked at:
    /// the blue activity dot AND the exit-status red tint (T2a). Called from
    /// `select_tab` so every selection change clears them.
    pub fn take_attention(&self) {
        let _ = self.listener.activity.swap(false, Ordering::SeqCst);
        if let Ok(mut g) = self.osc.lock() {
            g.exit_status = None;
            // 7501 `done`/`error` are cleared too: once selected, the tab's
            // colored dot has done its job. `working` stays so a still-running
            // program keeps its state until it reports otherwise.
            if let Some((s, _, _)) = g.program.as_ref() {
                if s == "done" || s == "error" {
                    g.program = None;
                }
            }
            // An error progress mark is attention too; clear it on select.
            if let Some((st, _)) = g.progress {
                if st == 2 {
                    g.progress = None;
                }
            }
        }
    }

    /// The program currently in the PTY's foreground process group: one
    /// `tcgetpgrp` syscall plus one string alloc, so it is cheap enough to
    /// call every frame from the draw path. No threads, no polling timers.
    pub fn foreground_program(&self) -> Option<String> {
        // SAFETY: `master_fd` is the pty's own master descriptor, captured
        // while the `Pty` was still alive; `tcgetpgrp` only reads kernel tty
        // state and returns a pid (or <= 0 for "no foreground group", which
        // falls back to the shell pid).
        let fg = unsafe { tcgetpgrp(self.master_fd) };
        let pid = if fg > 0 { fg } else { self.child_pid };
        if pid <= 0 {
            return None
        }

        // SAFETY: `proc_pidpath` writes a NUL-terminated path into the
        // caller-owned buffer; 4096 >= PATH_MAX (1024 on macOS) so the path
        // always fits and the bytes we read below are NUL-terminated.
        let mut path = [0u8; 4096];
        let written = unsafe {
            proc_pidpath(pid, path.as_mut_ptr() as *mut c_void, path.len() as u32)
        };
        if written > 0 {
            let bytes = &path[..written as usize];
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            let bytes = &bytes[..end];
            let name = match bytes.iter().rposition(|b| *b == b'/') {
              Some(slash) => &bytes[slash + 1..],
              _ => bytes
            };
            match String::from_utf8(name.to_vec()).ok() {
              Some(name) if !name.is_empty() => return Some(name),
              _ => {}
            }
        }

        // The executable path can be unavailable (a setuid/redacted process);
        // the kproc short name still identifies the program.
        // SAFETY: same buffer contract — 256 exceeds `MAXCOMLEN` (15 on
        // macOS), so the NUL terminator is always inside the buffer.
        let mut name_buf = [0u8; 256];
        let written = unsafe { proc_name(pid, name_buf.as_mut_ptr() as *mut c_void, name_buf.len() as u32) };
        if written > 0 {
            let bytes = &name_buf[..written as usize];
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            match String::from_utf8(bytes[..end].to_vec()).ok() {
              Some(string) if !string.is_empty() => return Some(string),
              _ => {}
            }
        }
        None
    }

    /// Whether a command has finished on this session since the last check
    /// (the listener's `Event::Title` arm sets the flag; the render path only
    /// peeks, so the dot persists across frames).
    pub fn has_activity(&self) -> bool {
        self.listener.activity.load(Ordering::SeqCst)
    }

    /// Take the pending bell flag: true once per raw BEL, so the view posts
    /// exactly one notification per bell and only for a non-selected tab
    /// (T1a).
    pub fn take_bell(&self) -> bool {
        self.listener.bell.swap(false, Ordering::SeqCst)
    }

    /// Take the just-finished command `(exit_status, seconds)`, if any: the
    /// view's notifier consumes it once, so a long command notifies its owner
    /// at most once (T2a).
    pub fn take_finished(&self) -> Option<(i32, f64)> {
        self.osc.lock().ok().and_then(|mut g| g.pending_finish.take())
    }
}

/// The renderer's view of a session's OSC state (one short lock, cloned).
#[derive(Default, Clone)]
pub struct OscSnapshot {
    /// OSC 6 tab tint, `None` when unset/reset (T1b).
    pub tint: Option<(f64, f64, f64)>,
    /// Last command's exit status from OSC 133;D, cleared on select (T2a).
    pub exit_status: Option<i32>,
    /// Latest OSC 7501 (state, app, msg) (T2b).
    pub program: Option<(String, String, String)>,
    /// Latest OSC 9;4 (st, pr) (T2c).
    pub progress: Option<(i32, i32)>,
}

/// Shorten an absolute path for a tab label: collapse the home prefix to `~`
/// and keep at most the LAST TWO components (`~/sub` / `~/a/b`). A one-level
/// path stays as-is. Returns `None` for an empty path.
fn shorten_cwd(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        return None
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty());
    let collapsed = match &home {
        Some(home) => {
            if path == home { "~".to_string() }
            else if let Some(rest) = path.strip_prefix(&format!("{home}/")) { format!("~/{rest}") }
            else { path.to_string() }
        }
        None => path.to_string(),
    };
    let parts: Vec<&str> = collapsed.split('/').filter(|p| !p.is_empty()).collect();
    // Keep the leading `~` (if any) plus the last two real components.
    let tail: Vec<&str> = if collapsed.starts_with('~') {
        let mut v = vec!["~"];
        v.extend(parts.iter().copied().filter(|p| *p != "~").rev().take(2).collect::<Vec<_>>().into_iter().rev());
        v
    } else {
        parts.iter().copied().rev().take(2).collect::<Vec<_>>().into_iter().rev().collect()
    };
    let out = if collapsed.starts_with('~') { tail.join("/") } else { format!("/{}", tail.join("/")) };
    Some(out)
}

/// Resolve a terminal color to sRGB components in `0..1`.
///
/// `Term.colors()` starts as `[None; 269]` and is only written by OSC/SGR
/// color operations, so a `None` entry falls back to our own default palette.
fn resolve(colors: &Colors, color: AnsiColor) -> (f64, f64, f64) {
    match color {
        AnsiColor::Spec(rgb) => (rgb.r as f64 / 255.0, rgb.g as f64 / 255.0, rgb.b as f64 / 255.0),
        AnsiColor::Indexed(index) => palette_or(colors, index as usize),
        AnsiColor::Named(named) => palette_or(colors, named as usize),
    }
}

/// Indexed palette fallback: the terminal's `colors()` table when set, else a
/// fixed dark-theme palette (black bg, light gray fg family).
fn palette_or(colors: &Colors, index: usize) -> (f64, f64, f64) {
    if let Some(rgb) = colors[index] {
        return (rgb.r as f64 / 255.0, rgb.g as f64 / 255.0, rgb.b as f64 / 255.0)
    }
    default_color(index)
}

/// A fixed dark-theme palette for the 16 named colors (alacritty's defaults).
fn default_color(index: usize) -> (f64, f64, f64) {
    let palette = [
        (0x18, 0x18, 0x18), (0xC4, 0x3C, 0x3B), (0x70, 0x98, 0x78), (0xD8, 0xA8, 0x4A),
        (0x4C, 0x72, 0x9E), (0xA8, 0x68, 0x94), (0x68, 0xA8, 0xC4), (0xD8, 0xD8, 0xD8),
        (0x60, 0x60, 0x60), (0xE4, 0x5C, 0x5B), (0x98, 0xC0, 0xA0), (0xF0, 0xC0, 0x62),
        (0x6C, 0x92, 0xBE), (0xD8, 0x98, 0xC4), (0x88, 0xC8, 0xE4), (0xF8, 0xF8, 0xF8),
    ];
    if index < palette.len() {
        let (r, g, b) = palette[index];
        return (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
    }
    // Named defaults (256..258): the theme's own fg/bg, not the ramp.
    // `NamedColor::Background` is what every unpainted cell carries, so
    // falling through to the ramp would paint the whole grid gray.
    if index >= 256 {
        return if index == 257 { (0.0, 0.0, 0.0) } else { (0xD8 as f64 / 255.0, 0xD8 as f64 / 255.0, 0xD8 as f64 / 255.0) }
    }
    // Color cube (16..232) and grayscale ramp (233..255): approximate.
    if index < 232 {
        let c = index - 16;
        let steps = [0x00, 0x5F, 0x87, 0xAF, 0xD7, 0xFF];
        let r = steps[c / 36 % 6];
        let g = steps[c / 6 % 6];
        let b = steps[c % 6];
        return (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
    }
    let v = 8 + (index - 233) * 10;
    (v as f64 / 255.0, v as f64 / 255.0, v as f64 / 255.0)
}

/// Post a macOS user notification with `osascript` (dependency-free; the
/// `display notification` AppleScript needs no bundle signing or
/// `UserNotifications` entitlement, unlike `NSUserNotification`). Runs on a
/// detached thread so a slow AppleScript launch never stalls the 60Hz draw
/// loop, and the script is passed via `-e` with the strings escaped so a
/// title containing quotes cannot inject AppleScript.
pub fn notify(title: &str, message: &str) {
    let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape(message),
        escape(title),
    );
    std::thread::spawn(move || {
        let _ = std::process::Command::new("osascript").arg("-e").arg(script).status();
    });
}

/// The login shell for the current user (`getpwuid(getuid()).pw_shell`).
pub fn login_shell() -> Option<String> {
    let entry = unsafe { getpwuid(getuid()) };
    if entry.is_null() {
        return None
    }

    let shell = unsafe { (*entry).pw_shell };
    if shell.is_null() {
        return None
    }

    let mut bytes = Vec::new();
    let mut offset = 0;
    loop {
        let byte = unsafe { *shell.add(offset) };
        if byte == 0 {
            break
        }
        bytes.push(byte as u8);
        offset += 1;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drive the tee's parser exactly as `OscReader::read` does and return the
    /// harvested state. Fed in small chunks to prove the parser is
    /// incremental (a sequence split across two `read`s still parses).
    fn parse(bytes: &[u8], chunks: usize) -> OscState {
        let state = Mutex::new(OscState::default());
        let activity = AtomicBool::new(false);
        let mut parser = VteParser::new();
        for chunk in bytes.chunks(chunks) {
            let mut handler = OscHandler { state: &state, activity: &activity };
            parser.advance(&mut handler, chunk);
        }
        state.into_inner().unwrap()
    }

    #[test]
    fn parses_cwd_exit_status_and_lifecycle() {
        // The exact byte stream the zsh hook emits, and the `133;C/D` gap
        // that measures command runtime.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x1b]133;A\x07");
        bytes.extend_from_slice(b"\x1b]1337;RemoteHost=me@host\x07");
        bytes.extend_from_slice(b"\x1b]1337;CurrentDir=/Users/me/projects/rust\x07");
        bytes.extend_from_slice(b"\x1b]133;C\x07");
        bytes.extend_from_slice(b"\x1b]133;D;1\x07");
        let state = parse(&bytes, 1);
        assert_eq!(state.cwd.as_deref(), Some("/Users/me/projects/rust"));
        assert_eq!(state.exit_status, Some(1));
        assert!(state.pending_finish.is_some(), "a started+finished command queues a finish");
    }

    #[test]
    fn first_prompt_does_not_queue_finish() {
        // A bare `133;D` (a prompt with no command before it) must NOT queue a
        // notification, or every freshly spawned tab would banner "completed".
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x1b]133;D;0\x07");
        let state = parse(&bytes, 3);
        assert_eq!(state.exit_status, Some(0));
        assert!(state.pending_finish.is_none(), "no command start -> no finish event");
    }

    #[test]
    fn parses_incrementally_across_chunks() {
        // One byte per chunk: every sequence is split mid-flight.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x1b]1337;CurrentDir=/tmp/x\x07");
        bytes.extend_from_slice(b"\x1b]133;D;0\x07");
        let state = parse(&bytes, 1);
        assert_eq!(state.cwd.as_deref(), Some("/tmp/x"));
        assert_eq!(state.exit_status, Some(0));
    }

    #[test]
    fn parses_tint_program_and_progress() {
        let mut bytes = Vec::new();
        // iTerm OSC 6: one channel per sequence, then the reset form.
        bytes.extend_from_slice(b"\x1b]6;1;bg;red;brightness;255\x1b\\");
        bytes.extend_from_slice(b"\x1b]6;1;bg;green;brightness;0\x1b\\");
        bytes.extend_from_slice(b"\x1b]6;1;bg;blue;brightness;128\x1b\\");
        bytes.extend_from_slice(b"\x1b]7501;state=error:app=make:msg=ZmFpbA==\x1b\\");
        bytes.extend_from_slice(b"\x1b]9;4;1;42\x1b\\");
        let state = parse(&bytes, 7);
        assert_eq!(state.tint, Some((1.0, 0.0, 128.0 / 255.0)));
        assert_eq!(state.program.as_ref().map(|(s, _, _)| s.as_str()), Some("error"));
        assert_eq!(state.progress, Some((1, 42)));
    }

    #[test]
    fn reset_clears_tint() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"\x1b]6;1;bg;red;brightness;255\x07");
        bytes.extend_from_slice(b"\x1b]6;1;bg;green;brightness;0\x07");
        bytes.extend_from_slice(b"\x1b]6;1;bg;blue;brightness;0\x07");
        bytes.extend_from_slice(b"\x1b]6;1;bg;*;default\x07");
        let state = parse(&bytes, 5);
        assert_eq!(state.tint, None);
    }

    /// The correctness bar for the tee: `OscReader::read` must return EXACTLY
    /// the bytes the pipe holds (no drop/duplicate/reorder) while ALSO feeding
    /// them to the parser. Uses a real pipe fd so this exercises the same
    /// `libc::read` path the PTY reader does.
    #[test]
    fn reader_is_byte_transparent_and_parses() {
        let mut fds = [0 as RawFd; 2];
        // SAFETY: `fds` is a valid 2-element array for `pipe(2)`.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (read_fd, write_fd) = (fds[0], fds[1]);

        let state = Arc::new(Mutex::new(OscState::default()));
        let activity = Arc::new(AtomicBool::new(false));
        let mut reader = OscReader {
            fd: read_fd,
            parser: VteParser::new(),
            state: state.clone(),
            activity: activity.clone(),
        };

        // A stream with ordinary grid content AROUND the OSC, so a tee that
        // shifted bytes would be caught by the equality assertion.
        let payload = b"hello\x1b]133;D;7\x07world\x1b]1337;CurrentDir=/tmp/z\x07!";
        // SAFETY: `payload` is a valid buffer and `write_fd` a valid fd.
        let written = unsafe {
            libc::write(write_fd, payload.as_ptr() as *const c_void, payload.len())
        };
        assert_eq!(written, payload.len() as isize);
        unsafe { libc::close(write_fd) };

        let mut got = Vec::new();
        let mut buf = [0u8; 64];
        loop {
            let n = reader.read(&mut buf).expect("read from pipe");
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        unsafe { libc::close(read_fd) };

        assert_eq!(got, payload, "the tee must be byte-transparent");
        let state = state.lock().unwrap();
        assert_eq!(state.exit_status, Some(7));
        assert_eq!(state.cwd.as_deref(), Some("/tmp/z"));
        assert!(activity.load(Ordering::SeqCst), "133;D must flag activity");
    }

    #[test]
    fn shorten_cwd_keeps_tail() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/nobody".to_string());
        assert_eq!(shorten_cwd(&home).as_deref(), Some("~"));
        assert_eq!(shorten_cwd(&format!("{home}/a")).as_deref(), Some("~/a"));
        assert_eq!(shorten_cwd(&format!("{home}/a/b/c")).as_deref(), Some("~/b/c"));
        assert_eq!(shorten_cwd("/usr/local/share").as_deref(), Some("/local/share"));
        assert_eq!(shorten_cwd(""), None);
    }
}
