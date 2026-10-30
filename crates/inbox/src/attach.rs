//! Attachments on messages (I:internal/inbox/attachments.go) and a copy of
//! `blob.SafeName` (I:internal/blob/attachpath.go:145-213), which the fence
//! prints. Merge note: when the blob crate lands, use its safe_name here.

use gocompat::text;

use crate::state::trim;
use crate::{FileRef, Inbox, st};

/// Caps a sanitized name (runes), keeping its extension.
const MAX_NAME: usize = 120;

/// Reduces an attachment name from another device's row to a single safe
/// file name; empty falls back to `attachment-<12 hex>`.
pub fn safe_name(name: &str, sha: &str) -> String {
    let mut name = name;
    if let Some(i) = name.rfind(['/', '\\']) {
        name = &name[i + 1..];
    }
    let nb = name.as_bytes();
    if nb.len() >= 2 && nb[1] == b':' && nb[0] < 0x80 && text::is_letter(nb[0] as char) {
        name = &name[2..];
    }
    if let Some(i) = name.find(':') {
        name = &name[..i]; // an alternate data stream
    }
    let mut b = String::with_capacity(name.len());
    for r in name.chars() {
        let bad = (r as u32) < 0x20
            || r as u32 == 0x7f
            || text::is_control(r)
            || r == char::REPLACEMENT_CHARACTER
            || text::is_cf(r)
            || "<>:\"/\\|?*".contains(r);
        if bad {
            b.push('_');
        } else {
            b.push(r);
        }
    }
    let t = trim(b.trim_end_matches(['.', ' ']));
    let mut name = t.trim_start_matches(['.', ' ']).to_string();
    if name.is_empty() {
        let id = if sha.len() > 12 { &sha[..12] } else { sha };
        return format!("attachment-{id}");
    }
    if reserved_name(&name) {
        name = format!("_{name}");
    }
    let r: Vec<char> = name.chars().collect();
    if r.len() > MAX_NAME {
        let mut ext: Vec<char> = ext_of(&name).chars().collect();
        if ext.len() > 16 {
            ext.clear();
        }
        let mut out: String = r[..MAX_NAME - ext.len()].iter().collect();
        out.extend(ext);
        name = out;
    }
    name
}

/// Go `filepath.Ext` of a name with no separators.
fn ext_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) => &name[i..],
        None => "",
    }
}

/// CON, PRN, AUX, NUL, CONIN$, CONOUT$, COM0-9 and LPT0-9 (with
/// superscript digits), with or without an extension, in any case.
fn reserved_name(name: &str) -> bool {
    let mut base = text::to_upper_str(name);
    if let Some(i) = base.find('.') {
        base.truncate(i);
    }
    let base = base.trim_end_matches(' ');
    if matches!(base, "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$") {
        return true;
    }
    let r: Vec<char> = base.chars().collect();
    if r.len() == 4 {
        let head: String = r[..3].iter().collect();
        if head == "COM" || head == "LPT" {
            let d = r[3];
            return d.is_ascii_digit() || d == '¹' || d == '²' || d == '³';
        }
    }
    false
}

impl Inbox {
    /// The file sha on a message reader may read: one it sent, or one with
    /// a delivery to it that isn't held. The person reads every message.
    pub fn attachment(&self, reader: &str, sha: &str) -> Option<FileRef> {
        self.mutate(|b| {
            let person = b.is_owner(reader);
            let find = |m: &crate::Message| m.attachments.iter().find(|f| f.sha256 == sha).cloned();
            let mut ids: Vec<&String> = b.msgs.keys().collect();
            ids.sort();
            for id in ids {
                let m = &b.msgs[id];
                if (m.from.key == reader || person)
                    && let Some(f) = find(m)
                {
                    return Some(f);
                }
            }
            for id in &b.order {
                let d = &b.dels[id];
                if d.d.to != reader || d.d.state == st::HELD {
                    continue;
                }
                if let Some(m) = b.msgs.get(&d.d.message_id)
                    && let Some(f) = find(m)
                {
                    return Some(f);
                }
            }
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::safe_name;

    #[test]
    fn safe_names() {
        let sha = "0123456789abcdef0123";
        assert_eq!(safe_name("../../etc/passwd", sha), "passwd");
        assert_eq!(safe_name(r"C:\x\a.txt", sha), "a.txt");
        assert_eq!(safe_name("c:evil.txt", sha), "evil.txt");
        assert_eq!(safe_name("a.txt:stream", sha), "a.txt");
        assert_eq!(safe_name("a\u{202e}b<c>.txt", sha), "a_b_c_.txt");
        assert_eq!(safe_name(" .. ", sha), "attachment-0123456789ab");
        assert_eq!(safe_name("con.txt", sha), "_con.txt");
        assert_eq!(safe_name("COM¹", sha), "_COM¹");
        assert_eq!(safe_name("name. . ", sha), "name");
        let long = format!("{}.png", "a".repeat(200));
        let s = safe_name(&long, sha);
        assert_eq!(s.chars().count(), 120);
        assert!(s.ends_with("a.png"));
    }
}
