//! Go `path.Match` (coord.md §7.9): `*`, `?`, `[...]` classes with `^`
//! negation and `\` escapes, and Go's rule for what is malformed. coord's
//! `NormalizeGlob` asks only "is this pattern well formed?", which is
//! `path.Match(pattern, "")` returning no error; [`valid`] is that.

/// The error `path.Match` returns for a malformed pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadPattern;

impl std::fmt::Display for BadPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("syntax error in pattern")
    }
}

impl std::error::Error for BadPattern {}

/// `path.Match(pattern, "") == (_, nil)`.
pub fn valid(pattern: &str) -> bool {
    matches(pattern, "").is_ok()
}

/// Go's `path.Match(pattern, name)`.
pub fn matches(pattern: &str, name: &str) -> Result<bool, BadPattern> {
    let mut pattern = pattern.as_bytes();
    let mut name = name.as_bytes();
    'pattern: while !pattern.is_empty() {
        let (star, chunk, rest) = scan_chunk(pattern);
        pattern = rest;
        if star && chunk.is_empty() {
            return Ok(!name.contains(&b'/'));
        }
        let (t, ok) = match_chunk(chunk, name)?;
        if ok && (t.is_empty() || !pattern.is_empty()) {
            name = t;
            continue;
        }
        if star {
            let mut i = 0;
            while i < name.len() && name[i] != b'/' {
                let (t, ok) = match_chunk(chunk, &name[i + 1..])?;
                if ok {
                    if pattern.is_empty() && !t.is_empty() {
                        i += 1;
                        continue;
                    }
                    name = t;
                    continue 'pattern;
                }
                i += 1;
            }
        }
        // Before returning false, check the rest of the pattern is well formed.
        while !pattern.is_empty() {
            let (_, chunk, rest) = scan_chunk(pattern);
            pattern = rest;
            match_chunk(chunk, b"")?;
        }
        return Ok(false);
    }
    Ok(name.is_empty())
}

fn scan_chunk(mut pattern: &[u8]) -> (bool, &[u8], &[u8]) {
    let mut star = false;
    while let [b'*', rest @ ..] = pattern {
        pattern = rest;
        star = true;
    }
    let mut inrange = false;
    let mut i = 0;
    while i < pattern.len() {
        match pattern[i] {
            b'\\' => {
                if i + 1 < pattern.len() {
                    i += 1;
                }
            }
            b'[' => inrange = true,
            b']' => inrange = false,
            b'*' if !inrange => break,
            _ => {}
        }
        i += 1;
    }
    (star, &pattern[..i], &pattern[i..])
}

fn decode(b: &[u8]) -> (u32, usize) {
    let (r, n) = crate::utf8::decode_rune(b);
    (r as u32, n)
}

fn match_chunk<'a>(mut chunk: &[u8], mut s: &'a [u8]) -> Result<(&'a [u8], bool), BadPattern> {
    let mut failed = false;
    while !chunk.is_empty() {
        if !failed && s.is_empty() {
            failed = true;
        }
        match chunk[0] {
            b'[' => {
                let mut r = 0u32;
                if !failed {
                    let (c, n) = decode(s);
                    r = c;
                    s = &s[n..];
                }
                chunk = &chunk[1..];
                let mut negated = false;
                if chunk.first() == Some(&b'^') {
                    negated = true;
                    chunk = &chunk[1..];
                }
                let mut matched = false;
                let mut nrange = 0;
                loop {
                    if chunk.first() == Some(&b']') && nrange > 0 {
                        chunk = &chunk[1..];
                        break;
                    }
                    let (lo, rest) = get_esc(chunk)?;
                    chunk = rest;
                    let mut hi = lo;
                    if chunk[0] == b'-' {
                        let (h, rest) = get_esc(&chunk[1..])?;
                        hi = h;
                        chunk = rest;
                    }
                    if lo <= r && r <= hi {
                        matched = true;
                    }
                    nrange += 1;
                }
                if matched == negated {
                    failed = true;
                }
            }
            b'?' => {
                if !failed {
                    if s[0] == b'/' {
                        failed = true;
                    }
                    let (_, n) = decode(s);
                    s = &s[n..];
                }
                chunk = &chunk[1..];
            }
            b'\\' => {
                chunk = &chunk[1..];
                if chunk.is_empty() {
                    return Err(BadPattern);
                }
                if !failed {
                    if chunk[0] != s[0] {
                        failed = true;
                    }
                    s = &s[1..];
                }
                chunk = &chunk[1..];
            }
            c => {
                if !failed {
                    if c != s[0] {
                        failed = true;
                    }
                    s = &s[1..];
                }
                chunk = &chunk[1..];
            }
        }
    }
    if failed {
        return Ok((b"", false));
    }
    Ok((s, true))
}

fn get_esc(mut chunk: &[u8]) -> Result<(u32, &[u8]), BadPattern> {
    if chunk.is_empty() || chunk[0] == b'-' || chunk[0] == b']' {
        return Err(BadPattern);
    }
    if chunk[0] == b'\\' {
        chunk = &chunk[1..];
        if chunk.is_empty() {
            return Err(BadPattern);
        }
    }
    let (r, n) = decode(chunk);
    if r == crate::utf8::RUNE_ERROR as u32 && n == 1 {
        return Err(BadPattern);
    }
    let rest = &chunk[n..];
    if rest.is_empty() {
        return Err(BadPattern);
    }
    Ok((r, rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_match_table() {
        // From Go's path/match_test.go.
        let cases: &[(&str, &str, Result<bool, BadPattern>)] = &[
            ("abc", "abc", Ok(true)),
            ("*", "abc", Ok(true)),
            ("*c", "abc", Ok(true)),
            ("a*", "a", Ok(true)),
            ("a*", "abc", Ok(true)),
            ("a*", "ab/c", Ok(false)),
            ("a*/b", "abc/b", Ok(true)),
            ("a*/b", "a/c/b", Ok(false)),
            ("a*b*c*d*e*/f", "axbxcxdxe/f", Ok(true)),
            ("a*b*c*d*e*/f", "axbxcxdxexxx/f", Ok(true)),
            ("a*b*c*d*e*/f", "axbxcxdxe/xxx/f", Ok(false)),
            ("a*b*c*d*e*/f", "axbxcxdxexxx/fff", Ok(false)),
            ("a*b?c*x", "abxbbxdbxebxczzx", Ok(true)),
            ("a*b?c*x", "abxbbxdbxebxczzy", Ok(false)),
            ("ab[c]", "abc", Ok(true)),
            ("ab[b-d]", "abc", Ok(true)),
            ("ab[e-g]", "abc", Ok(false)),
            ("ab[^c]", "abc", Ok(false)),
            ("ab[^b-d]", "abc", Ok(false)),
            ("ab[^e-g]", "abc", Ok(true)),
            ("a\\*b", "a*b", Ok(true)),
            ("a\\*b", "ab", Ok(false)),
            ("a?b", "a☺b", Ok(true)),
            ("a[^a]b", "a☺b", Ok(true)),
            ("a???b", "a☺b", Ok(false)),
            ("a[^a][^a][^a]b", "a☺b", Ok(false)),
            ("[a-ζ]*", "α", Ok(true)),
            ("*[a-ζ]", "A", Ok(false)),
            ("a?b", "a/b", Ok(false)),
            ("a*b", "a/b", Ok(false)),
            ("[\\]a]", "]", Ok(true)),
            ("[\\-]", "-", Ok(true)),
            ("[x\\-]", "x", Ok(true)),
            ("[x\\-]", "-", Ok(true)),
            ("[x\\-]", "z", Ok(false)),
            ("[\\-x]", "x", Ok(true)),
            ("[\\-x]", "-", Ok(true)),
            ("[\\-x]", "a", Ok(false)),
            ("[]a]", "]", Err(BadPattern)),
            ("[-]", "-", Err(BadPattern)),
            ("[x-]", "x", Err(BadPattern)),
            ("[x-]", "-", Err(BadPattern)),
            ("[x-]", "z", Err(BadPattern)),
            ("[-x]", "x", Err(BadPattern)),
            ("[-x]", "-", Err(BadPattern)),
            ("[-x]", "a", Err(BadPattern)),
            ("\\", "a", Err(BadPattern)),
            ("[a-b-c]", "a", Err(BadPattern)),
            ("[", "a", Err(BadPattern)),
            ("[^", "a", Err(BadPattern)),
            ("[^bc", "a", Err(BadPattern)),
            ("a[", "a", Err(BadPattern)),
            ("a[", "ab", Err(BadPattern)),
            ("a[", "x", Err(BadPattern)),
            ("a/b[", "x", Err(BadPattern)),
            ("*x", "xxx", Ok(true)),
        ];
        for (p, n, want) in cases {
            assert_eq!(matches(p, n), *want, "Match({p:?}, {n:?})");
        }
    }

    #[test]
    fn valid_rules() {
        assert!(valid("*.go"));
        assert!(valid("[b-a]"));
        assert!(!valid("["));
        assert!(!valid("[]"));
        assert!(!valid("[-a]"));
        assert!(!valid("[a-]"));
        assert!(!valid("a[b"));
    }
}
