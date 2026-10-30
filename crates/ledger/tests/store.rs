//! I:internal/ledger/store_test.go.

use gocompat::time::GoTime;
use ledger::{Entry, KEEP_SECS, Ledger, Role, key};

const PR: &str = "pr:acme/webapp#1437";

fn at(s: &str) -> GoTime {
    GoTime::parse_rfc3339(s).unwrap_or_else(|| panic!("bad time {s}"))
}

fn entry(k: &str, session: &str, role: Role, when: &str) -> Entry {
    Entry {
        key: k.into(),
        repo: "acme/webapp".into(),
        number: 1437,
        session: session.into(),
        role,
        source: "gh".into(),
        at: at(when),
        ..Entry::default()
    }
}

fn sessions(es: &[Entry]) -> Vec<String> {
    es.iter().map(|e| e.session.clone()).collect()
}

/// `t.Add(d)` for whole seconds and nanoseconds.
fn add(t: GoTime, secs: i64, nanos: i64) -> GoTime {
    GoTime::unix(t.secs + secs, i64::from(t.nanos) + nanos)
}

// Ported: "owners rank who made a thing over who touched or reviewed it, then earliest".
#[test]
fn test_owners_rank() {
    let l = Ledger::new();
    let now = at("2026-10-09T13:00:00Z");
    l.add(entry(
        PR,
        "dev/reviewer",
        Role::REVIEWER,
        "2026-10-09T10:00:00Z",
    ));
    l.add(entry(
        PR,
        "dev/late-toucher",
        Role::TOUCHED,
        "2026-10-09T12:00:00Z",
    ));
    l.add(entry(
        PR,
        "dev/early-toucher",
        Role::TOUCHED,
        "2026-10-09T11:00:00Z",
    ));
    assert_eq!(
        sessions(&l.owners(PR, &now)),
        ["dev/early-toucher", "dev/late-toucher", "dev/reviewer"]
    );
    l.add(entry(
        PR,
        "dev/creator",
        Role::AUTHOR,
        "2026-10-09T12:30:00Z",
    ));
    assert_eq!(l.owners(PR, &now)[0].session, "dev/creator");
}

#[test]
fn test_owners_tie_break_on_session() {
    let l = Ledger::new();
    let when = "2026-10-09T10:00:00Z";
    l.add(entry(PR, "dev/b", Role::TOUCHED, when));
    l.add(entry(PR, "dev/c", Role::TOUCHED, when));
    l.add(entry(PR, "dev/a", Role::TOUCHED, when));
    assert_eq!(
        sessions(&l.owners(PR, &at(when))),
        ["dev/a", "dev/b", "dev/c"]
    );
}

struct AddCase {
    name: &'static str,
    first: Option<Entry>,
    add: Entry,
    changed: bool,
    want_at: &'static str,
    want_url: &'static str,
    count: usize,
}

#[test]
fn test_add() {
    let base = entry(PR, "dev/s", Role::TOUCHED, "2026-10-09T12:00:00Z");
    let with = |f: &dyn Fn(&mut Entry)| {
        let mut e = base.clone();
        f(&mut e);
        e
    };
    const URL: &str = "https://github.com/acme/webapp/pull/1437";
    let c = |name, first: Option<Entry>, add, changed, want_at, want_url, count| AddCase {
        name,
        first,
        add,
        changed,
        want_at,
        want_url,
        count,
    };
    let b = || Some(base.clone());
    let tests = vec![
        c(
            "new identity",
            None,
            base.clone(),
            true,
            "2026-10-09T12:00:00Z",
            "",
            1,
        ),
        c(
            "empty key ignored",
            None,
            with(&|e| e.key = String::new()),
            false,
            "",
            "",
            0,
        ),
        c(
            "empty session ignored",
            None,
            with(&|e| e.session = String::new()),
            false,
            "",
            "",
            0,
        ),
        c(
            "unknown role ignored",
            None,
            with(&|e| e.role = Role::new("poster")),
            false,
            "",
            "",
            0,
        ),
        c(
            "empty role ignored",
            None,
            with(&|e| e.role = Role::new("")),
            false,
            "",
            "",
            0,
        ),
        c(
            "same again unchanged",
            b(),
            base.clone(),
            false,
            "2026-10-09T12:00:00Z",
            "",
            1,
        ),
        c(
            "later at keeps earliest",
            b(),
            with(&|e| e.at = at("2026-10-09T12:05:00Z")),
            false,
            "2026-10-09T12:00:00Z",
            "",
            1,
        ),
        c(
            "earlier at wins",
            b(),
            with(&|e| e.at = at("2026-10-09T11:00:00Z")),
            true,
            "2026-10-09T11:00:00Z",
            "",
            1,
        ),
        c(
            "zero at does not win",
            b(),
            with(&|e| e.at = GoTime::ZERO),
            false,
            "2026-10-09T12:00:00Z",
            "",
            1,
        ),
        c(
            "zero at ignored",
            None,
            with(&|e| e.at = GoTime::ZERO),
            false,
            "",
            "",
            0,
        ),
        c(
            "first non-empty url is taken",
            b(),
            with(&|e| e.url = URL.into()),
            true,
            "2026-10-09T12:00:00Z",
            URL,
            1,
        ),
        c(
            "a later url does not replace the first",
            Some(with(&|e| e.url = URL.into())),
            with(&|e| e.url = format!("{URL}/files")),
            false,
            "2026-10-09T12:00:00Z",
            URL,
            1,
        ),
        c(
            "same session other role is a new identity",
            b(),
            with(&|e| e.role = Role::REVIEWER),
            true,
            "2026-10-09T12:00:00Z",
            "",
            2,
        ),
        c(
            "same pr other session is a new identity",
            b(),
            with(&|e| e.session = "dev/other".into()),
            true,
            "2026-10-09T12:00:00Z",
            "",
            2,
        ),
    ];
    let now = at("2026-10-09T13:00:00Z");
    for tt in tests {
        let l = Ledger::new();
        let first_nil = tt.first.is_none();
        if let Some(f) = tt.first {
            l.add(f);
        }
        assert_eq!(l.add(tt.add), tt.changed, "{}: Add", tt.name);
        let got = l.owners(PR, &now);
        assert_eq!(got.len(), tt.count, "{}: owners {got:?}", tt.name);
        if tt.count == 0 {
            continue;
        }
        for e in &got {
            if e.session == base.session && e.role == base.role || first_nil {
                assert!(e.at.equal(&at(tt.want_at)), "{}: at = {:?}", tt.name, e.at);
                assert_eq!(e.url, tt.want_url, "{}: url", tt.name);
            }
        }
    }
}

// Ported: "recording the same thing again keeps the earliest time" (with URL
// standing in for tsukumo's label).
#[test]
fn test_add_keeps_earliest_and_first_url() {
    let l = Ledger::new();
    let now = at("2026-10-09T13:00:00Z");
    l.add(entry(PR, "dev/s", Role::AUTHOR, "2026-10-09T12:00:00Z"));
    let mut second = entry(PR, "dev/s", Role::AUTHOR, "2026-10-09T12:05:00Z");
    second.url = "https://github.com/acme/webapp/pull/1437".into();
    second.source = "gitOperation".into();
    assert!(l.add(second.clone()), "adding a url should change the view");
    let mut want = entry(PR, "dev/s", Role::AUTHOR, "2026-10-09T12:00:00Z");
    want.url = second.url;
    assert_eq!(l.owners(PR, &now), vec![want]);
}

// Ported: "entries older than 45 days are dropped".
#[test]
fn test_keep() {
    let now = at("2026-10-09T00:00:00Z");
    let tests = [
        ("old", at("2026-08-01T00:00:00Z"), false),
        ("just past keep", add(now, -KEEP_SECS, -1), false),
        ("exactly keep", add(now, -KEEP_SECS, 0), true),
        ("recent", add(now, -3600, 0), true),
        ("future", add(now, 3600, 0), true),
    ];
    for (name, when, keep) in tests {
        let l = Ledger::new();
        let mut e = entry(PR, "dev/s", Role::TOUCHED, "2026-10-09T00:00:00Z");
        e.at = when;
        l.add(e);
        assert_eq!(l.owners(PR, &now).len() == 1, keep, "{name}: owners");
        assert_eq!(l.for_repo("", &now).len() == 1, keep, "{name}: ForRepo");
        assert_eq!(
            l.for_session("dev/s", &now).len() == 1,
            keep,
            "{name}: ForSession"
        );
        // The row stays: it shows again for an earlier now.
        assert_eq!(
            l.owners(PR, &when).len(),
            1,
            "{name}: entry was dropped from the store"
        );
    }
}

fn pr_entry(repo: &str, n: i64, session: &str, role: Role, when: &str) -> Entry {
    Entry {
        key: key(repo, n),
        repo: repo.into(),
        number: n,
        session: session.into(),
        role,
        source: "pr-link".into(),
        at: at(when),
        ..Entry::default()
    }
}

fn key_role_sessions(es: &[Entry]) -> Vec<String> {
    es.iter()
        .map(|e| format!("{} {} {}", e.key, e.role, e.session))
        .collect()
}

#[test]
fn test_for_repo() {
    let l = Ledger::new();
    let now = at("2026-10-09T13:00:00Z");
    l.add(pr_entry(
        "acme/webapp",
        12,
        "dev/a",
        Role::REVIEWER,
        "2026-10-09T10:00:00Z",
    ));
    l.add(pr_entry(
        "acme/webapp",
        12,
        "dev/b",
        Role::AUTHOR,
        "2026-10-09T11:00:00Z",
    ));
    l.add(pr_entry(
        "Acme/WebApp",
        40,
        "dev/c",
        Role::TOUCHED,
        "2026-10-09T09:00:00Z",
    ));
    l.add(pr_entry(
        "acme/storefront",
        12,
        "dev/d",
        Role::AUTHOR,
        "2026-10-09T09:00:00Z",
    ));
    l.add(pr_entry(
        "acme/webapp",
        3,
        "dev/old",
        Role::AUTHOR,
        "2026-07-01T00:00:00Z",
    ));

    let webapp = vec![
        "pr:acme/webapp#40 touched dev/c",
        "pr:acme/webapp#12 author dev/b",
        "pr:acme/webapp#12 reviewer dev/a",
    ];
    let tests: Vec<(&str, Vec<&str>)> = vec![
        ("acme/webapp", webapp.clone()),
        ("ACME/WEBAPP", webapp),
        (
            "",
            vec![
                "pr:acme/webapp#40 touched dev/c",
                "pr:acme/storefront#12 author dev/d",
                "pr:acme/webapp#12 author dev/b",
                "pr:acme/webapp#12 reviewer dev/a",
            ],
        ),
        ("acme/none", vec![]),
    ];
    for (repo, want) in tests {
        assert_eq!(
            key_role_sessions(&l.for_repo(repo, &now)),
            want,
            "ForRepo({repo:?})"
        );
    }
}

#[test]
fn test_for_session() {
    let l = Ledger::new();
    let now = at("2026-10-09T13:00:00Z");
    l.add(pr_entry(
        "acme/webapp",
        12,
        "dev/a",
        Role::TOUCHED,
        "2026-10-09T10:00:00Z",
    ));
    l.add(pr_entry(
        "acme/webapp",
        13,
        "dev/a",
        Role::AUTHOR,
        "2026-10-09T12:00:00Z",
    ));
    l.add(pr_entry(
        "acme/webapp",
        12,
        "dev/a",
        Role::REVIEWER,
        "2026-10-09T10:00:00Z",
    ));
    l.add(pr_entry(
        "acme/webapp",
        11,
        "dev/a",
        Role::AUTHOR,
        "2026-10-09T10:00:00Z",
    ));
    l.add(pr_entry(
        "acme/webapp",
        14,
        "dev/b",
        Role::AUTHOR,
        "2026-10-09T12:30:00Z",
    ));
    l.add(pr_entry(
        "acme/webapp",
        2,
        "dev/a",
        Role::AUTHOR,
        "2026-06-01T00:00:00Z",
    ));
    assert_eq!(
        key_role_sessions(&l.for_session("dev/a", &now)),
        [
            "pr:acme/webapp#13 author dev/a",
            "pr:acme/webapp#11 author dev/a",
            "pr:acme/webapp#12 touched dev/a",
            "pr:acme/webapp#12 reviewer dev/a",
        ]
    );
    assert!(l.for_session("dev/none", &now).is_empty());
}

#[test]
fn test_views_are_copies() {
    let l = Ledger::new();
    let now = at("2026-10-09T13:00:00Z");
    l.add(entry(PR, "dev/s", Role::TOUCHED, "2026-10-09T12:00:00Z"));
    let mut got = l.owners(PR, &now);
    got[0].session = "mutated".into();
    got[0].url = "x".into();
    let again = l.owners(PR, &now);
    assert!(
        again[0].session == "dev/s" && again[0].url.is_empty(),
        "view mutation leaked: {:?}",
        again[0]
    );
}

#[test]
fn test_concurrent() {
    let l = Ledger::new();
    let now = at("2026-10-09T13:00:00Z");
    std::thread::scope(|s| {
        for g in 0..8 {
            let (l, now) = (&l, &now);
            s.spawn(move || {
                for i in 0..200 {
                    let session = format!("dev/s{g}");
                    l.add(pr_entry(
                        "acme/webapp",
                        i % 10 + 1,
                        &session,
                        Role::TOUCHED,
                        "2026-10-09T12:00:00Z",
                    ));
                    l.owners(&key("acme/webapp", i % 10 + 1), now);
                    l.for_repo("acme/webapp", now);
                    l.for_session(&session, now);
                }
            });
        }
    });
    assert_eq!(l.for_repo("", &now).len(), 80);
}
