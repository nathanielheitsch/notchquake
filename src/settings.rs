//! Runtime settings for the notch panel: geometry, font, colors, scrollback,
//! hotkey chords, bell.
//!
//! Every knob is plain, copyable data so the whole bundle lives in one
//! `Cell<Settings>` inside the view's ivars (see `notch_view.rs`). Defaults
//! reproduce today's compile-time constants exactly, so an app with no stored
//! preferences paints identically to the pre-settings build.
//!
//! Persistence is `NSUserDefaults` (`objc2_foundation`): `standardUserDefaults`
//! (the app's own search list — no suite name, so there is no `-init` instance
//! to keep alive) with every key prefixed `BNT.`. There is no serde
//! dependency in this project, so a color is three `setDouble:` channels
//! (`BNT.bg.r`) and a chord is two integers (`BNT.toggle.key` +
//! `BNT.toggle.mods`, the Carbon modifier mask from
//! `hotkey.rs::modifier_mask`). Enum-ish knobs (font family, cursor shape,
//! scrollback) store an INDEX into the preset lists below, so every stored
//! value goes through a `setInteger:`/`setDouble:`/`setBool:` setter — the
//! only unsafe `NSUserDefaults` setter (`setObject:forKey:`) is never used.
//! `setDouble:`/`setInteger:`/`setBool:` persist asynchronously on their own;
//! `synchronize()` is deprecated and not needed.

use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_foundation::CGFloat;
use objc2_foundation::MainThreadMarker;
use objc2_foundation::{NSInteger, NSString, NSUserDefaults};

use crate::consts::{
    CLOSED_HEIGHT, DEFAULT_SCROLLBACK_LINES, NOTCH_WIDTH, OPEN_HEIGHT, PANEL_HEIGHT, PANEL_WIDTH,
    SPACE_KEY_CODE,
};
use crate::font::FONT_SIZE;
use core::ffi::c_double;

/// An sRGB channel as a 0..255 integer component divided into 0.0..1.0 — the
/// form the palette tables in `render.rs` and `terminal.rs` are written in.
const fn channel(component: u8) -> CGFloat {
    component as CGFloat / 255.0
}

/// An sRGB color as three channels in 0.0..1.0 — the same tuple form `render.rs`
/// and `terminal.rs` already pass around.
#[derive(Clone, Copy, PartialEq)]
pub struct Color {
    r: CGFloat,
    g: CGFloat,
    b: CGFloat,
}

/// Cursor glyph shape, drawn by `render.rs::draw_grid` (a render read, not a
/// `Term` config).
#[derive(Clone, Copy, PartialEq)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
}

/// A Carbon hotkey chord: a macOS virtual key code (`NSEvent.keyCode()` space,
/// i.e. Carbon's Events.h table) plus which modifiers must be down.
#[derive(Clone, Copy, PartialEq)]
pub struct Chord {
    pub key_code: u16,
    pub option: bool,
    pub cmd: bool,
    pub shift: bool,
    pub ctrl: bool,
}

/// The settings bundle: all 16 knobs.
///
/// `font_family` is a `&'static str` rather than a `String`: `Cell<T>` requires
/// `T: Copy`, and a `String` field would make the whole bundle non-copyable
/// (lesson 17 — `Mut<T>` is unavailable in this toolchain, so a non-`Copy`
/// settings struct could not live in an ivar at all). The selectable families
/// are the `FONT_FAMILIES` literals, so a static name is enough.
#[derive(Clone, Copy)]
pub struct Settings {
    // Geometry — points. The upper bound is the notch display's logical width
    // (`screen.rs::display_width`) / `PANEL_HEIGHT`, sized at startup and re-read
    // on a display change — NOT a fixed 640×620 canvas: 640 is only the default
    // on the default 640pt-wide display. `Settings::load` clamps against the live
    // canvas so a wider display can actually be reached by the width slider.
    pub open_height: CGFloat,
    pub open_width: CGFloat,

    // Font
    pub font_family: &'static str,
    pub font_size: CGFloat,
    pub ligatures: bool,

    // Colors, each an sRGB triple in 0.0..1.0
    pub background: Color,
    pub foreground: Color,
    pub ansi: [Color; 16],
    pub cursor_color: Color,
    pub cursor_shape: CursorShape,

    // Scrollback
    pub scrollback_lines: usize,

    // Hotkey chords (Carbon virtual key code + modifier mask)
    pub toggle_chord: Chord,
    pub new_tab_chord: Chord,

    // Bell
    pub bell_enabled: bool,
    pub finished_notify_secs: CGFloat,
}

impl Color {
    /// The color as `(r, g, b)` in 0.0..1.0, the tuple form the renderers use.
    pub fn rgb(&self) -> (CGFloat, CGFloat, CGFloat) {
        (self.r, self.g, self.b)
    }

    /// The color as `(r, g, b, a)` with an opaque alpha, the form
    /// `CGColor::new_srgb` takes when a color well paints its swatch.
    pub fn rgba(&self) -> (CGFloat, CGFloat, CGFloat, CGFloat) {
        (self.r, self.g, self.b, 1.0)
    }

    /// The color as `#RRGGBB`, the form shown next to a color well.
    pub fn hex(&self) -> String {
        let byte = |channel: CGFloat| -> u8 {
            let scaled = (channel.clamp(0.0, 1.0) * 255.0 + 0.5).floor();
            scaled as u8
        };
        format!(
            "#{:02X}{:02X}{:02X}",
            byte(self.r),
            byte(self.g),
            byte(self.b)
        )
    }
}

/// Parse `#rrggbb` (or bare `rrggbb`) into a `Color`. A missing/invalid nibble
/// pair reads 0, which is what a truncated stored value should degrade to.
pub fn parse_hex(hex: &str) -> Color {
    let digits = if hex.starts_with("#") {
        hex[1..].trim()
    } else {
        hex.trim()
    };
    let nibble = |c: u8| -> u32 {
        match c {
            b'0'..b'9' => (c - b'0') as u32,
            b'a'..b'f' => (c - b'a') as u32 + 10,
            b'A'..b'F' => (c - b'A') as u32 + 10,
            _ => 0,
        }
    };
    let bytes = digits.as_bytes();
    let component = |i: usize| -> CGFloat {
        if i + 1 < bytes.len() {
            (nibble(bytes[i]) * 16 + nibble(bytes[i + 1])) as CGFloat / 255.0
        } else {
            0.0
        }
    };
    Color {
        r: component(0),
        g: component(2),
        b: component(4),
    }
}

/// The Carbon modifier mask for a chord — the same bit layout
/// `hotkey.rs::modifier_mask` builds (Cmd bit 8, Shift 9, Option 11, Control
/// 12, plus the active-flag bit 0 whenever any modifier is set). Duplicated
/// here so `Settings` stays plain data and `hotkey.rs` owns registration.
pub fn chord_mask(chord: Chord) -> u32 {
    let mut mask = 0u32;
    if chord.cmd {
        mask |= 1 << 8;
    }
    if chord.shift {
        mask |= 1 << 9;
    }
    if chord.option {
        mask |= 1 << 11;
    }
    if chord.ctrl {
        mask |= 1 << 12;
    }
    if mask != 0 {
        mask |= 1 << 0;
    }
    mask
}

/// Rebuild a `Chord` from a Carbon key code plus the modifier mask written by
/// `chord_mask`.
pub fn chord_of(key_code: u16, mask: u32) -> Chord {
    Chord {
        key_code,
        option: mask & (1 << 11) != 0,
        cmd: mask & (1 << 8) != 0,
        shift: mask & (1 << 9) != 0,
        ctrl: mask & (1 << 12) != 0,
    }
}

/// The chords the settings panel can cycle through. Every entry is resolvable
/// without an Input-Monitoring grant (Carbon `RegisterEventHotKey`) and uses
/// key codes from Carbon's virtual-key table (Events.h): Space 49, § 50,
/// Return 36, Delete/Backspace 51, T 0x11.
///
/// fn/globe chords are absent on principle: Carbon cannot deliver them, and
/// "record an arbitrary chord" would need a global key-event tap, which is
/// gated behind a grant this adhoc-signed bundle never gets.
pub const CHORD_PRESETS: [Chord; 10] = [
    Chord {
        key_code: SPACE_KEY_CODE,
        option: true,
        cmd: false,
        shift: false,
        ctrl: false,
    },
    Chord {
        key_code: SPACE_KEY_CODE,
        option: false,
        cmd: true,
        shift: false,
        ctrl: false,
    },
    Chord {
        key_code: SPACE_KEY_CODE,
        option: false,
        cmd: false,
        shift: false,
        ctrl: true,
    },
    Chord {
        key_code: 50,
        option: true,
        cmd: false,
        shift: false,
        ctrl: false,
    },
    Chord {
        key_code: 50,
        option: false,
        cmd: true,
        shift: false,
        ctrl: false,
    },
    Chord {
        key_code: 50,
        option: false,
        cmd: false,
        shift: false,
        ctrl: true,
    },
    Chord {
        key_code: 36,
        option: true,
        cmd: false,
        shift: false,
        ctrl: false,
    },
    Chord {
        key_code: 36,
        option: false,
        cmd: true,
        shift: false,
        ctrl: false,
    },
    Chord {
        key_code: 51,
        option: true,
        cmd: false,
        shift: false,
        ctrl: false,
    },
    Chord {
        key_code: 0x11,
        option: false,
        cmd: true,
        shift: false,
        ctrl: false,
    },
];

/// Human-readable name for a chord, as shown in the settings panel.
pub fn chord_name(chord: Chord) -> String {
    let mods = if chord.ctrl {
        "Ctrl+"
    } else if chord.cmd {
        "Cmd+"
    } else if chord.option {
        "Option+"
    } else if chord.shift {
        "Shift+"
    } else {
        ""
    };
    let named = match chord.key_code {
        SPACE_KEY_CODE => "Space",
        50 => "\u{00A7}", // §
        36 => "Enter",
        51 => "Backspace",
        0x11 => "T",
        _ => "",
    };
    let key = if named.is_empty() {
        format!("key {}", chord.key_code)
    } else {
        named.to_string()
    };
    mods.to_string() + &key
}

/// The index of a chord in `CHORD_PRESETS`, or 0 when it is not one of them
/// (a hand-tuned stored chord still renders as its own name).
pub fn chord_index(chord: Chord) -> usize {
    let at = CHORD_PRESETS.iter().position(|preset| *preset == chord);
    at.unwrap_or(0)
}

/// The font families the settings panel can cycle through: only names that
/// actually resolve (`font.rs::monospace_with` falls back to Menlo when one
/// does not).
pub const FONT_FAMILIES: [&'static str; 5] =
    ["Fira Code", "Menlo", "Monaco", "SF Mono", "Andale Mono"];

/// The scrollback values the settings panel steps through (lines of history).
pub const SCROLLBACK_PRESETS: [usize; 6] = [0, 1000, 5000, 10000, 50000, 100000];

/// The named colors a color well cycles through — the cheapest honest picker
/// without an `NSColorPanel` (which would need app activation).
pub const PALETTE_PRESETS: [Color; 12] = [
    Color {
        r: channel(0x00),
        g: channel(0x00),
        b: channel(0x00),
    },
    Color {
        r: channel(0x18),
        g: channel(0x18),
        b: channel(0x18),
    },
    Color {
        r: channel(0x14),
        g: channel(0x14),
        b: channel(0x14),
    },
    Color {
        r: channel(0x2B),
        g: channel(0x2B),
        b: channel(0x2B),
    },
    Color {
        r: channel(0x3B),
        g: channel(0x3C),
        b: channel(0x3B),
    },
    Color {
        r: channel(0x4C),
        g: channel(0x72),
        b: channel(0x9E),
    },
    Color {
        r: channel(0x70),
        g: channel(0x98),
        b: channel(0x78),
    },
    Color {
        r: channel(0xD8),
        g: channel(0xA8),
        b: channel(0x4A),
    },
    Color {
        r: channel(0xA8),
        g: channel(0x68),
        b: channel(0x94),
    },
    Color {
        r: channel(0x68),
        g: channel(0xA8),
        b: channel(0xC4),
    },
    Color {
        r: channel(0xD8),
        g: channel(0xD8),
        b: channel(0xD8),
    },
    Color {
        r: channel(0xFF),
        g: channel(0xFF),
        b: channel(0xFF),
    },
];

/// The ring of colors a color well steps through, rotated so it starts AFTER
/// `current` — a click must always change the color, even when the well already
/// holds a preset.
pub fn palette_presets(current: Color) -> Vec<Color> {
    let at = PALETTE_PRESETS.iter().position(|c| *c == current);
    let start = at.map(|i| i + 1).unwrap_or(0);
    let count = PALETTE_PRESETS.len();
    let mut out = PALETTE_PRESETS.to_vec();
    for i in 0..count {
        out[i] = PALETTE_PRESETS[(start + i) % count];
    }
    out
}

/// Today's hardcoded values, verbatim: `OPEN_HEIGHT`, `PANEL_WIDTH`, "Fira
/// Code", `FONT_SIZE`, ligatures on, black bg, `#D8D8D8` fg, the alacritty
/// dark palette `terminal.rs::default_color` ships, the 0.75-gray cursor block
/// (`render.rs` draws it at `CURSOR_ALPHA`), `DEFAULT_SCROLLBACK_LINES`,
/// Option+Space / Cmd+T, bell on, 5s.
impl Settings {
    /// Today's constants, so a fresh install paints exactly as before.
    pub fn default() -> Settings {
        Settings {
            open_height: OPEN_HEIGHT,
            open_width: PANEL_WIDTH,
            font_family: FONT_FAMILIES[0],
            font_size: FONT_SIZE,
            ligatures: true,
            background: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
            },
            foreground: Color {
                r: channel(0xD8),
                g: channel(0xD8),
                b: channel(0xD8),
            },
            ansi: [
                Color {
                    r: channel(0x18),
                    g: channel(0x18),
                    b: channel(0x18),
                },
                Color {
                    r: channel(0xC4),
                    g: channel(0x3C),
                    b: channel(0x3B),
                },
                Color {
                    r: channel(0x70),
                    g: channel(0x98),
                    b: channel(0x78),
                },
                Color {
                    r: channel(0xD8),
                    g: channel(0xA8),
                    b: channel(0x4A),
                },
                Color {
                    r: channel(0x4C),
                    g: channel(0x72),
                    b: channel(0x9E),
                },
                Color {
                    r: channel(0xA8),
                    g: channel(0x68),
                    b: channel(0x94),
                },
                Color {
                    r: channel(0x68),
                    g: channel(0xA8),
                    b: channel(0xC4),
                },
                Color {
                    r: channel(0xD8),
                    g: channel(0xD8),
                    b: channel(0xD8),
                },
                Color {
                    r: channel(0x60),
                    g: channel(0x60),
                    b: channel(0x60),
                },
                Color {
                    r: channel(0xE4),
                    g: channel(0x5C),
                    b: channel(0x5B),
                },
                Color {
                    r: channel(0x98),
                    g: channel(0xC0),
                    b: channel(0xA0),
                },
                Color {
                    r: channel(0xF0),
                    g: channel(0xC0),
                    b: channel(0x62),
                },
                Color {
                    r: channel(0x6C),
                    g: channel(0x92),
                    b: channel(0xBE),
                },
                Color {
                    r: channel(0xD8),
                    g: channel(0x98),
                    b: channel(0xC4),
                },
                Color {
                    r: channel(0x88),
                    g: channel(0xC8),
                    b: channel(0xE4),
                },
                Color {
                    r: channel(0xF8),
                    g: channel(0xF8),
                    b: channel(0xF8),
                },
            ],
            cursor_color: Color {
                r: 0.75,
                g: 0.75,
                b: 0.75,
            },
            cursor_shape: CursorShape::Block,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
            toggle_chord: CHORD_PRESETS[0],
            new_tab_chord: CHORD_PRESETS[CHORD_PRESETS.len() - 1],
            bell_enabled: true,
            finished_notify_secs: 5.0,
        }
    }
}

/// The store every knob is read from and written to.
///
/// Normally this is `standardUserDefaults()` — the app's own search list, no
/// suite name, so there is no `-init` instance to keep alive. Verification
/// builds set `BNT_DEFAULTS_SUITE` to divert every read AND write into a
/// throwaway domain, so driving the settings window cannot mutate the real
/// `~/Library/Preferences/local.bnt.plist`. (A click pass that persisted
/// `BNT.toggle.mods = 257` — Cmd+Space — into the real plist once left the
/// app unreachable by hotkey: Option+Space no longer matched the registered
/// chord, and Cmd+Space is swallowed by Spotlight.)
///
/// `initWithSuiteName:` returns `None` for an empty name, which reads as "no
/// store" — the caller then keeps `Settings::default()` rather than
/// persisting anywhere.
fn defaults_store() -> Option<Retained<NSUserDefaults>> {
    match std::env::var("BNT_DEFAULTS_SUITE") {
        Ok(suite) if !suite.is_empty() => {
            let name = NSString::from_str(&suite);
            NSUserDefaults::initWithSuiteName(NSUserDefaults::alloc(), Some(&name))
        }
        _ => Some(NSUserDefaults::standardUserDefaults()),
    }
}

/// The width the panel canvas is actually sized to: the NOTCH display's
/// logical width, read through `window.rs::canvas_size` — the SAME entry point
/// that sizes the panel window (`create_panel`) and `notch_view.rs::ivars.canvas`.
/// Not a max over every screen: a wider secondary display must not raise the
/// slider bound past the canvas the silhouette is painted into, or the panel
/// gets clipped and the rail lies. `pub` so
/// `settings_window.rs::width_max` can feed the SAME value to the slider rail
/// and the `apply_knob_value` clamp as this module's `load` clamp uses.
pub fn display_width() -> CGFloat {
    match MainThreadMarker::new() {
        Some(mtm) => crate::window::canvas_size(mtm),
        // Off the main thread there is no canvas to measure; fall back to the
        // compile-time default, which every caller already maxes its range
        // against.
        _ => PANEL_WIDTH,
    }
}

impl Settings {
    /// Read the stored preferences, falling back to `default()` per knob.
    /// `integerForKey:`/`doubleForKey:`/`boolForKey:` read 0 for an absent key,
    /// so every knob is probed for presence (`objectForKey:`) before it is
    /// accepted, and every numeric knob is clamped to its legal range.
    pub fn load() -> Settings {
        let base = Settings::default();
        let Some(defaults) = defaults_store() else {
            return base;
        };

        let has_key = |key: &str| -> bool {
            let name = NSString::from_str(&key_name(key));
            defaults.objectForKey(&name).is_some()
        };
        let get_double = |key: &str| -> CGFloat {
            let name = NSString::from_str(&key_name(key));
            defaults.doubleForKey(&name) as CGFloat
        };
        let get_bool = |key: &str| -> bool {
            let name = NSString::from_str(&key_name(key));
            defaults.boolForKey(&name)
        };
        // An index knob clamps into range, so a stale/garbage stored index
        // cannot panic on the preset list.
        let get_index = |key: &str, count: usize| -> usize {
            if !has_key(key) {
                return 0;
            }
            let name = NSString::from_str(&key_name(key));
            let stored = defaults.integerForKey(&name);
            if stored <= 0 {
                0
            } else {
                (stored as usize).min(count.saturating_sub(1))
            }
        };
        // A stored color is only trusted when all three channels are present,
        // so a half-written color cannot mix default + stored channels.
        let get_color = |prefix: &str, fallback: Color| -> Color {
            if has_key(&channel_key(prefix, "r"))
                && has_key(&channel_key(prefix, "g"))
                && has_key(&channel_key(prefix, "b"))
            {
                Color {
                    r: get_double(&channel_key(prefix, "r")).clamp(0.0, 1.0),
                    g: get_double(&channel_key(prefix, "g")).clamp(0.0, 1.0),
                    b: get_double(&channel_key(prefix, "b")).clamp(0.0, 1.0),
                }
            } else {
                fallback
            }
        };
        let get_chord = |prefix: &str, fallback: Chord| -> Chord {
            if has_key(&channel_key(prefix, "key")) && has_key(&channel_key(prefix, "mods")) {
                let name = NSString::from_str(&key_name(&channel_key(prefix, "key")));
                let code = defaults.integerForKey(&name) as u16;
                let name = NSString::from_str(&key_name(&channel_key(prefix, "mods")));
                chord_of(code, defaults.integerForKey(&name) as u32)
            } else {
                fallback
            }
        };
        let get_sized = |key: &str, lo: CGFloat, hi: CGFloat, fallback: CGFloat| -> CGFloat {
            if has_key(key) {
                get_double(key).clamp(lo, hi)
            } else {
                fallback
            }
        };

        let mut palette = base.ansi;
        for i in 0..16 {
            palette[i] = get_color(&ansi_key(i), base.ansi[i]);
        }

        Settings {
            open_height: get_sized(
                "openHeight",
                CLOSED_HEIGHT + 80.0,
                PANEL_HEIGHT - 20.0,
                base.open_height,
            ),
            open_width: get_sized(
                "openWidth",
                NOTCH_WIDTH,
                PANEL_WIDTH.max(display_width()),
                base.open_width,
            ),
            font_family: FONT_FAMILIES[get_index("fontFamily", FONT_FAMILIES.len())],
            font_size: get_sized("fontSize", 8.0, 32.0, base.font_size),
            ligatures: !has_key("ligatures") || get_bool("ligatures"),
            background: get_color("bg", base.background),
            foreground: get_color("fg", base.foreground),
            ansi: palette,
            cursor_color: get_color("cursor", base.cursor_color),
            cursor_shape: cursor_shape_of(get_index("cursorShape", 3)),
            scrollback_lines: if has_key("scrollbackLines") {
                SCROLLBACK_PRESETS[get_index("scrollbackLines", SCROLLBACK_PRESETS.len())]
            } else {
                base.scrollback_lines
            },
            toggle_chord: get_chord("toggle", base.toggle_chord),
            new_tab_chord: get_chord("newTab", base.new_tab_chord),
            bell_enabled: !has_key("bellEnabled") || get_bool("bellEnabled"),
            finished_notify_secs: get_sized("notifySecs", 0.0, 3600.0, base.finished_notify_secs),
        }
    }

    /// Write every knob back to `NSUserDefaults` under the `BNT.` prefix.
    /// Called after a control click mutates the settings cell; the writes are
    /// cheap and persist asynchronously (`synchronize` is deprecated and
    /// unnecessary).
    pub fn save(&self) {
        let Some(defaults) = defaults_store() else {
            return;
        };

        let set_double = |key: &str, value: CGFloat| {
            let name = NSString::from_str(&key_name(key));
            defaults.setDouble_forKey(value as c_double, &name);
        };
        let set_bool = |key: &str, value: bool| {
            let name = NSString::from_str(&key_name(key));
            defaults.setBool_forKey(value, &name);
        };
        let set_index = |key: &str, value: usize| {
            let name = NSString::from_str(&key_name(key));
            defaults.setInteger_forKey(value as NSInteger, &name);
        };
        let set_color = |prefix: &str, color: Color| {
            set_double(&channel_key(prefix, "r"), color.r);
            set_double(&channel_key(prefix, "g"), color.g);
            set_double(&channel_key(prefix, "b"), color.b);
        };
        let set_chord = |prefix: &str, chord: Chord| {
            let name = NSString::from_str(&key_name(&channel_key(prefix, "key")));
            defaults.setInteger_forKey(chord.key_code as NSInteger, &name);
            let name = NSString::from_str(&key_name(&channel_key(prefix, "mods")));
            defaults.setInteger_forKey(chord_mask(chord) as NSInteger, &name);
        };

        set_double("openHeight", self.open_height);
        set_double("openWidth", self.open_width);
        set_index("fontFamily", family_index(self.font_family));
        set_double("fontSize", self.font_size);
        set_bool("ligatures", self.ligatures);
        set_color("bg", self.background);
        set_color("fg", self.foreground);
        for i in 0..16 {
            set_color(&ansi_key(i), self.ansi[i]);
        }
        set_color("cursor", self.cursor_color);
        set_index("cursorShape", cursor_shape_index(self.cursor_shape));
        set_index("scrollbackLines", scrollback_index(self.scrollback_lines));
        set_chord("toggle", self.toggle_chord);
        set_chord("newTab", self.new_tab_chord);
        set_bool("bellEnabled", self.bell_enabled);
        set_double("notifySecs", self.finished_notify_secs);
    }
}

/// The stored index for a cursor shape.
pub fn cursor_shape_index(shape: CursorShape) -> usize {
    match shape {
        CursorShape::Block => 0,
        CursorShape::Bar => 1,
        CursorShape::Underline => 2,
    }
}

/// The cursor shape stored at `index`.
pub fn cursor_shape_of(index: usize) -> CursorShape {
    match index {
        1 => CursorShape::Bar,
        2 => CursorShape::Underline,
        _ => CursorShape::Block,
    }
}

/// The stored index for a font family (unknown names store 0 = "Fira Code").
pub fn family_index(family: &str) -> usize {
    let at = FONT_FAMILIES.iter().position(|name| *name == family);
    at.unwrap_or(0)
}

/// The scrollback value stored at `index`.
pub fn scrollback_of(index: usize) -> usize {
    SCROLLBACK_PRESETS[index.min(SCROLLBACK_PRESETS.len() - 1)]
}

/// The stored index for a scrollback value (off-preset values snap to the
/// nearest preset at or below them).
pub fn scrollback_index(lines: usize) -> usize {
    let at = SCROLLBACK_PRESETS
        .iter()
        .rposition(|preset| *preset <= lines);
    at.unwrap_or(0)
}

/// The `BNT.<key>` preference key for a knob name.
pub fn key_name(key: &str) -> String {
    "BNT.".to_string() + key
}

/// The preference key SUFFIX for one channel of a color, e.g. `bg.r`. Every
/// caller wraps this in `key_name` exactly once (the `has_key`/`get_*`/`set_*
/// helpers already apply the `BNT.` prefix, so applying it here too would
/// double-prefix color keys and leave chord keys single-prefixed.
pub fn channel_key(prefix: &str, channel: &str) -> String {
    [prefix, ".", channel].join("")
}

/// The preference key for the i-th ANSI color, e.g. `BNT.ansi.3`.
pub fn ansi_key(index: usize) -> String {
    "ansi.".to_string() + &index.to_string()
}
