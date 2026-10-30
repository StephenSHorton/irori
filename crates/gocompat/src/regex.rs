//! Regexes with Go RE2's ASCII classes (store.md §7.12, capture.md §7.3).
//!
//! In Go, `\w`, `\d`, `\s` and `\b` are ASCII-only; in Rust's `regex` they
//! are Unicode unless spelled otherwise, and Rust's ASCII `\s` also matches
//! `\v`, which Go's does not. Two ways in:
//!
//! - [`compile`] refuses a pattern that uses `\w \W \d \D \s \S \b \B` at
//!   all, except the explicit `(?-u:\b)` / `(?-u:\B)`. Patterns written for
//!   Rust then have to spell the class out (`[0-9A-Za-z_]`, `[0-9]`,
//!   `[\t\n\f\r ]`).
//! - [`from_go`] takes a pattern exactly as the Go source has it and rewrites
//!   those escapes to their ASCII spellings, inside and outside brackets.
//!
//! Both build a Unicode-aware `regex::Regex` otherwise, so `(?i)` folds and
//! `.` match whole characters as in Go.

/// What [`compile`] and [`from_go`] refuse.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "regex {pattern:?} uses {escape}, which is Unicode in Rust and ASCII in Go; spell the class out"
    )]
    UnicodeClass { pattern: String, escape: String },
    #[error(transparent)]
    Regex(#[from] regex::Error),
}

const WORD: &str = "0-9A-Za-z_";
const DIGIT: &str = "0-9";
const SPACE: &str = r"\t\n\f\r ";

/// Compile a pattern that never relies on a Unicode-default class.
pub fn compile(pattern: &str) -> Result<regex::Regex, Error> {
    let b = pattern.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            let c = b[i + 1];
            if matches!(c, b'w' | b'W' | b'd' | b'D' | b's' | b'S' | b'b' | b'B') {
                let ascii_boundary = matches!(c, b'b' | b'B')
                    && pattern[..i].ends_with("(?-u:")
                    && pattern[i + 2..].starts_with(')');
                if !ascii_boundary {
                    return Err(Error::UnicodeClass {
                        pattern: pattern.to_string(),
                        escape: format!("\\{}", c as char),
                    });
                }
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    Ok(regex::Regex::new(pattern)?)
}

/// Rewrite a Go RE2 pattern's `\w \W \d \D \s \S \b \B` into ASCII
/// spellings Rust's `regex` reads the Go way.
pub fn translate_go(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut in_class = false;
    let mut chars = pattern.chars().peekable();
    // Position in the current bracket: Go treats a ']' right after '[' or
    // '[^' as a literal.
    let mut class_start = false;
    while let Some(c) = chars.next() {
        if c == '\\' {
            let Some(n) = chars.next() else {
                out.push('\\');
                break;
            };
            let rep = match (n, in_class) {
                ('w', false) => Some(format!("[{WORD}]")),
                ('W', false) => Some(format!("[^{WORD}]")),
                ('d', false) => Some(format!("[{DIGIT}]")),
                ('D', false) => Some(format!("[^{DIGIT}]")),
                ('s', false) => Some(format!("[{SPACE}]")),
                ('S', false) => Some(format!("[^{SPACE}]")),
                ('w', true) => Some(WORD.to_string()),
                ('d', true) => Some(DIGIT.to_string()),
                ('s', true) => Some(SPACE.to_string()),
                ('W', true) => Some(format!("[^{WORD}]")),
                ('D', true) => Some(format!("[^{DIGIT}]")),
                ('S', true) => Some(format!("[^{SPACE}]")),
                ('b', false) => Some(r"(?-u:\b)".to_string()),
                ('B', false) => Some(r"(?-u:\B)".to_string()),
                _ => None,
            };
            match rep {
                Some(r) => out.push_str(&r),
                None => {
                    out.push('\\');
                    out.push(n);
                }
            }
            class_start = false;
            continue;
        }
        if in_class {
            if c == ']' && !class_start {
                in_class = false;
            } else if c == '[' && chars.peek() == Some(&':') {
                // A POSIX class like [:alpha:]; copy it whole.
                out.push(c);
                for d in chars.by_ref() {
                    out.push(d);
                    if d == ']' {
                        break;
                    }
                }
                class_start = false;
                continue;
            }
            out.push(c);
            class_start = class_start && c == '^';
            continue;
        }
        if c == '[' {
            in_class = true;
            class_start = true;
            out.push(c);
            if chars.peek() == Some(&'^') {
                out.push('^');
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Compile a pattern copied from Go source, with Go's ASCII classes.
pub fn from_go(pattern: &str) -> Result<regex::Regex, Error> {
    Ok(regex::Regex::new(&translate_go(pattern))?)
}
