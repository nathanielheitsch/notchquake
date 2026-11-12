//! Pure key routing. Native windows never enter the terminal byte encoder.
use crate::settings::Chord;

#[derive(Clone, Copy, Default)]
pub struct Context {
    pub terminal_open: bool,
    pub panel_is_key: bool,
    pub event_is_panel: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Modifiers {
    pub command: bool,
    pub option: bool,
    pub control: bool,
    pub shift: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    NewTab,
    CloseTab,
    SelectTab(usize),
    Paste,
    CopyUnavailable,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    Command(Command),
    Bytes(Vec<u8>),
    PassThrough,
}

pub fn classify(context: Context, key: u16, text: &str, m: Modifiers, new_tab: Chord) -> Route {
    if !(context.terminal_open && context.panel_is_key && context.event_is_panel) {
        return Route::PassThrough;
    }
    let chord = Chord {
        key_code: key,
        cmd: m.command,
        option: m.option,
        ctrl: m.control,
        shift: m.shift,
    };
    if chord == new_tab {
        return Route::Command(Command::NewTab);
    }
    if m.command {
        if m.option || m.control || m.shift {
            return Route::PassThrough;
        }
        return match key {
            13 => Route::Command(Command::CloseTab),
            9 => Route::Command(Command::Paste),
            8 => Route::Command(Command::CopyUnavailable),
            // Carbon virtual key codes for 1…8 (not contiguous).
            18 | 19 | 20 | 21 | 23 | 22 | 26 | 28 => {
                let index = [18, 19, 20, 21, 23, 22, 26, 28]
                    .iter()
                    .position(|k| *k == key)
                    .unwrap();
                Route::Command(Command::SelectTab(index))
            }
            // Cmd+Comma, Cmd+Q and unknown commands belong to AppKit/menu routing.
            _ => Route::PassThrough,
        };
    }
    if m.control
        && let Some(b) = text.as_bytes().first()
        && (0x40..0x7f).contains(b)
    {
        return Route::Bytes(vec![b & 0x1f]);
    }
    let special: Option<&[u8]> = match key {
        53 => Some(b"\x1b"),
        51 => Some(b"\x7f"),
        36 | 76 => Some(b"\r"),
        48 => Some(b"\t"),
        126 => Some(b"\x1b[A"),
        125 => Some(b"\x1b[B"),
        124 => Some(b"\x1b[C"),
        123 => Some(b"\x1b[D"),
        117 => Some(b"\x1b[3~"),
        _ => None,
    };
    if let Some(bytes) = special {
        return Route::Bytes(bytes.to_vec());
    }
    if text.is_empty() || text.chars().any(|c| ('\u{f700}'..='\u{f8ff}').contains(&c)) {
        return Route::PassThrough;
    }
    let mut bytes = text.as_bytes().to_vec();
    if m.option && bytes.len() == 1 && (0x20..0x80).contains(&bytes[0]) {
        bytes.insert(0, 0x1b);
    }
    Route::Bytes(bytes)
}

/// Deliberate paste; strip ESC from bracketed content so pasted data cannot end the wrapper.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    if bracketed {
        format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', "")).into_bytes()
    } else {
        text.replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::DEFAULT_NEW_TAB;

    fn terminal() -> Context {
        Context {
            terminal_open: true,
            panel_is_key: true,
            event_is_panel: true,
        }
    }

    #[test]
    fn native_and_closed_contexts_pass_through() {
        for c in [
            Context::default(),
            Context {
                panel_is_key: false,
                ..terminal()
            },
            Context {
                event_is_panel: false,
                ..terminal()
            },
        ] {
            assert_eq!(
                classify(c, 0, "a", Modifiers::default(), DEFAULT_NEW_TAB),
                Route::PassThrough
            );
        }
    }

    #[test]
    fn command_chords_never_leak_to_shell() {
        let modifiers = Modifiers {
            command: true,
            ..Modifiers::default()
        };
        for key in 0..128 {
            assert!(!matches!(
                classify(terminal(), key, "a", modifiers, DEFAULT_NEW_TAB),
                Route::Bytes(_)
            ));
        }
        assert_eq!(
            classify(terminal(), 17, "t", modifiers, DEFAULT_NEW_TAB),
            Route::Command(Command::NewTab)
        );
    }

    #[test]
    fn ordinary_special_control_and_paste() {
        assert_eq!(
            classify(terminal(), 13, "w", Modifiers::default(), DEFAULT_NEW_TAB),
            Route::Bytes(b"w".to_vec())
        );
        assert_eq!(
            classify(
                terminal(),
                126,
                "\u{f700}",
                Modifiers::default(),
                DEFAULT_NEW_TAB
            ),
            Route::Bytes(b"\x1b[A".to_vec())
        );
        assert_eq!(
            classify(
                terminal(),
                0,
                "a",
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
                DEFAULT_NEW_TAB,
            ),
            Route::Bytes(vec![1])
        );
        assert_eq!(paste_bytes("a\r\nb", false), b"a\rb");
        assert_eq!(paste_bytes("a\x1b[201~", true), b"\x1b[200~a[201~\x1b[201~");
    }
}
