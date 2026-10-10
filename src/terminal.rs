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

use std::borrow::Cow;
use std::sync::Arc;
use std::collections::HashMap;
use std::thread::JoinHandle;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg, State};
use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::{Colors};
use alacritty_terminal::tty::{new as pty_new, Options, Pty, Shell};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, NamedColor, Rgb as AnsiRgb};

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
}

impl TitleListener {
    pub fn new() -> Self {
        Self {
            title: Arc::new(std::sync::Mutex::new(None)),
            sender: Arc::new(std::sync::Mutex::new(None)),
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
}

impl EventListener for TitleListener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(t) => {
                if let Ok(mut g) = self.title.lock() {
                    *g = Some(t);
                }
            }
            Event::ResetTitle => {
                if let Ok(mut g) = self.title.lock() {
                    *g = None;
                }
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
    sender: EventLoopSender,
    reader: JoinHandle<(EventLoop<Pty, TitleListener>, State)>,
    rows: usize,
    columns: usize,
}

impl Session {
    /// Spawn the PTY once and start the PTY reader thread.
    pub fn new(rows: usize, columns: usize, cell_width: f64, cell_height: f64) -> Self {
        let shell = login_shell().expect("the current user has a login shell");
        let listener = TitleListener::new();

        let options = Options {
            shell: Some(Shell::new(shell.clone(), ["-l", "-i"].map(|arg| arg.to_string()).to_vec())),
            working_directory: None,
            drain_on_exit: true,
            env: HashMap::new(),
        };

        let window_size = WindowSize {
            num_lines: rows as u16,
            num_cols: columns as u16,
            cell_width: cell_width.ceil() as u16,
            cell_height: cell_height.ceil() as u16,
        };

        let pty = pty_new(&options, window_size, 1).expect("the PTY could not be spawned");

        let terminal = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &NotchSize { screen_lines: rows, columns },
            listener.clone(),
        )));

        let loop_ = EventLoop::new(terminal.clone(), listener.clone(), pty, true, false)
            .expect("the PTY event loop could not be created");

        // Wire the PTY writer so terminal responses (cursor-position reports,
        // device-status/attribute replies) are routed back to the shell. The
        // EventLoop owns the channel; hand the listener a clone of it.
        listener.set_sender(loop_.channel());

        Session {
            terminal,
            listener,
            shell,
            sender: loop_.channel(),
            reader: loop_.spawn(),
            rows,
            columns,
        }
    }

    /// Write user input (a keystroke, pasted text, …) to the PTY.
    pub fn send_input(&self, bytes: Vec<u8>) {
        let _ = self.sender.send(Msg::Input(Cow::from(bytes)));
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

    /// The tab label for this session: the OSC title the shell set (iTerm/
    /// zsh precmd style), falling back to the shell's basename (`zsh`).
    pub fn title(&self) -> String {
        if let Some(t) = self.listener.title() {
            if !t.trim().is_empty() {
                return t
            }
        }
        std::path::Path::new(&self.shell).file_name().and_then(|n| n.to_str())
            .map(|s| s.to_string()).unwrap_or_else(|| "shell".to_string())
    }
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
