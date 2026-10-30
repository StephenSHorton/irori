//! I:internal/ledger/bash_test.go, plus the `\v` regression (store.md
//! §7.12: Go's `\s` does not match a vertical tab).

use ledger::{BashRef, BashResult, Ref, Role, from_bash};

const WEB: &str = "acme/webapp";
const URL: &str = "https://github.com/acme/webapp/pull/1437";

fn author(repo: &str, n: i64, u: &str, source: &str) -> BashRef {
    BashRef {
        r#ref: Ref {
            repo: repo.into(),
            number: n,
        },
        url: u.into(),
        role: Role::AUTHOR,
        source: source.into(),
    }
}

fn reviewer(repo: &str, n: i64, u: &str) -> BashRef {
    BashRef {
        r#ref: Ref {
            repo: repo.into(),
            number: n,
        },
        url: u.into(),
        role: Role::REVIEWER,
        source: "gh".into(),
    }
}

fn b(command: &str, stdout: &str, git_operation: &str, default_repo: &str) -> BashResult {
    BashResult {
        command: command.into(),
        stdout: stdout.into(),
        git_operation: git_operation.as_bytes().to_vec(),
        default_repo: default_repo.into(),
    }
}

/// A command with a default repo only.
fn cmd(command: &str, default_repo: &str) -> BashResult {
    b(command, "", "", default_repo)
}

#[test]
fn test_from_bash() {
    let created = format!(r#"{{"pr":{{"number":1437,"url":"{URL}","action":"created"}}}}"#);
    let tests: Vec<(&str, BashResult, Vec<BashRef>)> = vec![
        (
            "gitOperation created",
            b(
                r#"gh pr create --title "x" --body "y""#,
                &format!("{URL}\n"),
                &created,
                "",
            ),
            vec![author(WEB, 1437, URL, "gitOperation")],
        ),
        (
            "gitOperation created beside push, any command",
            b(
                "git push -u origin feat/x && gh pr create --fill",
                "",
                &format!(
                    r#"{{"push":{{"branch":"feat/x"}},"pr":{{"number":1437,"url":"{URL}","action":"created"}}}}"#
                ),
                "",
            ),
            vec![author(WEB, 1437, URL, "gitOperation")],
        ),
        (
            "gitOperation wins over stdout",
            b(
                "gh pr create --fill",
                "https://github.com/acme/other/pull/5\n",
                &created,
                "",
            ),
            vec![author(WEB, 1437, URL, "gitOperation")],
        ),
        (
            "gitOperation without url takes DefaultRepo",
            b(
                "gh pr create",
                "",
                r#"{"pr":{"number":9,"action":"created"}}"#,
                WEB,
            ),
            vec![author(WEB, 9, "", "gitOperation")],
        ),
        (
            "gitOperation without url or repo is skipped",
            b(
                "echo hi",
                "",
                r#"{"pr":{"number":9,"action":"created"}}"#,
                "",
            ),
            vec![],
        ),
        (
            "gitOperation without number takes it from the url",
            b(
                "echo hi",
                "",
                &format!(r#"{{"pr":{{"url":"{URL}","action":"created"}}}}"#),
                "",
            ),
            vec![author(WEB, 1437, URL, "gitOperation")],
        ),
        (
            "gitOperation edited is not authorship",
            b(
                "gh pr edit 1437 --title x",
                "",
                &format!(r#"{{"pr":{{"number":1437,"url":"{URL}","action":"edited"}}}}"#),
                "",
            ),
            vec![],
        ),
        (
            "gitOperation merged is not authorship",
            b(
                "gh pr merge 1437",
                "",
                r#"{"pr":{"number":1437,"action":"merged"}}"#,
                WEB,
            ),
            vec![],
        ),
        (
            "edited gitOperation falls back to gh pr create stdout",
            b(
                "gh pr create --fill",
                &format!("{URL}\n"),
                r#"{"pr":{"number":1,"action":"edited"}}"#,
                "",
            ),
            vec![author(WEB, 1437, URL, "gh")],
        ),
        (
            "bad gitOperation json falls back to stdout",
            b("gh pr create --fill", URL, r#"{"pr":"#, ""),
            vec![author(WEB, 1437, URL, "gh")],
        ),
        (
            "gh pr create stdout, first url only",
            b(
                "gh pr create --fill",
                &format!(
                    "Creating pull request\r\n{URL}\r\nhttps://github.com/acme/webapp/pull/1438\r\n"
                ),
                "",
                "",
            ),
            vec![author(WEB, 1437, URL, "gh")],
        ),
        (
            "gh pr create with extra spaces",
            b("cd C:/src/webapp && gh  pr   create --fill", URL, "", ""),
            vec![author(WEB, 1437, URL, "gh")],
        ),
        (
            "stdout url without gh pr create is ignored",
            b("gh pr view 1437", URL, "", ""),
            vec![],
        ),
        (
            "gh pr created is not gh pr create",
            b("gh pr created", URL, "", ""),
            vec![],
        ),
        (
            "gh pr create without a url",
            b(
                "gh pr create --fill",
                "a pull request for branch already exists",
                "",
                "",
            ),
            vec![],
        ),
        (
            "review by url",
            cmd(&format!("gh pr review {URL} --approve"), ""),
            vec![reviewer(WEB, 1437, URL)],
        ),
        (
            "review by url without scheme",
            cmd(
                "gh pr review github.com/acme/webapp/pull/12/files --comment -b ok",
                "",
            ),
            vec![reviewer(WEB, 12, "https://github.com/acme/webapp/pull/12")],
        ),
        (
            "review by number with -R",
            cmd("gh pr review 12 -R acme/storefront --approve", WEB),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review by number with --repo=",
            cmd("gh pr review 12 --repo=acme/storefront --approve", ""),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review by number with --repo before the number",
            cmd("gh pr review --repo acme/storefront 12 --approve", ""),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review by number with -R before the number",
            cmd("gh pr review -R acme/storefront 12", ""),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review by quoted number",
            cmd(r#"gh pr review "12" --approve"#, WEB),
            vec![reviewer(WEB, 12, "")],
        ),
        (
            "review by number takes DefaultRepo",
            cmd("gh pr review 12 --approve", WEB),
            vec![reviewer(WEB, 12, "")],
        ),
        (
            "review by number without a repo is skipped",
            cmd("gh pr review 12 --approve", ""),
            vec![],
        ),
        (
            "review of a branch name is skipped",
            cmd("gh pr review feat/x --approve", WEB),
            vec![],
        ),
        (
            "every review, -R stops at the command separator",
            cmd(
                "gh pr review 12 --approve && gh pr review 13 -R acme/storefront; gh pr review 14 | cat",
                WEB,
            ),
            vec![
                reviewer(WEB, 12, ""),
                reviewer("acme/storefront", 13, ""),
                reviewer(WEB, 14, ""),
            ],
        ),
        (
            "reviews across CRLF lines",
            cmd(
                "gh pr review 12 --approve\r\ngh pr review 13 -R acme/storefront\r\n",
                WEB,
            ),
            vec![reviewer(WEB, 12, ""), reviewer("acme/storefront", 13, "")],
        ),
        (
            "review -R with a github.com host",
            cmd(
                "gh pr review 12 -R github.com/acme/storefront --approve",
                WEB,
            ),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review --repo with a github.com host",
            cmd("gh pr review 12 --repo GitHub.com/acme/storefront", WEB),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review --repo= with a github.com host",
            cmd("gh pr review 12 --repo=github.com/acme/storefront", WEB),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review leading -R with a github.com host",
            cmd("gh pr review -R github.com/acme/storefront 12", ""),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review -R on another host is skipped",
            cmd("gh pr review 12 -R ghe.example.com/acme/storefront", WEB),
            vec![],
        ),
        (
            "review -R glued to its value",
            cmd("gh pr review 12 -Racme/storefront", WEB),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review -R with a quoted value",
            cmd(r#"gh pr review 12 -R "acme/storefront""#, WEB),
            vec![reviewer("acme/storefront", 12, "")],
        ),
        (
            "review -R with a bad value is skipped",
            cmd("gh pr review 12 -R acme", WEB),
            vec![],
        ),
        (
            "review separator inside a quoted body",
            cmd(
                r#"gh pr review 12 --body "ok; fine | done && more" -R acme/other"#,
                "acme/def",
            ),
            vec![reviewer("acme/other", 12, "")],
        ),
        (
            "review -R inside a quoted body is not a flag",
            cmd(r#"gh pr review 12 --body "see --repo acme/x""#, "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review -R inside a single-quoted body is not a flag",
            cmd("gh pr review 12 -b 'use -R acme/x' --approve", "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review --repo needs a word boundary",
            cmd("gh pr review 12 --body-file x--repo", "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review --approve before the number",
            cmd("gh pr review --approve 12", "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review short boolean flags before the number",
            cmd(
                "gh pr review -a 12; gh pr review -c 13; gh pr review -r 14",
                "acme/def",
            ),
            vec![
                reviewer("acme/def", 12, ""),
                reviewer("acme/def", 13, ""),
                reviewer("acme/def", 14, ""),
            ],
        ),
        (
            "review --comment and --request-changes before the number",
            cmd(
                "gh pr review --comment 12 && gh pr review --request-changes 13",
                "acme/def",
            ),
            vec![reviewer("acme/def", 12, ""), reviewer("acme/def", 13, "")],
        ),
        (
            "review body flag value before the number is skipped",
            cmd(r#"gh pr review -c -b "looks good" 12"#, "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review numeric body value is not the target",
            cmd("gh pr review --body 99 12 --comment", "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review --body-file and -F values before the number",
            cmd(
                "gh pr review --body-file C:\\tmp\\r.md 12 && gh pr review -F ./r.md -R acme/x 13",
                "acme/def",
            ),
            vec![reviewer("acme/def", 12, ""), reviewer("acme/x", 13, "")],
        ),
        (
            "review --body= before the number",
            cmd(r#"gh pr review --body="a b" 12"#, "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review after -- end of flags",
            cmd("gh pr review --approve -- 12", "acme/def"),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "review url on a look-alike host is skipped",
            cmd(
                "gh pr review https://evilgithub.com/acme/webapp/pull/3 --approve",
                "",
            ),
            vec![],
        ),
        (
            "review url with junk after the number is skipped",
            cmd(
                "gh pr review github.com/acme/webapp/pull/3abc --approve",
                "",
            ),
            vec![],
        ),
        (
            "review url is stored canonical",
            cmd(
                "gh pr review HTTP://WWW.GitHub.com/acme/webapp/pull/3?x=1 --approve",
                "",
            ),
            vec![reviewer(WEB, 3, "https://github.com/acme/webapp/pull/3")],
        ),
        (
            "review url with dot-only segments is skipped",
            cmd("gh pr review https://github.com/../../pull/3", ""),
            vec![],
        ),
        (
            "review -R with dot-only segments is skipped",
            cmd("gh pr review 3 -R ../..", WEB),
            vec![],
        ),
        (
            "gh pr create stdout on a look-alike host is ignored",
            b(
                "gh pr create --fill",
                "https://evilgithub.com/acme/webapp/pull/3\n",
                "",
                "",
            ),
            vec![],
        ),
        (
            "gh pr create stdout url is stored canonical",
            b(
                "gh pr create --fill",
                "https://github.com/acme/webapp/pull/1437/files\n",
                "",
                "",
            ),
            vec![author(WEB, 1437, URL, "gh")],
        ),
        (
            "gitOperation url on a look-alike host takes DefaultRepo and no url",
            b(
                "gh pr create",
                "",
                r#"{"pr":{"number":9,"url":"https://evilgithub.com/x/y/pull/9","action":"created"}}"#,
                WEB,
            ),
            vec![author(WEB, 9, "", "gitOperation")],
        ),
        (
            "gh pr review quoted in a body is text",
            cmd(
                r#"gh pr review 12 --body "then run gh pr review 13" --comment"#,
                "acme/def",
            ),
            vec![reviewer("acme/def", 12, "")],
        ),
        (
            "escaped quote inside a double-quoted body",
            cmd(
                r#"gh pr review 12 -b "say \"hi; -R acme/x\"" -R acme/y"#,
                "acme/def",
            ),
            vec![reviewer("acme/y", 12, "")],
        ),
        (
            "review with no target",
            cmd("gh pr review --approve", "acme/def"),
            vec![],
        ),
        (
            "the same review twice is one",
            cmd(
                "gh pr review 12 --comment -b a; gh pr review 12 --approve",
                WEB,
            ),
            vec![reviewer(WEB, 12, "")],
        ),
        (
            "create and review together",
            b(
                "gh pr create --fill && gh pr review 1437 --comment -b self",
                URL,
                "",
                WEB,
            ),
            vec![author(WEB, 1437, URL, "gh"), reviewer(WEB, 1437, "")],
        ),
        (
            "review number zero is skipped",
            cmd("gh pr review 0", WEB),
            vec![],
        ),
        ("nothing", cmd("go test ./...", ""), vec![]),
    ];
    for (name, input, want) in tests {
        assert_eq!(from_bash(&input), want, "{name}");
    }
}

// Go's `\s` is [\t\n\f\r ]; Rust's ASCII `\s` would also take \v.
#[test]
fn vertical_tab_is_not_space() {
    assert!(from_bash(&b("gh\x0bpr\x0bcreate", URL, "", "")).is_empty());
    assert!(from_bash(&cmd("gh\x0bpr\x0breview 12", WEB)).is_empty());
    assert!(from_bash(&cmd("gh pr review\x0b12", WEB)).is_empty());
    assert_eq!(
        from_bash(&b("gh\tpr\ncreate", URL, "", "")),
        [author(WEB, 1437, URL, "gh")]
    );
    assert_eq!(
        from_bash(&cmd("gh\tpr\x0c review\t12", WEB)),
        [reviewer(WEB, 12, "")]
    );
}
