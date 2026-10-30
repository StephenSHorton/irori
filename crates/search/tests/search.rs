//! Ports of I:internal/search/search_test.go, plus the JSON shapes.

use gocompat::time::GoTime;
use search::{Error, Hit, Kind, ShowOptions, Shown, can_read, make_ref, page, parse_ref};

#[test]
fn parse_ref_table() {
    let cases: &[(&str, Option<(Kind, &str)>)] = &[
        ("turn:t-0123abcd", Some((Kind::Turn, "t-0123abcd"))),
        ("edit:e-99", Some((Kind::Edit, "e-99"))),
        ("commit:ABCDEF1", Some((Kind::Commit, "abcdef1"))),
        ("commit:abc", None),
        ("commit:zzzzzzz", None),
        ("msg:01JX", Some((Kind::Msg, "01JX"))),
        ("task:01JY", Some((Kind::Task, "01JY"))),
        (" Task:01JY ", Some((Kind::Task, "01JY"))),
        ("comment:c1", Some((Kind::Comment, "c1"))),
        (
            "pr:stephenshorton/irori#12",
            Some((Kind::Pr, "stephenshorton/irori#12")),
        ),
        ("pr:irori#12", None),
        ("pr:a/b#x", None),
        ("decision:d1", Some((Kind::Decision, "d1"))),
        ("agent:dev1/abc-123", Some((Kind::Agent, "dev1/abc-123"))),
        ("lease:gpu", None),
        ("task:", None),
        ("nocolon", None),
        ("task:a b", None),
    ];
    for (r, want) in cases {
        match (parse_ref(r), want) {
            (Ok((k, id)), Some((wk, wid))) => {
                assert_eq!((k, id.as_str()), (*wk, *wid), "ParseRef({r:?})")
            }
            (Err(e), None) => assert_eq!(e, Error::BadRef),
            (got, want) => panic!("ParseRef({r:?}) = {got:?}, want {want:?}"),
        }
    }
}

#[test]
fn page_table() {
    let full = "é".repeat(10); // 20 bytes
    let cases: &[(&str, ShowOptions, &str, bool, i64)] = &[
        ("all", ShowOptions::default(), &full, false, 0),
        (
            "cut on rune",
            ShowOptions {
                limit: 5,
                ..Default::default()
            },
            "éé",
            true,
            4,
        ),
        (
            "offset mid rune snaps back",
            ShowOptions {
                offset: 5,
                limit: 4,
                ..Default::default()
            },
            "éé",
            true,
            8,
        ),
        (
            "past end",
            ShowOptions {
                offset: 99,
                ..Default::default()
            },
            "",
            false,
            0,
        ),
    ];
    for (name, o, text, more, next) in cases {
        let got = page(&full, o);
        assert_eq!(got, (*text, *more, *next), "{name}");
    }
}

#[test]
fn errors_and_helpers() {
    assert_eq!(
        Error::BadRef.to_string(),
        "search: not a ref (kind:id, e.g. task:01J... or pr:owner/repo#12)"
    );
    assert_eq!(Error::NotFound.to_string(), "search: nothing has that ref");
    assert_eq!(make_ref(Kind::Pr, "o/n#1"), "pr:o/n#1");
    assert!(can_read::<&str>("", &["a"]));
    assert!(can_read::<&str>("b", &[]));
    assert!(can_read("a", &["a"]));
    assert!(!can_read("b", &["a"]));
}

#[test]
fn hit_and_shown_json() {
    let h = Hit {
        r#ref: "task:1".into(),
        kind: Kind::Task,
        at: GoTime::ZERO,
        title: "t".into(),
        snippet: "s".into(),
        agent: String::new(),
        repo_key: String::new(),
    };
    assert_eq!(
        String::from_utf8(gocompat::json::marshal(&h).unwrap()).unwrap(),
        r#"{"ref":"task:1","kind":"task","at":"0001-01-01T00:00:00Z","title":"t","snippet":"s"}"#
    );
    let s = Shown {
        r#ref: "pr:o/n#1".into(),
        kind: Kind::Pr,
        title: "x".into(),
        text: "y".into(),
        more: false,
        next: 0,
        related: vec![],
    };
    assert_eq!(
        String::from_utf8(gocompat::json::marshal(&s).unwrap()).unwrap(),
        r#"{"ref":"pr:o/n#1","kind":"pr","title":"x","text":"y","more":false}"#
    );
}
