//! Ledger and repo keys (I:internal/ledger/keys.go).

use gocompat::text;

const GITHUB_PREFIX: &str = "remote:github.com/";

/// Go `strings.TrimSpace`: `unicode.IsSpace` at both ends.
pub(crate) fn trim_space(s: &str) -> &str {
    s.trim_matches(text::is_space)
}

/// `"pr:<lowercased owner/name>#<n>"`.
pub fn key(repo: &str, n: i64) -> String {
    format!("pr:{}#{n}", text::to_lower_str(trim_space(repo)))
}

/// The repo.Resolver key for a GitHub `"owner/name"`:
/// `"remote:github.com/<lowercased owner/name>"`.
pub fn repo_key(repo: &str) -> String {
    format!("{GITHUB_PREFIX}{}", text::to_lower_str(trim_space(repo)))
}

/// Inverts [`repo_key`]: the `"owner/name"` part of a `remote:github.com/`
/// key, or `""` when the key is another kind or the rest is not exactly
/// owner/name.
pub fn repo_from_key(k: &str) -> &str {
    let Some(rest) = k.strip_prefix(GITHUB_PREFIX) else {
        return "";
    };
    let Some((owner, name)) = rest.split_once('/') else {
        return "";
    };
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return "";
    }
    rest
}
