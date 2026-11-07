//! Keys to the bytes an xterm-256color shell expects (suzuri-view's
//! `keys.js` and `kitty.js`): the legacy encoding, plus the Kitty keyboard
//! pieces Go suzuri sends (CSI u for modified Enter and a few chords legacy
//! xterm can't express), and the copy/paste chords (`clipShortcut`).

use super::frame::{M_ALT_SCREEN, M_APP_CURSOR, M_KITTY_SHIFT};

/// A key, as the encoder sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum K {
    /// What the key types (one character, or a control character).
    Text(String),
    Enter,
    Backspace,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Escape,
    Tab,
    /// F1 to F24.
    F(u8),
}

/// A key press with its modifiers. `meta` is ⌘ on macOS (the Windows key
/// elsewhere); `base` is what the key types with no modifiers held (⌥A
/// types "å", its base is 'a'), as keys.js reads `e.code`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyIn {
    pub key: K,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub meta: bool,
    pub base: Option<char>,
}

impl KeyIn {
    #[cfg(test)]
    pub fn new(key: K) -> KeyIn {
        KeyIn {
            key,
            shift: false,
            alt: false,
            ctrl: false,
            meta: false,
            base: None,
        }
    }

    /// From Rune's key event. `Key::Command(c)` is ⌘ plus c on macOS and
    /// Ctrl plus c elsewhere.
    pub fn from_rune(ev: &rune::KeyEvent, base: Option<char>, is_mac: bool) -> Option<KeyIn> {
        use rune::Key;
        let m = ev.mods;
        let key = match &ev.key {
            Key::Text(t) => K::Text(t.clone()),
            Key::Command(c) => K::Text(c.to_string()),
            Key::Enter => K::Enter,
            Key::Backspace => K::Backspace,
            Key::Delete => K::Delete,
            Key::Insert => K::Insert,
            Key::Left => K::Left,
            Key::Right => K::Right,
            Key::Up => K::Up,
            Key::Down => K::Down,
            Key::Home => K::Home,
            Key::End => K::End,
            Key::PageUp => K::PageUp,
            Key::PageDown => K::PageDown,
            Key::Escape => K::Escape,
            Key::Tab => K::Tab,
            Key::Function(n) => K::F(*n),
            _ => return None,
        };
        let meta = if is_mac { m.cmd || m.logo } else { m.logo };
        let base = base.or_else(|| match &key {
            K::Text(t) if t.chars().count() == 1 => t.chars().next(),
            _ => None,
        });
        Some(KeyIn {
            key,
            shift: m.shift,
            alt: m.alt,
            ctrl: m.ctrl,
            meta,
            base: base.map(|c| c.to_ascii_lowercase()),
        })
    }

    /// The letter the key sits on, lowercase (keys.js's `e.code` "KeyX").
    fn letter(&self) -> Option<char> {
        self.base.filter(|c| c.is_ascii_lowercase())
    }
}

const CSI: &str = "\x1b[";
const SS3: &str = "\x1bO";

fn mod_param(k: &KeyIn) -> u32 {
    let m = 1 + k.shift as u32 + 2 * k.alt as u32 + 4 * k.ctrl as u32 + 8 * k.meta as u32;
    if m > 1 { m } else { 0 }
}

/// The app's live Kitty flags (CSI > flags u) from a frame's modes.
pub fn kitty_flags(modes: u32) -> u32 {
    (modes >> M_KITTY_SHIFT) & 31
}

fn kitty_active(modes: u32) -> bool {
    kitty_flags(modes) & (1 | 8) != 0
}

fn kitty_mods(shift: bool, alt: bool, ctrl: bool, sup: bool) -> u32 {
    1 + shift as u32 + 2 * alt as u32 + 4 * ctrl as u32 + 8 * sup as u32
}

fn csi_u(code: u32, mods: u32) -> String {
    if mods <= 1 {
        format!("\x1b[{code}u")
    } else {
        format!("\x1b[{code};{mods}u")
    }
}

/// Enter: CR unmodified; with modifiers CSI 13 ; mods u (how Claude Code
/// tells Shift+Enter from Enter), except a bare Alt+Enter outside the
/// protocol, which stays ESC CR.
pub fn encode_enter(modes: u32, shift: bool, alt: bool, ctrl: bool, sup: bool) -> String {
    if !shift && !alt && !ctrl && !sup {
        return "\r".into();
    }
    if !kitty_active(modes) && alt && !shift && !ctrl && !sup {
        return "\x1b\r".into();
    }
    csi_u(13, kitty_mods(shift, alt, ctrl, sup))
}

/// Chords full-screen apps get that have no legacy encoding: ⌘Z is SUB
/// (undo), ⌘⇧Z CSI 122;10u (redo), Ctrl+; ' . , / and Ctrl+Shift+letter
/// as CSI u.
fn alt_screen_chord(k: &KeyIn, is_mac: bool) -> Option<String> {
    let (ctrl, shift, alt) = (k.ctrl, k.shift, k.alt);
    let sup = is_mac && k.meta;
    let letter = k.letter();
    if sup && !ctrl && !alt && letter == Some('z') {
        return Some(if shift {
            csi_u('z' as u32, kitty_mods(true, false, false, true))
        } else {
            "\x1a".into()
        });
    }
    if ctrl && !sup && !alt && !shift {
        if let Some(p) = k.base.filter(|c| ";',./".contains(*c)) {
            return Some(csi_u(p as u32, kitty_mods(false, false, true, false)));
        }
    }
    if ctrl && shift && !sup && !alt {
        if let Some(l) = letter {
            return Some(csi_u(l as u32, kitty_mods(true, false, true, false)));
        }
    }
    None
}

/// The bytes for a key press, or None to leave it to the window.
pub fn encode(k: &KeyIn, modes: u32, is_mac: bool) -> Option<String> {
    let m = mod_param(k);
    if modes & M_ALT_SCREEN != 0
        && let Some(c) = alt_screen_chord(k, is_mac)
    {
        return Some(c);
    }
    // macOS: ⌘ chords belong to the app; Option is Meta (ESC prefix).
    if is_mac && k.meta {
        return None;
    }
    let cursor = match k.key {
        K::Up => Some('A'),
        K::Down => Some('B'),
        K::Right => Some('C'),
        K::Left => Some('D'),
        K::Home => Some('H'),
        K::End => Some('F'),
        _ => None,
    };
    if let Some(c) = cursor {
        if m > 0 {
            return Some(format!("{CSI}1;{m}{c}"));
        }
        let lead = if modes & M_APP_CURSOR != 0 { SS3 } else { CSI };
        return Some(format!("{lead}{c}"));
    }
    let tilde = match k.key {
        K::Insert => Some(2),
        K::Delete => Some(3),
        K::PageUp => Some(5),
        K::PageDown => Some(6),
        K::F(5) => Some(15),
        K::F(6) => Some(17),
        K::F(7) => Some(18),
        K::F(8) => Some(19),
        K::F(9) => Some(20),
        K::F(10) => Some(21),
        K::F(11) => Some(23),
        K::F(12) => Some(24),
        _ => None,
    };
    if let Some(t) = tilde {
        return Some(if m > 0 {
            format!("{CSI}{t};{m}~")
        } else {
            format!("{CSI}{t}~")
        });
    }
    if let K::F(n @ 1..=4) = k.key {
        let c = ['P', 'Q', 'R', 'S'][n as usize - 1];
        return Some(if m > 0 {
            format!("{CSI}1;{m}{c}")
        } else {
            format!("{SS3}{c}")
        });
    }
    match &k.key {
        K::Enter => return Some(encode_enter(modes, k.shift, k.alt, k.ctrl, false)),
        K::Backspace => {
            let mut s = String::new();
            if k.alt {
                s.push('\x1b');
            }
            s.push(if k.ctrl { '\x08' } else { '\x7f' });
            return Some(s);
        }
        K::Tab => {
            return Some(if k.shift {
                format!("{CSI}Z")
            } else {
                "\t".into()
            });
        }
        K::Escape => return Some("\x1b".into()),
        K::Text(_) => {}
        _ => return None,
    }
    let K::Text(t) = &k.key else { return None };
    let mut chars = t.chars();
    let (Some(ch), None) = (chars.next(), chars.next()) else {
        return None;
    };
    // A control character the platform already made (Ctrl+C as ETX).
    if (ch as u32) < 0x20 || ch == '\x7f' {
        return Some(ch.to_string());
    }
    if k.ctrl && !k.alt {
        let c = k.base.unwrap_or(ch).to_ascii_lowercase();
        if c.is_ascii_lowercase() {
            return Some(char::from(c as u8 - 96).to_string());
        }
        return match c {
            ' ' | '@' | '2' => Some("\x00".into()),
            '[' | '3' => Some("\x1b".into()),
            '\\' | '4' => Some("\x1c".into()),
            ']' | '5' => Some("\x1d".into()),
            '^' | '6' => Some("\x1e".into()),
            '_' | '-' | '7' => Some("\x1f".into()),
            _ => None,
        };
    }
    if k.alt {
        // Option on macOS composes (å for a): use the key's own letter so
        // Option works as Meta.
        let base = k.letter().unwrap_or(ch);
        let b = if k.shift {
            base.to_ascii_uppercase()
        } else {
            base
        };
        return Some(format!("\x1b{b}"));
    }
    Some(ch.to_string())
}

/// A copy or paste chord (Go suzuri's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    Copy,
    Paste,
    /// Ctrl+C with a selection off macOS: copies, and sends ^C when there
    /// turned out to be nothing to copy.
    CopyOrInterrupt,
}

/// ⌘C/⌘V on macOS; elsewhere Ctrl+Shift+C/V, Ctrl+Insert, Shift+Insert,
/// Ctrl+V, and Ctrl+C while there is a selection (otherwise it stays ^C).
pub fn clip_shortcut(k: &KeyIn, is_mac: bool, has_selection: bool) -> Option<Clip> {
    if k.alt {
        return None;
    }
    let l = k.letter();
    if is_mac {
        if !k.meta || k.ctrl || k.shift {
            return None;
        }
        return match l {
            Some('c') => Some(Clip::Copy),
            Some('v') => Some(Clip::Paste),
            _ => None,
        };
    }
    if k.meta {
        return None;
    }
    let ins = k.key == K::Insert;
    match (k.ctrl, k.shift) {
        (true, true) if l == Some('c') => Some(Clip::Copy),
        (true, true) if l == Some('v') => Some(Clip::Paste),
        (true, false) if ins => Some(Clip::Copy),
        (false, true) if ins => Some(Clip::Paste),
        (true, false) if l == Some('v') => Some(Clip::Paste),
        (true, false) if l == Some('c') && has_selection => Some(Clip::CopyOrInterrupt),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: K) -> KeyIn {
        KeyIn::new(k)
    }
    fn text(s: &str) -> KeyIn {
        let mut k = KeyIn::new(K::Text(s.into()));
        k.base = s.chars().next().map(|c| c.to_ascii_lowercase());
        k
    }
    fn enc(k: &KeyIn) -> Option<String> {
        encode(k, 0, false)
    }

    #[test]
    fn arrows_follow_app_cursor_mode() {
        assert_eq!(enc(&key(K::Up)).unwrap(), "\x1b[A");
        assert_eq!(encode(&key(K::Up), M_APP_CURSOR, false).unwrap(), "\x1bOA");
        let mut k = key(K::Left);
        k.shift = true;
        assert_eq!(enc(&k).unwrap(), "\x1b[1;2D");
        let mut k = key(K::End);
        k.ctrl = true;
        assert_eq!(encode(&k, M_APP_CURSOR, false).unwrap(), "\x1b[1;5F");
    }

    #[test]
    fn tilde_and_function_keys() {
        assert_eq!(enc(&key(K::Delete)).unwrap(), "\x1b[3~");
        assert_eq!(enc(&key(K::PageDown)).unwrap(), "\x1b[6~");
        assert_eq!(enc(&key(K::F(1))).unwrap(), "\x1bOP");
        assert_eq!(enc(&key(K::F(12))).unwrap(), "\x1b[24~");
        let mut k = key(K::F(5));
        k.alt = true;
        assert_eq!(enc(&k).unwrap(), "\x1b[15;3~");
        let mut k = key(K::F(2));
        k.ctrl = true;
        assert_eq!(enc(&k).unwrap(), "\x1b[1;5Q");
    }

    #[test]
    fn ctrl_letters_and_symbols() {
        let mut k = text("c");
        k.ctrl = true;
        assert_eq!(enc(&k).unwrap(), "\x03");
        for (c, want) in [
            ("@", "\x00"),
            (" ", "\x00"),
            ("[", "\x1b"),
            ("\\", "\x1c"),
            ("]", "\x1d"),
            ("^", "\x1e"),
            ("_", "\x1f"),
            ("-", "\x1f"),
        ] {
            let mut k = text(c);
            k.ctrl = true;
            assert_eq!(enc(&k).unwrap(), want, "ctrl+{c:?}");
        }
        let mut k = text("=");
        k.ctrl = true;
        assert_eq!(enc(&k), None);
        // A platform that already sends the control character.
        assert_eq!(enc(&text("\x03")).unwrap(), "\x03");
    }

    #[test]
    fn enter_and_its_chords() {
        assert_eq!(enc(&key(K::Enter)).unwrap(), "\r");
        let mut k = key(K::Enter);
        k.shift = true;
        assert_eq!(enc(&k).unwrap(), "\x1b[13;2u");
        let mut k = key(K::Enter);
        k.alt = true;
        assert_eq!(enc(&k).unwrap(), "\x1b\r");
        // Inside the Kitty protocol Alt+Enter is CSI u too.
        assert_eq!(encode(&k, 1 << M_KITTY_SHIFT, false).unwrap(), "\x1b[13;3u");
    }

    #[test]
    fn backspace_tab_escape() {
        assert_eq!(enc(&key(K::Backspace)).unwrap(), "\x7f");
        let mut k = key(K::Backspace);
        k.ctrl = true;
        assert_eq!(enc(&k).unwrap(), "\x08");
        k.alt = true;
        assert_eq!(enc(&k).unwrap(), "\x1b\x08");
        assert_eq!(enc(&key(K::Tab)).unwrap(), "\t");
        let mut k = key(K::Tab);
        k.shift = true;
        assert_eq!(enc(&k).unwrap(), "\x1b[Z");
        assert_eq!(enc(&key(K::Escape)).unwrap(), "\x1b");
    }

    #[test]
    fn option_is_meta() {
        // ⌥A types "å" on a US Mac; its base is 'a'.
        let mut k = KeyIn::new(K::Text("å".into()));
        k.alt = true;
        k.base = Some('a');
        assert_eq!(encode(&k, 0, true).unwrap(), "\x1ba");
        k.shift = true;
        assert_eq!(encode(&k, 0, true).unwrap(), "\x1bA");
        let mut k = text(".");
        k.alt = true;
        assert_eq!(enc(&k).unwrap(), "\x1b.");
    }

    #[test]
    fn command_on_mac_is_the_apps() {
        let mut k = text("k");
        k.meta = true;
        assert_eq!(encode(&k, 0, true), None);
        let mut k = key(K::Left);
        k.meta = true;
        assert_eq!(encode(&k, 0, true), None);
        // Plain text types.
        assert_eq!(encode(&text("é"), 0, true).unwrap(), "é");
    }

    #[test]
    fn alt_screen_chords() {
        let mut k = text("z");
        k.meta = true;
        assert_eq!(encode(&k, M_ALT_SCREEN, true).unwrap(), "\x1a");
        k.shift = true;
        assert_eq!(encode(&k, M_ALT_SCREEN, true).unwrap(), "\x1b[122;10u");
        let mut k = text(";");
        k.ctrl = true;
        assert_eq!(encode(&k, M_ALT_SCREEN, false).unwrap(), "\x1b[59;5u");
        let mut k = text("P");
        k.ctrl = true;
        k.shift = true;
        assert_eq!(encode(&k, M_ALT_SCREEN, false).unwrap(), "\x1b[112;6u");
        // Outside the alt screen ⌘Z stays the app's.
        let mut k = text("z");
        k.meta = true;
        assert_eq!(encode(&k, 0, true), None);
    }

    #[test]
    fn clip_chords() {
        let mut k = text("c");
        k.meta = true;
        assert_eq!(clip_shortcut(&k, true, false), Some(Clip::Copy));
        let mut k = text("v");
        k.meta = true;
        assert_eq!(clip_shortcut(&k, true, false), Some(Clip::Paste));
        // Ctrl+C on a Mac is ^C.
        let mut k = text("c");
        k.ctrl = true;
        assert_eq!(clip_shortcut(&k, true, true), None);
        // Elsewhere.
        assert_eq!(clip_shortcut(&k, false, false), None);
        assert_eq!(clip_shortcut(&k, false, true), Some(Clip::CopyOrInterrupt));
        k.shift = true;
        assert_eq!(clip_shortcut(&k, false, false), Some(Clip::Copy));
        let mut k = text("v");
        k.ctrl = true;
        assert_eq!(clip_shortcut(&k, false, false), Some(Clip::Paste));
        let mut k = key(K::Insert);
        k.shift = true;
        assert_eq!(clip_shortcut(&k, false, false), Some(Clip::Paste));
        k.shift = false;
        k.ctrl = true;
        assert_eq!(clip_shortcut(&k, false, false), Some(Clip::Copy));
    }
}
