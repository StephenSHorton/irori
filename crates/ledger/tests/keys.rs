//! I:internal/ledger/keys_test.go.

use ledger::{key, repo_from_key, repo_key};

#[test]
fn test_key() {
    let tests = [
        ("acme/webapp", 1437, "pr:acme/webapp#1437"),
        ("Acme/WebApp", 12, "pr:acme/webapp#12"),
        (" acme/webapp ", 1, "pr:acme/webapp#1"),
    ];
    for (repo, n, want) in tests {
        assert_eq!(key(repo, n), want, "Key({repo:?}, {n})");
    }
}

#[test]
fn test_repo_key() {
    let tests = [
        ("acme/webapp", "remote:github.com/acme/webapp"),
        ("Acme/WebApp", "remote:github.com/acme/webapp"),
    ];
    for (repo, want) in tests {
        assert_eq!(repo_key(repo), want, "RepoKey({repo:?})");
    }
}

#[test]
fn test_repo_from_key() {
    let rk = repo_key("Acme/WebApp");
    let tests = [
        ("remote:github.com/acme/webapp", "acme/webapp"),
        (rk.as_str(), "acme/webapp"),
        ("remote:gitlab.com/acme/webapp", ""),
        ("local:C:/Users/sam/projects/webapp", ""),
        ("remote:github.com/acme", ""),
        ("remote:github.com/acme/", ""),
        ("remote:github.com//webapp", ""),
        ("remote:github.com/acme/webapp/extra", ""),
        ("Remote:github.com/acme/webapp", ""),
        ("", ""),
    ];
    for (k, want) in tests {
        assert_eq!(repo_from_key(k), want, "RepoFromKey({k:?})");
    }
}
