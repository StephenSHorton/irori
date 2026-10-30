//! Version comparison (Go update/service.go:365-411, macapp.go:42-55).

/// Go `isDev`: an empty or `dev` version never offers updates and never
/// touches the network.
pub fn is_dev(current: &str) -> bool {
    current.is_empty() || current == "dev"
}

/// Go `isNewer`: whether `tag` (`vX.Y.Z`, the `v` optional) is newer than
/// `current`. A dev build is never behind.
pub fn is_newer(tag: &str, current: &str) -> bool {
    if is_dev(current) {
        return false;
    }
    cmp_semver(
        tag.strip_prefix('v').unwrap_or(tag),
        current.strip_prefix('v').unwrap_or(current),
    ) > 0
}

/// Go `cmpSemver`: numeric `X.Y.Z`, then a final release above a
/// prerelease, then prereleases compared as byte strings.
pub fn cmp_semver(a: &str, b: &str) -> i32 {
    let (na, prea) = parse_ver(a);
    let (nb, preb) = parse_ver(b);
    for i in 0..3 {
        if na[i] != nb[i] {
            return if na[i] > nb[i] { 1 } else { -1 };
        }
    }
    match (prea.is_empty(), preb.is_empty()) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => -1,
        _ => match prea.cmp(preb) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        },
    }
}

/// Go `parseVer`: `[major, minor, patch]` (each `strconv.Atoi` of the
/// trimmed part, 0 when it does not parse) and the prerelease after `-`;
/// build metadata after `+` is dropped.
pub fn parse_ver(v: &str) -> ([i64; 3], &str) {
    let v = v.split_once('+').map_or(v, |(a, _)| a);
    let (num, pre) = v.split_once('-').unwrap_or((v, ""));
    let mut out = [0i64; 3];
    for (i, p) in num.splitn(3, '.').enumerate() {
        out[i] = atoi(p.trim_matches(go_space));
    }
    (out, pre)
}

/// Go `strconv.Atoi`: an optional sign and ASCII digits, nothing else.
fn atoi(s: &str) -> i64 {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    s.parse().unwrap_or(0)
}

/// `strings.TrimSpace`'s set.
fn go_space(c: char) -> bool {
    c.is_whitespace()
}

/// Go `numericVersion`: `v` without a leading `v` or a prerelease or build
/// part; the `X.Y.Z` stamped into `CFBundleShortVersionString` and the
/// installer's version resource.
pub fn numeric_version(v: &str) -> &str {
    let v = v.strip_prefix('v').unwrap_or(v);
    match v.find(['-', '+']) {
        Some(i) => &v[..i],
        None => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go TestIsNewer.
    #[test]
    fn newer() {
        let cases = [
            ("v1.2.3", "1.2.2", true),
            ("v1.2.3", "1.2.3", false),
            ("v1.2.3", "1.3.0", false),
            ("v2.0.0", "1.9.9", true),
            ("v1.0.0", "1.0.0-rc.1", true),
            ("v1.0.0-rc.1", "1.0.0", false),
            ("v1.2.3+build.7", "1.2.3", false),
            ("v1.2.3", "dev", false),
            ("v1.2.3", "", false),
            ("1.2.3", "1.2.0", true),
            // D11: the first Rust release installs over the Go line.
            ("v0.6.0", "0.5.0", true),
            ("v0.6.0", "0.5.9", true),
            ("v0.6.0", "0.6.0-rc.1", true),
        ];
        for (tag, cur, want) in cases {
            assert_eq!(is_newer(tag, cur), want, "is_newer({tag:?}, {cur:?})");
        }
    }

    // Go TestCmpSemver.
    #[test]
    fn semver() {
        assert!(cmp_semver("1.10.0", "1.9.0") > 0);
        assert_eq!(cmp_semver("1.2.3", "1.2.3"), 0);
        assert_eq!(parse_ver("1.2.3.4"), ([1, 2, 0], ""));
        assert_eq!(parse_ver(" 1 .x.3-rc+b"), ([1, 0, 3], "rc"));
        // Cut at '+' first, then '-', as Go does.
        assert_eq!(parse_ver("+1.-2.3"), ([0, 0, 0], ""));
        assert_eq!(parse_ver("1.-2.3"), ([1, 0, 0], "2.3"));
        assert_eq!(parse_ver("1. 2 .03"), ([1, 2, 3], ""));
        assert_eq!(parse_ver("99999999999999999999.1"), ([0, 1, 0], ""));
    }

    // Go TestNumericVersion.
    #[test]
    fn numeric() {
        for (i, w) in [
            ("1.2.3", "1.2.3"),
            ("v1.2.3", "1.2.3"),
            ("0.4.0-rc.1", "0.4.0"),
            ("1.0.0+b7", "1.0.0"),
        ] {
            assert_eq!(numeric_version(i), w);
        }
    }
}
