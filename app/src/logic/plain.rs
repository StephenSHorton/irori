//! Untrusted text made safe to show, and the small time and path words
//! rows use (the React app's lib/irori.ts: plainLine, plainBlock,
//! relativeTime, shortRepo, shortPath).

/// C0/C1 controls, zero-width marks, bidi embeddings, overrides and
/// isolates, and the BOM.
fn hidden(c: char) -> bool {
    let c = c as u32;
    c < 0x20
        || (0x7f..=0x9f).contains(&c)
        || (0x200b..=0x200f).contains(&c)
        || (0x202a..=0x202e).contains(&c)
        || (0x2066..=0x2069).contains(&c)
        || c == 0xfeff
}

/// Agent text on one line: hidden characters become spaces, whitespace
/// runs collapse, the ends are trimmed.
pub fn plain_line(s: &str) -> String {
    let out: String = s.chars().map(|c| if hidden(c) { ' ' } else { c }).collect();
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// [`plain_line`] for multi-line text: the same characters are
/// neutralised, but newlines and tabs stay; more than three blank lines in
/// a row become three.
pub fn plain_block(s: &str) -> String {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let out: String = s
        .chars()
        .map(|c| {
            if c == '\n' || c == '\t' || !hidden(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut squeezed = String::with_capacity(out.len());
    let mut run = 0;
    for c in out.chars() {
        if c == '\n' {
            run += 1;
            if run > 3 {
                continue;
            }
        } else {
            run = 0;
        }
        squeezed.push(c);
    }
    squeezed.trim().to_string()
}

/// "just now", "42s ago", "5m ago", "3h ago", "2d ago"; "never" for no
/// time (ms since the epoch, 0 for none).
pub fn relative_time(then_ms: i64, now_ms: i64) -> String {
    if then_ms <= 0 {
        return "never".into();
    }
    let s = ((now_ms - then_ms) as f64 / 1000.0).round() as i64;
    if s < 5 {
        return "just now".into();
    }
    if s < 60 {
        return format!("{s}s ago");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m ago");
    }
    let h = m / 60;
    if h < 24 {
        return format!("{h}h ago");
    }
    format!("{}d ago", h / 24)
}

/// A repo key in a row: "owner/name" for a hosted remote, "local repo
/// 1a2b3c4" for a root-commit key.
pub fn short_repo(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    if let Some(rest) = key.strip_prefix("root:") {
        return format!("local repo {}", rest.chars().take(7).collect::<String>());
    }
    let parts: Vec<&str> = key.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() > 2 {
        parts[parts.len() - 2..].join("/")
    } else {
        key.to_string()
    }
}

/// A cwd that fits a row: the home directory becomes "~" and only the last
/// two parts stay (`~/…/a/b`).
pub fn short_path(p: &str) -> String {
    if p.is_empty() {
        return "(no directory)".into();
    }
    let sep = if p.contains('\\') { "\\" } else { "/" };
    let home_len = home_prefix(p);
    let (head, rest) = match home_len {
        Some(n) => ("~".to_string(), &p[n..]),
        None => {
            let b = p.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                (p[..2].to_string(), &p[2..])
            } else {
                (String::new(), p)
            }
        }
    };
    let parts: Vec<&str> = rest.split(['/', '\\']).filter(|x| !x.is_empty()).collect();
    if parts.len() > 2 {
        let mut v = vec![head.as_str(), "…"];
        v.extend_from_slice(&parts[parts.len() - 2..]);
        return v.join(sep);
    }
    if home_len.is_some() {
        let mut v = vec![head.as_str()];
        v.extend_from_slice(&parts);
        return v.join(sep);
    }
    p.to_string()
}

/// The length of a home directory at the start of `p`
/// (`/Users/x`, `/home/x`, `C:\Users\x`), when it ends there or at a
/// separator.
fn home_prefix(p: &str) -> Option<usize> {
    let lower_ok = |rest: &str, base: usize| -> Option<usize> {
        let name_len = rest.find(['/', '\\']).unwrap_or(rest.len());
        (name_len > 0).then_some(base + name_len)
    };
    if let Some(r) = p.strip_prefix("/Users/") {
        let n = r.find('/').unwrap_or(r.len());
        return (n > 0).then_some(7 + n);
    }
    if let Some(r) = p.strip_prefix("/home/") {
        let n = r.find('/').unwrap_or(r.len());
        return (n > 0).then_some(6 + n);
    }
    let b = p.as_bytes();
    if b.len() > 9
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/')
        && &p[3..8] == "Users"
        && (b[8] == b'\\' || b[8] == b'/')
    {
        return lower_ok(&p[9..], 9);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_line_neutralises_and_collapses() {
        assert_eq!(plain_line("  a\u{202e}b\n\tc  "), "a b c");
        assert_eq!(plain_line("x\u{200b}y"), "x y");
        assert_eq!(plain_line(""), "");
    }

    #[test]
    fn plain_block_keeps_lines() {
        assert_eq!(plain_block("a\r\nb\u{202e}c"), "a\nb c");
        assert_eq!(plain_block("a\n\n\n\n\n\nb"), "a\n\n\nb");
        assert_eq!(plain_block("  a\tb \n"), "a\tb");
    }

    #[test]
    fn times() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(0, now), "never");
        assert_eq!(relative_time(now - 2_000, now), "just now");
        assert_eq!(relative_time(now - 42_000, now), "42s ago");
        assert_eq!(relative_time(now - 5 * 60_000, now), "5m ago");
        assert_eq!(relative_time(now - 3 * 3_600_000, now), "3h ago");
        assert_eq!(relative_time(now - 49 * 3_600_000, now), "2d ago");
    }

    #[test]
    fn repos_and_paths() {
        assert_eq!(short_repo("remote:github.com/sh/irori"), "sh/irori");
        assert_eq!(short_repo("root:1a2b3c4d5e"), "local repo 1a2b3c4");
        assert_eq!(short_repo(""), "");
        assert_eq!(short_path("/Users/me/projects/a/b"), "~/…/a/b");
        assert_eq!(short_path("/Users/me/a"), "~/a");
        assert_eq!(short_path("C:\\Users\\me\\projects\\a\\b"), "~\\…\\a\\b");
        assert_eq!(short_path("/srv/x"), "/srv/x");
        assert_eq!(short_path(""), "(no directory)");
    }
}
