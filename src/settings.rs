//! Validated, owned settings and asynchronous NSUserDefaults submission.
//! `BNT.*` keys and `BNT_DEFAULTS_SUITE` are stable compatibility contracts.
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_foundation::CGFloat;
use objc2_foundation::{MainThreadMarker, NSString, NSUserDefaults};

use crate::consts::{
    CLOSED_HEIGHT, DEFAULT_SCROLLBACK_LINES, NOTCH_WIDTH, OPEN_HEIGHT, PANEL_HEIGHT, PANEL_WIDTH,
};
use crate::font::FONT_SIZE;

pub type SettingsSnapshot = Rc<Settings>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditPhase {
    Preview,
    Commit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingsError {
    Validation(String),
    Hotkey(String),
    Persistence(PersistenceError),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(s) => write!(f, "Invalid setting: {s}"),
            Self::Hotkey(s) => write!(f, "Shortcut registration: {s}"),
            Self::Persistence(e) => e.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistenceError(pub String);

impl fmt::Display for PersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Preferences store: {}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    r: CGFloat,
    g: CGFloat,
    b: CGFloat,
}

impl Color {
    pub const fn new(r: CGFloat, g: CGFloat, b: CGFloat) -> Self {
        Self { r, g, b }
    }

    pub fn rgb(&self) -> (CGFloat, CGFloat, CGFloat) {
        (self.r, self.g, self.b)
    }

    pub fn rgba(&self) -> (CGFloat, CGFloat, CGFloat, CGFloat) {
        (self.r, self.g, self.b, 1.0)
    }

    pub fn hex(&self) -> String {
        format!(
            "#{:02X}{:02X}{:02X}",
            (self.r * 255.0).round() as u8,
            (self.g * 255.0).round() as u8,
            (self.b * 255.0).round() as u8
        )
    }

    fn valid(self) -> bool {
        [self.r, self.g, self.b]
            .iter()
            .all(|x| x.is_finite() && (0.0..=1.0).contains(x))
    }
}

pub fn parse_hex(hex: &str) -> Color {
    let h = hex.trim().trim_start_matches('#');
    let value = if h.len() == 6 {
        u32::from_str_radix(h, 16).unwrap_or(0)
    } else {
        0
    };
    Color::new(
        ((value >> 16) & 255) as f64 / 255.0,
        ((value >> 8) & 255) as f64 / 255.0,
        (value & 255) as f64 / 255.0,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub key_code: u16,
    pub option: bool,
    pub cmd: bool,
    pub shift: bool,
    pub ctrl: bool,
}

const fn chord(key_code: u16, option: bool, cmd: bool, shift: bool, ctrl: bool) -> Chord {
    Chord {
        key_code,
        option,
        cmd,
        shift,
        ctrl,
    }
}

pub const DEFAULT_TOGGLE: Chord = chord(49, true, false, false, false);
pub const DEFAULT_NEW_TAB: Chord = chord(17, false, true, false, false);
// Local recording needs no global monitoring permission. fn/globe is unsupported.
pub const CHORD_PRESETS: [Chord; 7] = [
    DEFAULT_TOGGLE,
    chord(50, true, false, false, false),
    chord(50, false, false, false, true),
    chord(36, true, false, false, false),
    chord(36, false, true, false, false),
    chord(51, true, false, false, false),
    DEFAULT_NEW_TAB,
];
/// Terminal-local fallbacks, tried in order when the stored new-tab chord is
/// unusable (see [`Settings::effective_chords`]). Every entry passes
/// [`validate_local_chord`] and none equals [`DEFAULT_TOGGLE`], and the picker
/// additionally skips whatever toggle is actually in effect, so the fallback
/// pair can never collide.
pub const LOCAL_CHORD_FALLBACKS: [Chord; 3] = [
    DEFAULT_NEW_TAB,
    chord(50, true, false, false, false),
    chord(50, false, false, false, true),
];
pub const FONT_FAMILIES: [&str; 5] = ["Fira Code", "Menlo", "Monaco", "SF Mono", "Andale Mono"];
pub const SCROLLBACK_PRESETS: [usize; 6] = [0, 1000, 5000, 10000, 50000, 100000];
pub const PALETTE_PRESETS: [Color; 12] = [
    Color::new(0.0, 0.0, 0.0),
    Color::new(0.094, 0.094, 0.094),
    Color::new(0.078, 0.078, 0.078),
    Color::new(0.169, 0.169, 0.169),
    Color::new(0.231, 0.235, 0.231),
    Color::new(0.298, 0.447, 0.620),
    Color::new(0.439, 0.596, 0.471),
    Color::new(0.847, 0.659, 0.290),
    Color::new(0.659, 0.408, 0.580),
    Color::new(0.408, 0.659, 0.769),
    Color::new(0.847, 0.847, 0.847),
    Color::new(1.0, 1.0, 1.0),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub cursor_color: Color,
    pub ansi: [Color; 16],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub open_height: CGFloat,
    pub open_width: CGFloat,
    pub font_family: String,
    pub font_size: CGFloat,
    pub ligatures: bool,
    pub background: Color,
    pub foreground: Color,
    pub ansi: [Color; 16],
    pub cursor_color: Color,
    pub cursor_shape: CursorShape,
    pub scrollback_lines: usize,
    pub toggle_chord: Chord,
    pub new_tab_chord: Chord,
    pub bell_enabled: bool,
    pub finished_notify_secs: CGFloat,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            open_height: OPEN_HEIGHT,
            open_width: PANEL_WIDTH,
            font_family: "Fira Code".into(),
            font_size: FONT_SIZE,
            ligatures: true,
            background: parse_hex("000000"),
            foreground: parse_hex("D8D8D8"),
            ansi: [
                "181818", "C43C3B", "709878", "D8A84A", "4C729E", "A86894", "68A8C4", "D8D8D8",
                "606060", "E45C5B", "98C0A0", "F0C062", "6C92BE", "D898C4", "88C8E4", "F8F8F8",
            ]
            .map(parse_hex),
            cursor_color: Color::new(0.75, 0.75, 0.75),
            cursor_shape: CursorShape::Block,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            toggle_chord: DEFAULT_TOGGLE,
            new_tab_chord: DEFAULT_NEW_TAB,
            bell_enabled: true,
            finished_notify_secs: 5.0,
        }
    }
}

impl Settings {
    pub fn palette(&self) -> Palette {
        Palette {
            background: self.background,
            foreground: self.foreground,
            cursor_color: self.cursor_color,
            ansi: self.ansi,
        }
    }

    /// The width the panel actually paints: the persisted preference clamped
    /// into the live canvas. `open_width` is the RAW user preference and
    /// survives a monitor change; a canvas shrink follows it here instead of
    /// rewriting or re-persisting it, and a canvas grow restores it by itself.
    pub fn effective_open_width(&self, canvas: f64) -> CGFloat {
        let canvas = sane_canvas(canvas);
        let floor = NOTCH_WIDTH.min(canvas);
        if self.open_width.is_finite() {
            self.open_width.clamp(floor, canvas.max(floor))
        } else {
            floor
        }
    }

    /// The chord pair the runtime actually uses. `toggle_chord` and
    /// `new_tab_chord` are the persisted RAW values — load never rewrites them,
    /// so the user's intent survives and is re-persisted verbatim — while a
    /// value that cannot be registered (a legacy reserved global chord) or that
    /// collides with the toggle stays stored, is reported at load, and is
    /// replaced at runtime by these fallbacks. Changing a chord still requires a
    /// valid replacement ([`SettingsState::validate_proposal`]), so a fallback is
    /// never silently persisted in place of the user's choice.
    pub fn effective_chords(&self) -> (Chord, Chord) {
        let toggle = if validate_toggle_chord(self.toggle_chord).is_ok() {
            self.toggle_chord
        } else {
            DEFAULT_TOGGLE
        };
        if validate_local_chord(self.new_tab_chord).is_ok() && self.new_tab_chord != toggle {
            (toggle, self.new_tab_chord)
        } else {
            let local = LOCAL_CHORD_FALLBACKS
                .iter()
                .copied()
                .find(|c| validate_local_chord(*c).is_ok() && *c != toggle)
                .unwrap_or(DEFAULT_NEW_TAB);
            (toggle, local)
        }
    }

    /// Pure field validation. Font availability is checked by the main-thread
    /// acceptor. Geometry and chords are validated against what the runtime can
    /// actually use: the width needs only a finite value at or above the canvas
    /// floor (the painted width is [`Settings::effective_open_width`], so a
    /// shrunk display is not a settings error), and the chord policy applies to
    /// the [`Settings::effective_chords`] pair, so a legacy stored chord that
    /// cannot be registered never blocks an unrelated edit.
    pub fn validate(&self, canvas: f64) -> Result<(), SettingsError> {
        let range = |name: &str, x: f64, lo: f64, hi: f64| {
            if x.is_finite() && (lo..=hi).contains(&x) {
                Ok(())
            } else {
                Err(SettingsError::Validation(format!(
                    "{name} must be finite and between {lo} and {hi}"
                )))
            }
        };
        range(
            "Height",
            self.open_height,
            CLOSED_HEIGHT + 80.0,
            PANEL_HEIGHT - 20.0,
        )?;
        // Raw-vs-effective: a persisted width above a SHRUNK canvas is a
        // geometry situation, not an invalid setting. Only the floor is a
        // validation failure; the top bound is applied when painting.
        let width_floor = NOTCH_WIDTH.min(sane_canvas(canvas));
        if !self.open_width.is_finite() || self.open_width < width_floor {
            return Err(SettingsError::Validation(format!(
                "Width must be finite and at least {width_floor}"
            )));
        }
        range("Font size", self.font_size, 8.0, 32.0)?;
        range(
            "Notification duration",
            self.finished_notify_secs,
            0.0,
            3600.0,
        )?;
        if self.font_family.trim().is_empty() || self.font_family.contains('\0') {
            return Err(SettingsError::Validation(
                "Choose an installed monospace font".into(),
            ));
        }
        if self.scrollback_lines > 100_000 {
            return Err(SettingsError::Validation(
                "History limit is 100000 lines".into(),
            ));
        }
        if ![self.background, self.foreground, self.cursor_color]
            .into_iter()
            .chain(self.ansi)
            .all(Color::valid)
        {
            return Err(SettingsError::Validation(
                "Colors require finite sRGB channels in 0…1".into(),
            ));
        }
        // The runtime pair, not the raw stored pair: this asserts the invariant
        // that `effective_chords` always yields a registrable, non-conflicting
        // global/local combination. Explicit chord edits are additionally
        // validated against the raw value in `SettingsState::validate_proposal`.
        let (toggle, local) = self.effective_chords();
        validate_chords(toggle, local)
    }

    pub fn load() -> Self {
        Self::load_with_status().0
    }

    pub fn load_with_status() -> (Self, Vec<String>) {
        let store = match defaults_store() {
            Ok(s) => s,
            Err(e) => return (Self::default(), vec![e.to_string()]),
        };
        let mut values = Preferences::new();
        for (key, kind) in Self::default().preferences() {
            let name = NSString::from_str(&key);
            if store.objectForKey(&name).is_none() {
                continue;
            }
            let value = match kind {
                PrefValue::Text(_) => match store.stringForKey(&name) {
                    Some(s) => PrefValue::Text(s.to_string()),
                    None => continue,
                },
                PrefValue::Bool(_) => PrefValue::Bool(store.boolForKey(&name)),
                PrefValue::Integer(_) => PrefValue::Integer(store.integerForKey(&name) as i64),
                PrefValue::Number(_) => PrefValue::Number(store.doubleForKey(&name)),
            };
            values.insert(key, value);
        }
        Self::from_preferences(&values, display_width())
    }

    /// Submission only: success is NOT synchronous durable or atomic storage.
    pub fn save(&self) -> Result<(), PersistenceError> {
        let store = defaults_store()?;
        for (key, value) in self.preferences() {
            let name = NSString::from_str(&key);
            match value {
                PrefValue::Text(s) => {
                    let s = NSString::from_str(&s);
                    unsafe { store.setObject_forKey(Some(&s), &name) };
                }
                PrefValue::Bool(v) => store.setBool_forKey(v, &name),
                PrefValue::Integer(v) => store.setInteger_forKey(v as isize, &name),
                PrefValue::Number(v) => store.setDouble_forKey(v, &name),
            }
        }
        Ok(())
    }

    pub fn preferences(&self) -> Preferences {
        let mut out = Preferences::new();
        for (k, v) in [
            ("openHeight", self.open_height),
            ("openWidth", self.open_width),
            ("fontSize", self.font_size),
            ("notifySecs", self.finished_notify_secs),
        ] {
            out.insert(key_name(k), PrefValue::Number(v));
        }
        for (k, v) in [
            ("ligatures", self.ligatures),
            ("bellEnabled", self.bell_enabled),
        ] {
            out.insert(key_name(k), PrefValue::Bool(v));
        }
        for (k, v) in [
            ("fontFamily", family_index(&self.font_family)),
            ("cursorShape", cursor_shape_index(self.cursor_shape)),
            ("scrollbackLines", scrollback_index(self.scrollback_lines)),
            ("historyLines", self.scrollback_lines),
        ] {
            out.insert(key_name(k), PrefValue::Integer(v as i64));
        }
        out.insert(
            key_name("fontFamilyName"),
            PrefValue::Text(self.font_family.clone()),
        );
        for (prefix, color) in [
            ("bg".into(), self.background),
            ("fg".into(), self.foreground),
            ("cursor".into(), self.cursor_color),
        ]
        .into_iter()
        .chain(self.ansi.iter().enumerate().map(|(i, c)| (ansi_key(i), *c)))
        {
            for (c, v) in [("r", color.r), ("g", color.g), ("b", color.b)] {
                out.insert(key_name(&channel_key(&prefix, c)), PrefValue::Number(v));
            }
        }
        for (p, c) in [
            ("toggle", self.toggle_chord),
            ("newTab", self.new_tab_chord),
        ] {
            out.insert(
                key_name(&channel_key(p, "key")),
                PrefValue::Integer(c.key_code as i64),
            );
            out.insert(
                key_name(&channel_key(p, "mods")),
                PrefValue::Integer(chord_mask(c) as i64),
            );
        }
        out
    }

    /// Pure decoding; malformed fields fall back individually with warnings.
    pub fn from_preferences(values: &Preferences, canvas: f64) -> (Self, Vec<String>) {
        let mut s = Self::default();
        let mut warnings = vec![];
        let number = |k: &str| match values.get(&key_name(k)) {
            Some(PrefValue::Number(v)) => Some(*v),
            Some(PrefValue::Integer(v)) => Some(*v as f64),
            _ => None,
        };
        let canvas = sane_canvas(canvas);
        for (k, field, lo, hi) in [
            (
                "openHeight",
                &mut s.open_height,
                CLOSED_HEIGHT + 80.0,
                PANEL_HEIGHT - 20.0,
            ),
            ("fontSize", &mut s.font_size, 8.0, 32.0),
            ("notifySecs", &mut s.finished_notify_secs, 0.0, 3600.0),
        ] {
            if let Some(v) = number(k) {
                if v.is_finite() {
                    *field = v.clamp(lo, hi);
                } else {
                    warnings.push(format!("Invalid stored {k}; using default"));
                }
            }
            *field = field.clamp(lo, hi);
        }
        // `openWidth` is stored RAW and is NOT clamped to the canvas: the user's
        // preferred width outlives a monitor change (see
        // `effective_open_width`), so a shrink reports rather than destroys it.
        // Only a non-finite value is discarded, and only the floor is enforced.
        if let Some(v) = number("openWidth") {
            if v.is_finite() {
                s.open_width = v.max(NOTCH_WIDTH.min(canvas));
                if v > canvas {
                    warnings.push(format!(
                        "Stored width {v:.0}pt exceeds the current display ({canvas:.0}pt); using {canvas:.0}pt until you change it"
                    ));
                }
            } else {
                warnings.push("Invalid stored openWidth; using default".into());
            }
        }
        s.open_width = s.open_width.max(NOTCH_WIDTH.min(canvas));
        for (k, field) in [
            ("ligatures", &mut s.ligatures),
            ("bellEnabled", &mut s.bell_enabled),
        ] {
            if let Some(PrefValue::Bool(v)) = values.get(&key_name(k)) {
                *field = *v;
            }
        }
        let index = |k: &str| {
            number(k)
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(0.0) as usize
        };
        s.font_family = decode_font_family(values);
        s.cursor_shape = cursor_shape_of(index("cursorShape"));
        s.scrollback_lines = if number("historyLines").is_some() {
            index("historyLines").min(100_000)
        } else if number("scrollbackLines").is_some() {
            scrollback_of(index("scrollbackLines"))
        } else {
            s.scrollback_lines
        };
        for (p, field) in [
            ("bg".into(), &mut s.background),
            ("fg".into(), &mut s.foreground),
            ("cursor".into(), &mut s.cursor_color),
        ]
        .into_iter()
        .chain(s.ansi.iter_mut().enumerate().map(|(i, c)| (ansi_key(i), c)))
        {
            if let (Some(r), Some(g), Some(b)) = (
                number(&channel_key(&p, "r")),
                number(&channel_key(&p, "g")),
                number(&channel_key(&p, "b")),
            ) {
                let c = Color::new(r, g, b);
                if c.valid() {
                    *field = c;
                } else {
                    warnings.push(format!("Invalid stored {p} color; using default"));
                }
            }
        }
        // Chords decode RAW and are never rewritten here: an unusable stored
        // value is reported and handled by `effective_chords` at runtime, while
        // `preferences()` keeps persisting the original intent verbatim. The
        // malformed case (non-finite / out of range) cannot be decoded at all,
        // so it does fall back to the default and says so.
        for (p, field) in [
            ("toggle", &mut s.toggle_chord),
            ("newTab", &mut s.new_tab_chord),
        ] {
            if let (Some(k), Some(m)) = (
                number(&channel_key(p, "key")),
                number(&channel_key(p, "mods")),
            ) {
                if k.is_finite()
                    && (0.0..128.0).contains(&k)
                    && m.is_finite()
                    && (0.0..=65535.0).contains(&m)
                {
                    *field = chord_of(k as u16, m as u32);
                } else {
                    warnings.push(format!("Invalid stored {p} shortcut; using default"));
                    *field = if p == "toggle" {
                        DEFAULT_TOGGLE
                    } else {
                        DEFAULT_NEW_TAB
                    };
                }
            }
        }
        // An unusable stored chord, or one that duplicates the effective toggle,
        // is stored raw and reported; the runtime replaces it either way.
        if validate_toggle_chord(s.toggle_chord).is_err() {
            warnings.push(chord_status(
                s.toggle_chord,
                ChordKind::Toggle,
                DEFAULT_TOGGLE,
            ));
        }
        if validate_local_chord(s.new_tab_chord).is_err() || s.toggle_chord == s.new_tab_chord {
            warnings.push(chord_status(
                s.new_tab_chord,
                ChordKind::NewTab,
                s.toggle_chord,
            ));
        }
        (s, warnings)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PrefValue {
    Text(String),
    Number(f64),
    Integer(i64),
    Bool(bool),
}
pub type Preferences = BTreeMap<String, PrefValue>;

pub fn decode_font_family(values: &Preferences) -> String {
    if let Some(PrefValue::Text(s)) = values.get(&key_name("fontFamilyName"))
        && !s.trim().is_empty()
    {
        return s.clone();
    }
    let i = match values.get(&key_name("fontFamily")) {
        Some(PrefValue::Integer(i)) => (*i).max(0) as usize,
        _ => 0,
    };
    FONT_FAMILIES[i.min(FONT_FAMILIES.len() - 1)].into()
}

fn defaults_store() -> Result<Retained<NSUserDefaults>, PersistenceError> {
    match std::env::var("BNT_DEFAULTS_SUITE") {
        Ok(s) if s.trim().is_empty() => Err(PersistenceError(
            "BNT_DEFAULTS_SUITE is empty; refusing default-domain fallback".into(),
        )),
        Ok(s) => NSUserDefaults::initWithSuiteName(
            NSUserDefaults::alloc(),
            Some(&NSString::from_str(&s)),
        )
        .ok_or_else(|| PersistenceError("Could not create defaults suite".into())),
        Err(std::env::VarError::NotPresent) => Ok(NSUserDefaults::standardUserDefaults()),
        Err(_) => Err(PersistenceError("BNT_DEFAULTS_SUITE is not UTF-8".into())),
    }
}

/// A usable canvas width: the runtime display width, or `PANEL_WIDTH` when the
/// screen lookup yields a non-finite or non-positive value. One place guards
/// finiteness, so the load clamp, `validate`'s floor and
/// `effective_open_width` cannot disagree.
fn sane_canvas(canvas: f64) -> CGFloat {
    if canvas.is_finite() && canvas > 0.0 {
        canvas
    } else {
        PANEL_WIDTH
    }
}

pub fn display_width() -> CGFloat {
    MainThreadMarker::new()
        .map(crate::window::canvas_size)
        .unwrap_or(PANEL_WIDTH)
}

pub fn chord_mask(c: Chord) -> u32 {
    let m = ((c.cmd as u32) << 8)
        | ((c.shift as u32) << 9)
        | ((c.option as u32) << 11)
        | ((c.ctrl as u32) << 12);
    m | u32::from(m != 0)
}

pub fn chord_of(key_code: u16, mask: u32) -> Chord {
    chord(
        key_code,
        mask & (1 << 11) != 0,
        mask & (1 << 8) != 0,
        mask & (1 << 9) != 0,
        mask & (1 << 12) != 0,
    )
}

fn local_reserved(c: Chord) -> bool {
    c.cmd
        && matches!(c.key_code, 0 | 6..=9 | 12 | 13 | 18..=23 | 25 | 26 | 28 | 43 | 48 | 49 | 50 | 53)
}

pub fn validate_toggle_chord(c: Chord) -> Result<(), SettingsError> {
    if c.key_code >= 128 || !(c.option || c.cmd || c.ctrl) {
        return Err(SettingsError::Validation(
            "Global toggle needs Command, Option or Control and a supported key".into(),
        ));
    }
    // Spotlight/input switching, native/local commands, app switching, screenshots.
    if local_reserved(c) || (c.key_code == 49 && c.ctrl) || (c.cmd && c.key_code == 17) {
        return Err(SettingsError::Validation(
            "This shortcut is reserved by macOS or a native/local command".into(),
        ));
    }
    Ok(())
}

pub fn validate_local_chord(c: Chord) -> Result<(), SettingsError> {
    if c.key_code >= 128 || !(c.cmd || c.ctrl || c.option) || local_reserved(c) {
        return Err(SettingsError::Validation(
            "New-tab shortcut conflicts with a native/local command or unsupported key".into(),
        ));
    }
    Ok(())
}

pub fn validate_chords(toggle: Chord, local: Chord) -> Result<(), SettingsError> {
    validate_toggle_chord(toggle)?;
    validate_local_chord(local)?;
    if toggle == local {
        return Err(SettingsError::Validation(
            "Toggle and new tab need different shortcuts".into(),
        ));
    }
    Ok(())
}

/// Proposal policy for a [`Chord`] the user is actively changing to, checked
/// against the toggle the runtime will actually register (the first element of
/// [`Settings::effective_chords`]).
///
/// This is the RAW side of the raw-vs-effective split: a value that is merely
/// STORED and unusable must not block an unrelated edit, but a chord the user is
/// committing to must be registrable and must not duplicate the toggle. A
/// duplicate global/local chord is never accepted.
pub fn validate_chord_change(
    chord: Chord,
    changed: bool,
    effective_toggle: Chord,
) -> Result<(), SettingsError> {
    if !changed {
        return Ok(());
    }
    validate_local_chord(chord)?;
    if chord == effective_toggle {
        return Err(SettingsError::Validation(
            "Toggle and new tab need different shortcuts".into(),
        ));
    }
    Ok(())
}

/// What a UI should report about one stored chord while the runtime uses a
/// fallback. Pure and value-preserving; the caller decides whether the stored
/// chord needs a report at all (see the load-time guards).
pub fn chord_status(chord: Chord, kind: ChordKind, effective_toggle: Chord) -> String {
    match kind {
        ChordKind::Toggle => format!(
            "Stored toggle {} cannot be registered; {DEFAULT_TOGGLE_NAME} is active until you choose a new one",
            chord_name(chord),
        ),
        ChordKind::NewTab if chord == effective_toggle => format!(
            "Stored new-tab {} is the toggle shortcut; terminal-local Cmd+T is used instead",
            chord_name(chord),
        ),
        ChordKind::NewTab => format!(
            "Stored new-tab {} conflicts with a native or local command; terminal-local Cmd+T is used until you choose a new one",
            chord_name(chord),
        ),
    }
}

pub enum ChordKind {
    Toggle,
    NewTab,
}

/// The built-in toggle fallback's name, in one place so the load-time warning
/// and the runtime registration warning cannot drift.
pub const DEFAULT_TOGGLE_NAME: &str = "Option+Space";

pub fn chord_name(c: Chord) -> String {
    let mut s = String::new();
    for (on, name) in [
        (c.ctrl, "Ctrl+"),
        (c.option, "Option+"),
        (c.shift, "Shift+"),
        (c.cmd, "Cmd+"),
    ] {
        if on {
            s.push_str(name);
        }
    }
    s.push_str(&match c.key_code {
        49 => "Space".into(),
        50 => "`".into(),
        36 => "Return".into(),
        51 => "Backspace".into(),
        17 => "T".into(),
        k => format!("Key {k}"),
    });
    s
}

pub fn chord_index(c: Chord) -> usize {
    CHORD_PRESETS.iter().position(|x| *x == c).unwrap_or(0)
}

pub fn family_index(s: &str) -> usize {
    FONT_FAMILIES.iter().position(|x| *x == s).unwrap_or(0)
}

pub fn cursor_shape_index(s: CursorShape) -> usize {
    match s {
        CursorShape::Block => 0,
        CursorShape::Bar => 1,
        CursorShape::Underline => 2,
    }
}

pub fn cursor_shape_of(i: usize) -> CursorShape {
    match i {
        1 => CursorShape::Bar,
        2 => CursorShape::Underline,
        _ => CursorShape::Block,
    }
}

pub fn scrollback_index(n: usize) -> usize {
    SCROLLBACK_PRESETS
        .iter()
        .rposition(|x| *x <= n)
        .unwrap_or(0)
}

pub fn scrollback_of(i: usize) -> usize {
    SCROLLBACK_PRESETS[i.min(SCROLLBACK_PRESETS.len() - 1)]
}

pub fn palette_presets(c: Color) -> Vec<Color> {
    let start = PALETTE_PRESETS
        .iter()
        .position(|x| *x == c)
        .map_or(0, |i| i + 1);
    (0..PALETTE_PRESETS.len())
        .map(|i| PALETTE_PRESETS[(start + i) % PALETTE_PRESETS.len()])
        .collect()
}

pub fn key_name(k: &str) -> String {
    format!("BNT.{k}")
}

pub fn channel_key(p: &str, c: &str) -> String {
    format!("{p}.{c}")
}

pub fn ansi_key(i: usize) -> String {
    format!("ansi.{i}")
}

#[cfg(test)]
mod tests {
    use super::*;
    // Same-crate only, for proposal-policy tests; nothing in `effects` depends on
    // the settings store, so this adds no AppKit or `NSUserDefaults` to these tests.
    use crate::effects::SettingsState;

    #[test]
    fn legacy_and_name_roundtrip() {
        let mut p = Preferences::new();
        p.insert(key_name("fontFamily"), PrefValue::Integer(2));
        assert_eq!(decode_font_family(&p), "Monaco");
        p.insert(
            key_name("fontFamilyName"),
            PrefValue::Text("Custom Mono".into()),
        );
        assert_eq!(decode_font_family(&p), "Custom Mono");
        let s = Settings {
            font_family: "Custom Mono".into(),
            scrollback_lines: 1234,
            ..Settings::default()
        };
        let (actual, w) = Settings::from_preferences(&s.preferences(), 1920.0);
        assert_eq!(s, actual);
        assert!(w.is_empty());
    }

    #[test]
    fn field_validation() {
        let mut s = Settings::default();
        assert!(s.validate(1920.0).is_ok());
        s.font_size = f64::NAN;
        assert!(s.validate(1920.0).is_err());
        s.font_size = 14.0;
        s.background = Color::new(f64::INFINITY, 0.0, 0.0);
        assert!(s.validate(1920.0).is_err());
    }

    #[test]
    fn reserved_and_conflicting_chords() {
        assert!(validate_toggle_chord(chord(49, false, true, false, false)).is_err());
        assert!(validate_toggle_chord(DEFAULT_NEW_TAB).is_err());
        assert!(validate_chords(DEFAULT_TOGGLE, DEFAULT_TOGGLE).is_err());
        assert_eq!(chord_of(49, chord_mask(DEFAULT_TOGGLE)), DEFAULT_TOGGLE);
    }

    #[test]
    fn malformed_preferences_warn_without_nan() {
        let mut p = Preferences::new();
        p.insert(key_name("fontSize"), PrefValue::Number(f64::NAN));
        let (s, w) = Settings::from_preferences(&p, 1920.0);
        assert!(s.font_size.is_finite());
        assert_eq!(w.len(), 1);
    }

    /// A legacy build could persist a toggle that macOS or a local command owns.
    /// Load must keep that value VERBATIM (so a later release that supports it
    /// does not silently lose the user's intent), report it once, and let the
    /// runtime fall back — never make the invalid chord active.
    #[test]
    fn legacy_invalid_toggle_is_stored_raw_with_runtime_fallback() {
        for legacy in [
            chord(49, false, true, false, false), // Cmd+Space (Spotlight)
            chord(49, false, false, false, true), // Ctrl+Space
            DEFAULT_NEW_TAB,                      // Cmd+T (local new tab)
        ] {
            let s = Settings {
                toggle_chord: legacy,
                ..Settings::default()
            };
            let mut prefs = s.preferences();
            // A legacy plist may hold either typed form; decode accepts both.
            prefs.insert(
                key_name("toggle.key"),
                PrefValue::Number(legacy.key_code as f64),
            );
            prefs.insert(
                key_name("toggle.mods"),
                PrefValue::Number(chord_mask(legacy) as f64),
            );
            let (loaded, warnings) = Settings::from_preferences(&prefs, 1920.0);

            assert_eq!(loaded.toggle_chord, legacy, "raw intent must survive");
            assert!(
                validate_toggle_chord(loaded.toggle_chord).is_err(),
                "{} must stay unusable",
                chord_name(legacy)
            );
            assert!(
                warnings.iter().any(|w| w.contains("cannot be registered")),
                "status must be reported: {warnings:?}"
            );
            // Raw intent is still what gets re-encoded, so a save round-trip does
            // not overwrite the user's choice with the fallback.
            let saved = loaded.preferences();
            assert_eq!(
                saved[&key_name("toggle.key")],
                PrefValue::Integer(legacy.key_code as i64)
            );
            assert_eq!(
                saved[&key_name("toggle.mods")],
                PrefValue::Integer(chord_mask(legacy) as i64)
            );
            assert_eq!(
                Settings::from_preferences(&saved, 1920.0).0,
                Settings::from_preferences(&prefs, 1920.0).0
            );

            let (toggle, local) = loaded.effective_chords();
            assert_eq!(toggle, DEFAULT_TOGGLE, "runtime falls back to Option+Space");
            assert!(validate_toggle_chord(toggle).is_ok());
            assert!(validate_local_chord(local).is_ok());
            assert_ne!(toggle, local, "fallback pair cannot duplicate");
            // The stored value is unusable, so `validate` must not reject the
            // bundle on chord grounds — geometry/colors/history stay editable.
            assert!(loaded.validate(1920.0).is_ok());
            assert_eq!(
                chord_status(legacy, ChordKind::Toggle, DEFAULT_TOGGLE),
                format!(
                    "Stored toggle {} cannot be registered; {DEFAULT_TOGGLE_NAME} is active until you choose a new one",
                    chord_name(legacy)
                )
            );
        }
    }

    /// A legacy invalid toggle must not lock out unrelated edits, while an
    /// explicit chord edit still has to be usable and non-duplicating.
    #[test]
    fn unrelated_edits_survive_a_legacy_toggle_and_chord_edits_still_validate() {
        let current = Settings {
            toggle_chord: chord(49, false, true, false, false), // invalid Cmd+Space
            open_width: 1600.0,
            ..Settings::default()
        };
        let state = SettingsState::new(current.clone(), vec![]);

        let mut proposal = current.clone();
        proposal.font_size = 18.0;
        proposal.background = Color::new(0.1, 0.2, 0.3);
        proposal.scrollback_lines = 20_000;
        assert!(
            state
                .validate_proposal(&proposal, EditPhase::Commit, 1920.0)
                .is_ok(),
            "a legacy toggle must not block font/color/history edits"
        );

        // An explicit switch to another unusable global chord is rejected.
        let mut bad = current.clone();
        bad.toggle_chord = chord(49, false, false, false, true); // Ctrl+Space
        assert!(
            state
                .validate_proposal(&bad, EditPhase::Commit, 1920.0)
                .is_err(),
            "a chord change needs a registrable replacement"
        );

        // A valid replacement is accepted, and the runtime pair is usable.
        let mut good = current.clone();
        good.toggle_chord = chord(50, true, false, false, false); // Option+`
        assert!(
            state
                .validate_proposal(&good, EditPhase::Commit, 1920.0)
                .is_ok()
        );
        let (toggle, local) = good.effective_chords();
        assert_eq!(toggle, chord(50, true, false, false, false));
        assert_ne!(local, toggle);

        // Preview must never re-register, whichever chord moved.
        assert!(
            state
                .validate_proposal(&good, EditPhase::Preview, 1920.0)
                .is_err()
        );

        // The new-tab chord cannot become the toggle the runtime registers, even
        // though the stored toggle is itself unusable.
        let mut dupe = current.clone();
        dupe.new_tab_chord = DEFAULT_TOGGLE;
        assert!(
            state
                .validate_proposal(&dupe, EditPhase::Commit, 1920.0)
                .is_err(),
            "no duplicate global/local chord"
        );
    }

    /// A stored new-tab chord that is unusable, or that equals the effective
    /// toggle, is kept raw and replaced at runtime by a terminal-local chord.
    #[test]
    fn new_tab_conflicts_fall_back_without_touching_storage() {
        let s = Settings {
            new_tab_chord: chord(17, false, true, false, false), // Cmd+T == DEFAULT_NEW_TAB
            ..Settings::default()
        };
        let (toggle, local) = s.effective_chords();
        assert_eq!(toggle, DEFAULT_TOGGLE);
        assert_eq!(local, DEFAULT_NEW_TAB);

        // Identical to the toggle: stored stays, runtime picks a non-conflicting
        // terminal-local chord.
        let dupe = Settings {
            new_tab_chord: DEFAULT_TOGGLE,
            ..Settings::default()
        };
        let (toggle, local) = dupe.effective_chords();
        assert_eq!(toggle, DEFAULT_TOGGLE);
        assert_ne!(local, toggle);
        assert!(validate_local_chord(local).is_ok());
        assert_eq!(
            chord_status(dupe.new_tab_chord, ChordKind::NewTab, toggle),
            format!(
                "Stored new-tab {} is the toggle shortcut; terminal-local Cmd+T is used instead",
                chord_name(DEFAULT_TOGGLE)
            )
        );
        assert!(dupe.validate(1920.0).is_ok());

        // Load reports the conflict once and keeps the raw value.
        let (loaded, warnings) = Settings::from_preferences(&dupe.preferences(), 1920.0);
        assert_eq!(loaded.new_tab_chord, DEFAULT_TOGGLE);
        assert!(
            warnings.iter().any(|w| w.contains("toggle shortcut")),
            "{warnings:?}"
        );

        // An unusable local chord (reserved Cmd+key) reports and falls back.
        let reserved = Settings {
            new_tab_chord: chord(19, false, true, false, false), // Cmd+W
            ..Settings::default()
        };
        assert!(validate_local_chord(reserved.new_tab_chord).is_err());
        let (_, local) = reserved.effective_chords();
        assert!(validate_local_chord(local).is_ok());
        let (_, warnings) = Settings::from_preferences(&reserved.preferences(), 1920.0);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("native or local command")),
            "{warnings:?}"
        );

        for fallback in LOCAL_CHORD_FALLBACKS {
            assert!(validate_local_chord(fallback).is_ok());
            assert_ne!(fallback, DEFAULT_TOGGLE);
        }
    }

    /// FIX C: a canvas SHRINK is a geometry situation, not a settings error. The
    /// raw preference survives, the effective width clamps, and unrelated edits
    /// keep working; the load clamp never rewrites the stored preference.
    #[test]
    fn display_shrink_clamps_effectively_without_rejecting_or_rewriting() {
        let s = Settings {
            open_width: 1600.0,
            ..Settings::default()
        };
        assert_eq!(s.effective_open_width(1600.0), 1600.0);
        assert_eq!(s.effective_open_width(1200.0), 1200.0);
        assert_eq!(s.open_width, 1600.0, "raw preference preserved");
        assert_eq!(
            s.effective_open_width(NOTCH_WIDTH - 10.0),
            NOTCH_WIDTH - 10.0
        );
        assert_eq!(s.effective_open_width(f64::NAN), PANEL_WIDTH);

        // Decoding on a small canvas warns but keeps the raw value.
        let (shrunk, warnings) = Settings::from_preferences(&s.preferences(), 1200.0);
        assert_eq!(shrunk.open_width, 1600.0);
        assert_eq!(shrunk.effective_open_width(1200.0), 1200.0);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("exceeds the current display")),
            "{warnings:?}"
        );
        // Re-encoding must not persist the clamped value.
        assert_eq!(shrunk.preferences(), s.preferences());

        let state = SettingsState::new(shrunk.clone(), vec![]);
        let mut proposal = shrunk.clone();
        proposal.font_size = 16.0;
        proposal.foreground = Color::new(0.9, 0.9, 0.8);
        assert!(
            state
                .validate_proposal(&proposal, EditPhase::Commit, 1200.0)
                .is_ok(),
            "a shrunk display must not lock out unrelated edits"
        );

        // Bounds stay safe at any canvas: a degenerate canvas resolves to the
        // documented fallback canvas, never to a negative or NaN width.
        assert_eq!(s.effective_open_width(0.0), PANEL_WIDTH);
        assert!(s.effective_open_width(-500.0).is_finite());
        let mut too_narrow = s.clone();
        too_narrow.open_width = 10.0;
        assert!(too_narrow.validate(1920.0).is_err());
        too_narrow.open_width = f64::NAN;
        assert!(too_narrow.validate(1920.0).is_err());
    }
}
