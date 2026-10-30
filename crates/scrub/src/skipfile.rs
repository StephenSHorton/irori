//! Go `SkipFile` (`skipfile.go`).

/// Whether a path's patch must not be stored at all: base names `.env` and
/// `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa*`, `id_ed25519*`,
/// `id_ecdsa*`, `credentials*`, `.netrc`, `.npmrc`, `.pypirc`
/// (case-insensitive, either slash). The base name is first reduced to the
/// file Windows would open: a drive prefix (`C:.env`) and an alternate data
/// stream (`.env::$DATA`) are dropped, and so are trailing dots and spaces.
pub fn skip_file(path: &str) -> bool {
    let mut base = path;
    if let Some(i) = base.rfind(['/', '\\']) {
        base = &base[i + 1..];
    } else if base.len() >= 2
        && base.as_bytes()[1] == b':'
        && base.as_bytes()[0].is_ascii_alphabetic()
    {
        base = &base[2..];
    }
    if let Some(i) = base.find(':') {
        base = &base[..i];
    }
    let trimmed = go_trim_space(base).trim_end_matches(['.', ' ']);
    let base = gocompat::text::to_lower_str(trimmed);
    if base.is_empty() {
        return false;
    }
    if matches!(base.as_str(), ".env" | ".netrc" | ".npmrc" | ".pypirc")
        || base.starts_with(".env.")
    {
        return true;
    }
    if [".pem", ".key", ".p12", ".pfx"]
        .iter()
        .any(|e| base.ends_with(e))
    {
        return true;
    }
    ["id_rsa", "id_ed25519", "id_ecdsa", "credentials"]
        .iter()
        .any(|p| base.starts_with(p))
}

/// Go `strings.TrimSpace`: trims Unicode white space (Go's table).
pub(crate) fn go_trim_space(s: &str) -> &str {
    s.trim_matches(gocompat::text::is_space)
}
