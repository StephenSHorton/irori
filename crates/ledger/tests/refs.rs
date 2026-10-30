//! I:internal/ledger/refs_test.go.

use std::collections::HashMap;

use ledger::{Ref, RefOptions, extract_refs, key, parse_target};

const WEB: &str = "acme/webapp";

fn ref_keys(refs: &[Ref]) -> Vec<String> {
    refs.iter().map(|r| key(&r.repo, r.number)).collect()
}

fn aliases(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn dr(repo: &str) -> RefOptions {
    RefOptions {
        default_repo: repo.into(),
        ..RefOptions::default()
    }
}

fn owner(o: &str) -> RefOptions {
    RefOptions {
        owner: o.into(),
        ..RefOptions::default()
    }
}

fn none() -> RefOptions {
    RefOptions::default()
}

fn r(repo: &str, number: i64) -> Ref {
    Ref {
        repo: repo.into(),
        number,
    }
}

#[test]
fn test_extract_refs() {
    let al = aliases(&[("web", "webapp"), ("shop", "acme/storefront")]);
    let tests: Vec<(&str, &str, RefOptions, Vec<&str>)> = vec![
        // Ported from tsukumo refs.test.ts.
        (
            "URLs, repo#N, aliases and bare #N in order, without repeats",
            "APPROVED webapp#1407 (3ff5049c7). See https://github.com/acme/webapp/pull/1407 and WEB#1408, SHOP#498, api-server#143, then #1414.",
            RefOptions {
                default_repo: WEB.into(),
                aliases: al.clone(),
                ..none()
            },
            vec![
                "pr:acme/webapp#1407",
                "pr:acme/webapp#1408",
                "pr:acme/storefront#498",
                "pr:acme/api-server#143",
                "pr:acme/webapp#1414",
            ],
        ),
        (
            "bare #N needs a default repo",
            "REVIEW-PLEASE #492",
            none(),
            vec![],
        ),
        (
            "channel and role mentions are not PRs",
            "<#100000000000000001> <@&100000000000000014> <@100000000000000015> hi",
            dr(WEB),
            vec![],
        ),
        (
            "bare #N with a default repo",
            "REVIEW-PLEASE #492",
            dr(WEB),
            vec!["pr:acme/webapp#492"],
        ),
        (
            "owner/repo#N",
            "APPROVED acme/webapp#1407",
            none(),
            vec!["pr:acme/webapp#1407"],
        ),
        (
            "dot-only repo segments are not PRs",
            "acme/..#12 and github.com/../x/pull/3 then acme/webapp#4",
            none(),
            vec!["pr:acme/webapp#4"],
        ),
        (
            "PR#N and pull#N",
            "PR#1407 and pull#1408",
            dr(WEB),
            vec!["pr:acme/webapp#1407", "pr:acme/webapp#1408"],
        ),
        (
            "code spans and long numbers",
            "see `#12` and ```\n#13\n``` and order #333333",
            dr(WEB),
            vec![],
        ),
        (
            "a repo without an owner needs one",
            "webapp#12",
            RefOptions {
                aliases: HashMap::new(),
                ..none()
            },
            vec![],
        ),
        (
            "explicit owner",
            "webapp#12",
            owner("acme"),
            vec!["pr:acme/webapp#12"],
        ),
        // Irori edge cases.
        (
            "PR#N without default repo is skipped",
            "PR#12",
            none(),
            vec![],
        ),
        (
            "owner beats default repo owner",
            "api#7",
            RefOptions {
                default_repo: WEB.into(),
                owner: "other".into(),
                ..none()
            },
            vec!["pr:other/api#7"],
        ),
        (
            "default repo owner for name#N",
            "api#7",
            dr(WEB),
            vec!["pr:acme/api#7"],
        ),
        (
            "alias with owner needs no owner",
            "shop#5",
            RefOptions {
                aliases: al.clone(),
                ..none()
            },
            vec!["pr:acme/storefront#5"],
        ),
        (
            "alias without owner needs one",
            "web#5",
            RefOptions {
                aliases: al.clone(),
                ..none()
            },
            vec![],
        ),
        (
            "alias lookup is case-insensitive",
            "Web#5",
            RefOptions {
                owner: "acme".into(),
                aliases: al.clone(),
                ..none()
            },
            vec!["pr:acme/webapp#5"],
        ),
        (
            "repo case is kept, key dedupes",
            "Acme/WebApp#5 acme/webapp#5",
            none(),
            vec!["pr:acme/webapp#5"],
        ),
        (
            "name#N 1 digit",
            "acme/webapp#1",
            none(),
            vec!["pr:acme/webapp#1"],
        ),
        (
            "name#N 7 digits is not a ref",
            "acme/webapp#1234567",
            none(),
            vec![],
        ),
        ("bare #N 1 digit is not a ref", "#7", dr(WEB), vec![]),
        (
            "bare #N 5 digits",
            "#12345",
            dr(WEB),
            vec!["pr:acme/webapp#12345"],
        ),
        ("bare #N 6 digits is not a ref", "#123456", dr(WEB), vec![]),
        (
            "digits followed by a word char",
            "#12a acme/webapp#3x",
            dr(WEB),
            vec![],
        ),
        (
            "digits followed by punctuation",
            "(#12), acme/webapp#3.",
            dr(WEB),
            vec!["pr:acme/webapp#12", "pr:acme/webapp#3"],
        ),
        (
            "a word before # makes name#N",
            "abc#12",
            dr(WEB),
            vec!["pr:acme/abc#12"],
        ),
        (
            "bare #N after a word char only",
            "1#12 _#13",
            dr(WEB),
            vec![],
        ),
        (
            "bare #N after slash, <, #, &, @",
            "/#12 <#13 ##14 &#15 @#16",
            dr(WEB),
            vec![],
        ),
        (
            "bare #N after dash is allowed",
            "-#12",
            dr(WEB),
            vec!["pr:acme/webapp#12"],
        ),
        (
            "name#N after dash is not a ref",
            "x -webapp#12",
            owner("acme"),
            vec![],
        ),
        (
            "name#N after slash, <, # is not a ref",
            "a/b/c#12 <web#13 #web#14",
            owner("acme"),
            vec![],
        ),
        (
            "name#N after non-ASCII",
            "éwebapp#12",
            owner("acme"),
            vec!["pr:acme/webapp#12"],
        ),
        (
            "name#N after & or @ is allowed",
            "&webapp#12 @api#13",
            owner("acme"),
            vec!["pr:acme/webapp#12", "pr:acme/api#13"],
        ),
        (
            "name must start with a letter",
            "1webapp#12 _x#13",
            owner("acme"),
            vec![],
        ),
        (
            "a dash inside a name is part of it",
            "x-webapp#12 then #14",
            dr(WEB),
            vec!["pr:acme/x-webapp#12", "pr:acme/webapp#14"],
        ),
        // The lookbehind rejects "/a#12"; the scan goes on and finds b#13.
        (
            "rejected match retries at the next position",
            "/a#12 b#13",
            owner("acme"),
            vec!["pr:acme/b#13"],
        ),
        (
            "rejected name start does not swallow a URL",
            "-github.com/acme/webapp/pull/9",
            none(),
            vec!["pr:acme/webapp#9"],
        ),
        (
            "URL without scheme",
            "github.com/acme/webapp/pull/77/files",
            none(),
            vec!["pr:acme/webapp#77"],
        ),
        (
            "issue URL is not a PR",
            "https://github.com/acme/webapp/issues/77",
            none(),
            vec![],
        ),
        (
            "URL number too big",
            "github.com/acme/webapp/pull/99999999999999999999999",
            none(),
            vec![],
        ),
        ("zero is not a PR", "acme/webapp#0 #00", dr(WEB), vec![]),
        (
            "inline code across a newline is not code",
            "`a\n#12 b`",
            dr(WEB),
            vec!["pr:acme/webapp#12"],
        ),
        (
            "fenced code with CRLF",
            "```go\r\n#12\r\n```\r\n#13\r\n",
            dr(WEB),
            vec!["pr:acme/webapp#13"],
        ),
        (
            "unclosed fence is prose",
            "```\n#12",
            dr(WEB),
            vec!["pr:acme/webapp#12"],
        ),
        ("empty", "", dr(WEB), vec![]),
    ];
    for (name, text, o, want) in tests {
        assert_eq!(
            ref_keys(&extract_refs(text, &o)),
            want,
            "{name}: ExtractRefs({text:?})"
        );
    }
}

#[test]
fn test_extract_refs_keeps_repo_case() {
    let got = extract_refs("Acme/WebApp#5 and #60", &dr("Acme/WebApp"));
    assert_eq!(got, [r("Acme/WebApp", 5), r("Acme/WebApp", 60)]);
}

#[test]
fn test_parse_target() {
    let ok = |repo: &str, n| Some(r(repo, n));
    let tests: Vec<(&str, Option<Ref>)> = vec![
        ("pr:acme/webapp#1437", ok(WEB, 1437)),
        ("PR:acme/webapp#1437", ok(WEB, 1437)),
        ("acme/webapp#1437", ok(WEB, 1437)),
        ("Acme/WebApp#3", ok("Acme/WebApp", 3)),
        ("  acme/webapp#3\r\n", ok(WEB, 3)),
        ("https://github.com/acme/webapp/pull/1437", ok(WEB, 1437)),
        ("http://github.com/acme/webapp/pull/1437", ok(WEB, 1437)),
        ("github.com/acme/webapp/pull/1437", ok(WEB, 1437)),
        (
            "https://www.github.com/acme/webapp/pull/1437",
            ok(WEB, 1437),
        ),
        ("https://GitHub.com/acme/webapp/pull/1437", ok(WEB, 1437)),
        (
            "https://github.com/acme/webapp/pull/1437/files",
            ok(WEB, 1437),
        ),
        ("https://github.com/acme/webapp/pull/1437/", ok(WEB, 1437)),
        (
            "https://github.com/acme/webapp/pull/1437?diff=split",
            ok(WEB, 1437),
        ),
        (
            "https://github.com/acme/webapp/pull/1437#issuecomment-1",
            ok(WEB, 1437),
        ),
        (
            "https://github.com/acme/my.repo-x/pull/9",
            ok("acme/my.repo-x", 9),
        ),
        ("pr:acme#12", None),
        ("webapp#12", None),
        ("#12", None),
        ("acme/webapp#12x", None),
        ("acme/webapp#12 extra", None),
        ("acme/webapp#0", None),
        ("https://github.com/acme/webapp/pull/", None),
        ("https://github.com/acme/webapp/pull/12x", None),
        ("https://github.com/acme/webapp/issues/12", None),
        ("https://gitlab.com/acme/webapp/pull/12", None),
        ("https://evilgithub.com/acme/webapp/pull/12", None),
        ("see https://github.com/acme/webapp/pull/12", None),
        ("pr:https://github.com/acme/webapp/pull/12", None),
        ("", None),
        ("../..#1", None),
        ("pr:acme/..#1", None),
        ("./webapp#1", None),
        ("https://github.com/../../pull/1", None),
        ("https://github.com/acme/.../pull/1", None),
        ("acme/.github#4", ok("acme/.github", 4)),
        ("acme/web..app#4", ok("acme/web..app", 4)),
    ];
    for (input, want) in tests {
        assert_eq!(parse_target(input), want, "ParseTarget({input:?})");
    }
}
