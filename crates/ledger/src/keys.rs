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

/// The one spelling of a repoKey, the way a device's checkout reports it
/// (capture's `normalize_remote`): `remote:` with its host and path
/// lowercased, trailing `/` and a final `.git` dropped; `root:` with its sha
/// lowercased. Anything else comes back trimmed and unchanged, so the key
/// checks still refuse it.
///
/// Cards written before #480 kept whatever case they were given
/// ("remote:github.com/StephenSHorton/irori") while checkouts are always
/// lowercase, so they never matched one. Every place that stores or compares
/// a key runs it through here, and the folds do too, which repairs those
/// old rows as they are read back.
pub fn canonical_repo_key(k: &str) -> String {
    let k = trim_space(k);
    let Some((kind, rest)) = k.split_once(':') else {
        return k.to_string();
    };
    if kind.eq_ignore_ascii_case("remote") {
        let mut p = text::to_lower_str(rest);
        loop {
            let before = p.len();
            while p.ends_with('/') {
                p.pop();
            }
            if let Some(s) = p.strip_suffix(".git")
                && !s.is_empty()
                && !s.ends_with('/')
            {
                p.truncate(s.len());
            }
            if p.len() == before {
                break;
            }
        }
        format!("remote:{p}")
    } else if kind.eq_ignore_ascii_case("root") {
        format!("root:{}", text::to_lower_str(rest))
    } else {
        k.to_string()
    }
}

/// Whether two repoKeys name the same repo once both are canonical.
pub fn same_repo_key(a: &str, b: &str) -> bool {
    a == b || canonical_repo_key(a) == canonical_repo_key(b)
}

/// A repoKey as a person reads it: the repo's own name ("irori" for
/// `remote:github.com/a/irori`), "a local repo (1a2b3c4)" for a `root:` key,
/// and "" for "". Never the raw key.
pub fn repo_name(k: &str) -> String {
    let k = canonical_repo_key(k);
    if k.is_empty() {
        return String::new();
    }
    if let Some(sha) = k.strip_prefix("root:") {
        let short: String = sha.chars().take(7).collect();
        return format!("a local repo ({short})");
    }
    let rest = k.strip_prefix("remote:").unwrap_or(&k);
    match rest.rsplit('/').find(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => rest.to_string(),
    }
}

#[cfg(test)]
mod canonical_tests {
    use super::*;

    #[test]
    fn mixed_case_keys_become_the_checkouts_spelling() {
        let want = "remote:github.com/stephenshorton/irori";
        for k in [
            "remote:github.com/StephenSHorton/irori",
            "  remote:GitHub.com/StephenSHorton/irori  ",
            "Remote:github.com/stephenshorton/irori",
            "remote:github.com/StephenSHorton/irori/",
            "remote:github.com/StephenSHorton/irori.git",
            "remote:github.com/StephenSHorton/irori.git/",
            want,
        ] {
            assert_eq!(canonical_repo_key(k), want, "{k:?}");
            assert!(same_repo_key(k, want), "{k:?}");
        }
        assert_eq!(repo_key("StephenSHorton/irori"), want);
    }

    #[test]
    fn root_keys_lowercase_and_others_pass_through() {
        assert_eq!(canonical_repo_key("root:ABC123"), "root:abc123");
        assert_eq!(canonical_repo_key(""), "");
        assert_eq!(canonical_repo_key("  "), "");
        assert_eq!(
            canonical_repo_key("StephenSHorton/irori"),
            "StephenSHorton/irori"
        );
        assert_eq!(canonical_repo_key("other:Thing"), "other:Thing");
        // A bare ".git" name is a name, not a suffix.
        assert_eq!(
            canonical_repo_key("remote:host/a/.git"),
            "remote:host/a/.git"
        );
        assert_eq!(
            canonical_repo_key("remote:dev.azure.com/Org/Proj/_git/Repo"),
            "remote:dev.azure.com/org/proj/_git/repo"
        );
        assert!(!same_repo_key(
            "remote:github.com/a/irori",
            "remote:github.com/a/suzuri"
        ));
    }

    #[test]
    fn repo_names_are_short() {
        assert_eq!(repo_name("remote:github.com/StephenSHorton/irori"), "irori");
        assert_eq!(repo_name("remote:github.com/a/rust-suzuri/"), "rust-suzuri");
        assert_eq!(
            repo_name("root:10375ae4db720359e35e191ded43e130c0b4a0d9"),
            "a local repo (10375ae)"
        );
        assert_eq!(repo_name(""), "");
    }
}
