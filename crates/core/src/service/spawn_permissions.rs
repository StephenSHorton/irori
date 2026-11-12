//! The permissions a spawned agent starts with (card #449,
//! docs/design/spawned-agents.md §7 "Permission mode", §13 decision 8 (c),
//! §14 part 5).
//!
//! Every spawn passes `--permission-mode`, since without it the agent
//! inherits the caller's (bypass on both machines). The board's choice is
//! set in the app only, never by a tool (decision 23):
//! - [`Choice::Limited`]: `acceptEdits` plus a dedicated `--settings` file
//!   with allow rules for the repo's checks, git and the irori tools, and
//!   deny rules for what must never run;
//! - [`Choice::EditsOnly`]: `acceptEdits` alone, so any shell command
//!   prompts;
//! - [`Choice::Bypass`]: `bypassPermissions`, refused on a shared board
//!   and whenever another person's text is in the brief. There it falls
//!   back to [`Choice::Limited`], deny rules included.
//!
//! Claude Code's allow rules are prefix matches, so "git, but never push
//! to main" is a deny rule, not an allow rule with an exception.

use serde_json::json;

pub const ACCEPT_EDITS: &str = "acceptEdits";
pub const BYPASS: &str = "bypassPermissions";

/// The board's permission choice (§13.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Choice {
    /// (a), the recommended default.
    #[default]
    Limited,
    /// (b).
    EditsOnly,
    /// (c), guarded.
    Bypass,
}

/// How a repo's work reaches main (§7 "Merge and push policy, per repo").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MergePolicy {
    /// The lead opens a PR (the merge desk): nobody pushes to main.
    #[default]
    PullRequest,
    /// The lead rebases and pushes to main (Irori).
    StraightToMain,
}

/// A board's permission setting: the choice and the rules the person added
/// in Settings › Agents (the repo's build and test commands, as Claude
/// Code rules such as `Bash(cargo test:*)`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoardPermissions {
    pub choice: Choice,
    pub allow: Vec<String>,
    pub deny: Vec<String>,
}

/// What one spawn is started with.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPermissions {
    /// `--permission-mode`.
    pub mode: &'static str,
    /// The `--settings` file's contents, when the choice has limits.
    pub settings: Option<serde_json::Value>,
    /// Why a bypass choice wasn't used for this spawn, for the card.
    pub fell_back: Option<&'static str>,
}

impl SpawnPermissions {
    /// Whether the receive decisions treat this agent as a bypass session
    /// (§7 "The brief is data"): bypass itself, or shell commands on its
    /// allow list, either of which lets it act on another person's row.
    pub fn counts_as_bypass(&self) -> bool {
        self.mode == BYPASS
            || self.settings.as_ref().is_some_and(|s| {
                s["permissions"]["allow"].as_array().is_some_and(|a| {
                    a.iter()
                        .any(|r| r.as_str().is_some_and(|r| r.starts_with("Bash")))
                })
            })
    }
}

/// The spawn this card is getting.
#[derive(Debug, Clone, Default)]
pub struct Spawn {
    /// The card is on a board shared with other people.
    pub shared_board: bool,
    /// The brief quotes text written by someone other than the person
    /// (on the private board, too: a pasted line from another member).
    pub others_text: bool,
    pub merge: MergePolicy,
    /// The card's lead (or its only agent), the one that merges.
    pub lead: bool,
    /// Canonical URLs (`https://github.com/o/r/pull/N`) of the PRs the card
    /// links at spawn. The PR rules below bind to exactly these.
    pub prs: Vec<String>,
    /// "Review without asking" is on at spawn (merge-desk §12.8).
    pub reviews: bool,
    /// A review card (merge-desk §12.10): never Bypass, its own short list,
    /// none of the board's build or test rules, and no merge or push.
    pub review: bool,
}

pub const SHARED_BOARD: &str = "bypass is never used in a shared space";
pub const REVIEW_CARD: &str = "a review card never uses bypass and has its own short list";
pub const OTHERS_TEXT: &str = "bypass is never used when the brief has another person's text";

/// The permissions for `spawn` under the board's `p`.
pub fn spawn_permissions(p: &BoardPermissions, spawn: Spawn) -> SpawnPermissions {
    let refused = if spawn.shared_board {
        Some(SHARED_BOARD)
    } else if spawn.others_text {
        Some(OTHERS_TEXT)
    } else {
        None
    };
    if spawn.review {
        return SpawnPermissions {
            mode: ACCEPT_EDITS,
            settings: Some(settings(&review_allow(&spawn), &review_deny(p))),
            fell_back: (p.choice == Choice::Bypass).then_some(REVIEW_CARD),
        };
    }
    match (p.choice, refused) {
        (Choice::Bypass, None) => SpawnPermissions {
            mode: BYPASS,
            settings: None,
            fell_back: None,
        },
        (Choice::EditsOnly, _) => SpawnPermissions {
            mode: ACCEPT_EDITS,
            settings: Some(settings(&[], &deny_rules(p, &spawn))),
            fell_back: None,
        },
        (Choice::Bypass, why @ Some(_)) | (Choice::Limited, why) => SpawnPermissions {
            mode: ACCEPT_EDITS,
            settings: Some(settings(&allow_rules(p, &spawn), &deny_rules(p, &spawn))),
            fell_back: if p.choice == Choice::Bypass {
                why
            } else {
                None
            },
        },
    }
}

/// The allow rule for reading exactly the brief at `path`: Claude Code's
/// absolute path form, `Read(//abs/path)`, in POSIX spelling on Windows
/// too (`C:\x\b.md` is `//c/x/b.md`). The brief is outside the agent's
/// tree, so without it the agent's first step stops on a prompt.
pub fn brief_read_rule(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy();
    let raw = raw.strip_prefix(r"\\?\").unwrap_or(&raw);
    let p = raw.replace('\\', "/");
    let b = p.as_bytes();
    let posix = if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        format!("/{}{}", (b[0] as char).to_ascii_lowercase(), &p[2..])
    } else {
        p
    };
    let rest = posix.trim_start_matches('/');
    format!("Read(//{rest})")
}

/// Adds the read rule for the brief at `path` to a spawn's settings.
pub fn allow_brief_read(settings: &mut serde_json::Value, path: &std::path::Path) {
    let rule = serde_json::Value::from(brief_read_rule(path));
    let allow = &mut settings["permissions"]["allow"];
    match allow.as_array_mut() {
        Some(a) if a.contains(&rule) => {}
        Some(a) => a.push(rule),
        None => *allow = serde_json::json!([rule]),
    }
}

fn settings(allow: &[String], deny: &[String]) -> serde_json::Value {
    json!({ "permissions": { "allow": allow, "deny": deny } })
}

/// The allow rules for a limited spawn: the irori tools, staging and
/// committing, pushing (the deny rules keep main out), a PR where the repo
/// merges through one, reading and commenting on the PRs the card links
/// (reviewing too while the setting is on), and the board's own rules.
fn allow_rules(p: &BoardPermissions, spawn: &Spawn) -> Vec<String> {
    let mut out: Vec<String> = [
        "mcp__irori",
        "Bash(git status:*)",
        "Bash(git diff:*)",
        "Bash(git log:*)",
        "Bash(git add:*)",
        "Bash(git commit:*)",
        "Bash(git fetch:*)",
        "Bash(git rebase:*)",
        "Bash(git push:*)",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    if spawn.merge == MergePolicy::PullRequest {
        out.push("Bash(gh pr create:*)".into());
        // The lead merges its PR itself once the person says Merge it on
        // its "Can I merge…?"; a prompt here would sit unseen.
        if spawn.lead {
            out.push("Bash(gh pr merge:*)".into());
        }
    }
    // Merge-desk §12.12 B1c: reading, commenting and reviewing a PR is
    // bound to the URL of a PR the card links, never a bare `gh pr review`
    // or `gh pr comment` rule. The rule is the exact URL, or the URL then
    // a space and flags, so `/pull/5` never matches `/pull/50`.
    for url in &spawn.prs {
        let mut verbs = vec!["view", "diff", "checks", "comment"];
        if spawn.reviews {
            verbs.push("review");
        }
        for verb in verbs {
            out.push(format!("Bash(gh pr {verb} {url})"));
            out.push(format!("Bash(gh pr {verb} {url} *)"));
        }
    }
    for r in &p.allow {
        let r = r.trim();
        if !r.is_empty() && !out.iter().any(|o| o == r) {
            out.push(r.to_string());
        }
    }
    out
}

/// A review card's short list (merge-desk §12.8, §12.10): the irori tools,
/// reading git, and `gh pr` reading, commenting and (while the setting is
/// on) reviewing, each bound to its PR's URL. Nothing from the board's
/// build or test rules, and no `git add`, `commit`, `rebase` or `push`.
fn review_allow(spawn: &Spawn) -> Vec<String> {
    let mut out: Vec<String> = [
        "mcp__irori",
        "Bash(git status:*)",
        "Bash(git diff:*)",
        "Bash(git log:*)",
        "Bash(git fetch:*)",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for url in &spawn.prs {
        let mut verbs = vec!["view", "diff", "checks", "comment"];
        if spawn.reviews {
            verbs.push("review");
        }
        for verb in verbs {
            out.push(format!("Bash(gh pr {verb} {url})"));
            out.push(format!("Bash(gh pr {verb} {url} *)"));
        }
    }
    out
}

/// What a review card never runs: merging, checking out and pushing, and
/// the board's own deny rules.
fn review_deny(p: &BoardPermissions) -> Vec<String> {
    let mut out: Vec<String> = [
        "Bash(gh pr merge:*)",
        "Bash(gh pr checkout:*)",
        "Bash(git push:*)",
        "Bash(git merge:*)",
        "Bash(git checkout:*)",
        "Bash(rm -rf:*)",
        "Bash(rm -fr:*)",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    for r in &p.deny {
        let r = r.trim();
        if !r.is_empty() && !out.iter().any(|o| o == r) {
            out.push(r.to_string());
        }
    }
    out
}

/// What must never run, whatever the allow list says: force pushes,
/// `rm -rf`, and pushing to main unless this is the lead on a
/// straight-to-main repo.
fn deny_rules(p: &BoardPermissions, spawn: &Spawn) -> Vec<String> {
    let mut out: Vec<String> = [
        "Bash(git push --force:*)",
        "Bash(git push -f:*)",
        "Bash(git push --force-with-lease:*)",
        "Bash(rm -rf:*)",
        "Bash(rm -fr:*)",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let main_ok = spawn.merge == MergePolicy::StraightToMain && spawn.lead;
    if !main_ok {
        for r in [
            "Bash(git push origin main:*)",
            "Bash(git push origin HEAD:main:*)",
            "Bash(git push origin master:*)",
            "Bash(git push origin HEAD:master:*)",
        ] {
            out.push(r.into());
        }
    }
    for r in &p.deny {
        let r = r.trim();
        if !r.is_empty() && !out.iter().any(|o| o == r) {
            out.push(r.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(choice: Choice) -> BoardPermissions {
        BoardPermissions {
            choice,
            allow: vec!["Bash(cargo test:*)".into(), " ".into()],
            deny: vec!["Bash(cargo publish:*)".into()],
        }
    }

    fn list(s: &SpawnPermissions, which: &str) -> Vec<String> {
        s.settings.as_ref().unwrap()["permissions"][which]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn every_spawn_names_a_mode_and_none_inherits() {
        for choice in [Choice::Limited, Choice::EditsOnly, Choice::Bypass] {
            for shared_board in [false, true] {
                for others_text in [false, true] {
                    let s = spawn_permissions(
                        &board(choice),
                        Spawn {
                            shared_board,
                            others_text,
                            ..Default::default()
                        },
                    );
                    assert!(s.mode == ACCEPT_EDITS || s.mode == BYPASS, "{s:?}");
                }
            }
        }
    }

    #[test]
    fn bypass_only_on_the_private_board_with_only_his_text() {
        let b = board(Choice::Bypass);
        let s = spawn_permissions(&b, Spawn::default());
        assert_eq!(
            (s.mode, s.settings.is_none(), s.fell_back),
            (BYPASS, true, None)
        );

        let s = spawn_permissions(
            &b,
            Spawn {
                shared_board: true,
                ..Default::default()
            },
        );
        assert_eq!(s.mode, ACCEPT_EDITS);
        assert_eq!(s.fell_back, Some(SHARED_BOARD));
        // The fallback keeps the board's rules, deny rules included.
        assert!(list(&s, "deny").contains(&"Bash(cargo publish:*)".to_string()));
        assert!(list(&s, "deny").contains(&"Bash(git push origin main:*)".to_string()));
        assert!(list(&s, "allow").contains(&"Bash(cargo test:*)".to_string()));

        let s = spawn_permissions(
            &b,
            Spawn {
                others_text: true,
                ..Default::default()
            },
        );
        assert_eq!((s.mode, s.fell_back), (ACCEPT_EDITS, Some(OTHERS_TEXT)));
        assert!(s.counts_as_bypass(), "shell rules on the allow list");
    }

    #[test]
    fn edits_only_allows_no_shell_but_still_denies() {
        let s = spawn_permissions(&board(Choice::EditsOnly), Spawn::default());
        assert_eq!(s.mode, ACCEPT_EDITS);
        assert!(list(&s, "allow").is_empty());
        assert!(list(&s, "deny").contains(&"Bash(rm -rf:*)".to_string()));
        assert!(!s.counts_as_bypass());
    }

    #[test]
    fn a_pr_repos_lead_may_merge_its_pr() {
        let b = board(Choice::Limited);
        let merges = |spawn| {
            list(&spawn_permissions(&b, spawn), "allow")
                .contains(&"Bash(gh pr merge:*)".to_string())
        };
        assert!(merges(Spawn {
            lead: true,
            ..Default::default()
        }));
        assert!(!merges(Spawn::default()), "a helper");
        assert!(
            !merges(Spawn {
                lead: true,
                merge: MergePolicy::StraightToMain,
                ..Default::default()
            }),
            "a straight-to-main repo has no PR"
        );
    }

    #[test]
    fn main_is_pushed_only_by_the_lead_of_a_straight_to_main_repo() {
        let b = board(Choice::Limited);
        let denies_main = |spawn| {
            list(&spawn_permissions(&b, spawn), "deny")
                .contains(&"Bash(git push origin HEAD:main:*)".to_string())
        };
        assert!(denies_main(Spawn::default()), "a PR repo");
        assert!(
            denies_main(Spawn {
                lead: true,
                ..Default::default()
            }),
            "a PR repo's lead"
        );
        assert!(
            denies_main(Spawn {
                merge: MergePolicy::StraightToMain,
                ..Default::default()
            }),
            "a helper"
        );
        assert!(!denies_main(Spawn {
            merge: MergePolicy::StraightToMain,
            lead: true,
            ..Default::default()
        }));
        // A PR repo's agents may open one; a straight-to-main repo's don't need to.
        let allow = |spawn| list(&spawn_permissions(&b, spawn), "allow");
        assert!(allow(Spawn::default()).contains(&"Bash(gh pr create:*)".to_string()));
        assert!(
            !allow(Spawn {
                merge: MergePolicy::StraightToMain,
                ..Default::default()
            })
            .contains(&"Bash(gh pr create:*)".to_string())
        );
        // Force pushes are denied for everyone.
        assert!(
            list(
                &spawn_permissions(
                    &b,
                    Spawn {
                        merge: MergePolicy::StraightToMain,
                        lead: true,
                        ..Default::default()
                    }
                ),
                "deny"
            )
            .contains(&"Bash(git push --force:*)".to_string())
        );
    }

    #[test]
    fn the_boards_rules_are_added_once_and_blanks_dropped() {
        let mut b = board(Choice::Limited);
        b.allow.push("Bash(git add:*)".into());
        let s = spawn_permissions(&b, Spawn::default());
        let allow = list(&s, "allow");
        assert_eq!(allow.iter().filter(|r| *r == "Bash(git add:*)").count(), 1);
        assert!(allow.iter().all(|r| !r.trim().is_empty()));
        assert!(allow.contains(&"mcp__irori".to_string()));
    }

    #[test]
    fn pr_rules_bind_to_the_linked_urls_and_review_follows_the_setting() {
        let b = BoardPermissions {
            choice: Choice::Limited,
            ..Default::default()
        };
        let u5 = "https://github.com/o/r/pull/5";
        let u9 = "https://github.com/o/r/pull/9";
        let allow = |reviews, prs: &[&str]| {
            list(
                &spawn_permissions(
                    &b,
                    Spawn {
                        reviews,
                        prs: prs.iter().map(|u| u.to_string()).collect(),
                        ..Default::default()
                    },
                ),
                "allow",
            )
        };
        let has = |a: &[String], r: String| a.contains(&r);

        // On: view, diff, checks, comment and review, each for both URLs.
        let on = allow(true, &[u5, u9]);
        for u in [u5, u9] {
            for v in ["view", "diff", "checks", "comment", "review"] {
                assert!(has(&on, format!("Bash(gh pr {v} {u})")), "{v} {u}");
                assert!(has(&on, format!("Bash(gh pr {v} {u} *)")), "{v} {u} *");
            }
        }

        // Off: everything but review.
        let off = allow(false, &[u5]);
        assert!(has(&off, format!("Bash(gh pr comment {u5})")));
        assert!(has(&off, format!("Bash(gh pr view {u5} *)")));
        assert!(off.iter().all(|r| !r.contains("gh pr review")), "{off:?}");

        // No linked PR: no view/diff/checks/comment/review rule at all.
        for reviews in [true, false] {
            let none = allow(reviews, &[]);
            for v in ["view", "diff", "checks", "comment", "review"] {
                assert!(
                    none.iter().all(|r| !r.contains(&format!("gh pr {v}"))),
                    "{none:?}"
                );
            }
        }

        // Never an unbound rule: every review and comment rule names a
        // URL, and none ends in a bare prefix wildcard.
        for r in on
            .iter()
            .filter(|r| r.contains("gh pr review") || r.contains("gh pr comment"))
        {
            assert!(r.contains("https://github.com/o/r/pull/"), "{r}");
            assert!(!r.ends_with(":*)"), "{r}");
        }
        // /pull/5 never reaches /pull/50.
        assert!(
            !on.iter()
                .any(|r| r.starts_with("Bash(gh pr comment https://github.com/o/r/pull/5:"))
        );
    }

    fn review_spawn(reviews: bool) -> Spawn {
        Spawn {
            shared_board: true,
            review: true,
            reviews,
            prs: vec!["https://github.com/o/r/pull/5".into()],
            ..Default::default()
        }
    }

    /// A review card under the Bypass choice comes out limited with its own
    /// short list: none of the board's build or test rules, merge, push and
    /// checkout denied, and review rules bound to its PR (merge-desk
    /// §12.10).
    #[test]
    fn a_review_card_is_limited_under_bypass_with_its_own_list() {
        for choice in [Choice::Bypass, Choice::Limited, Choice::EditsOnly] {
            let s = spawn_permissions(&board(choice), review_spawn(true));
            assert_eq!(s.mode, ACCEPT_EDITS, "{choice:?}");
            let allow = list(&s, "allow");
            let deny = list(&s, "deny");
            assert!(
                !allow.contains(&"Bash(cargo test:*)".to_string()),
                "{allow:?}"
            );
            for no in [
                "git add",
                "git commit",
                "git rebase",
                "git push",
                "gh pr merge",
                "gh pr create",
            ] {
                assert!(
                    !allow.iter().any(|r| r.starts_with(&format!("Bash({no}"))),
                    "{no} allowed: {allow:?}"
                );
            }
            for d in [
                "Bash(gh pr merge:*)",
                "Bash(gh pr checkout:*)",
                "Bash(git push:*)",
                "Bash(git merge:*)",
                "Bash(git checkout:*)",
                "Bash(cargo publish:*)",
            ] {
                assert!(deny.contains(&d.to_string()), "{d} not denied: {deny:?}");
            }
            // PR rules bound to the URL, with a space-bounded form.
            let u = "https://github.com/o/r/pull/5";
            for verb in ["view", "diff", "checks", "comment", "review"] {
                assert!(allow.contains(&format!("Bash(gh pr {verb} {u})")), "{verb}");
                assert!(
                    allow.contains(&format!("Bash(gh pr {verb} {u} *)")),
                    "{verb}"
                );
            }
            assert!(
                !allow
                    .iter()
                    .any(|r| r.contains("gh pr review:*") || r.contains("gh pr comment:*")),
                "unbound rule: {allow:?}"
            );
        }
        let s = spawn_permissions(&board(Choice::Bypass), review_spawn(true));
        assert_eq!(s.fell_back, Some(REVIEW_CARD));
    }

    #[test]
    fn the_brief_rule_is_that_one_file_in_posix_form() {
        use std::path::Path;
        assert_eq!(
            brief_read_rule(Path::new("/Users/me/.irori/briefs/01ABC-1.md")),
            "Read(//Users/me/.irori/briefs/01ABC-1.md)"
        );
        assert_eq!(
            brief_read_rule(Path::new(
                r"C:\Users\me\AppData\Local\irori\briefs\01ABC-2.md"
            )),
            "Read(//c/Users/me/AppData/Local/irori/briefs/01ABC-2.md)"
        );
        assert_eq!(
            brief_read_rule(Path::new(r"\\?\D:\irori\briefs\x-1.md")),
            "Read(//d/irori/briefs/x-1.md)"
        );
    }

    /// Limited, edits-only and review spawns each get the brief's rule,
    /// once, and nothing wider (no glob, no Edit).
    #[test]
    fn every_limited_spawn_may_read_its_brief_and_nothing_more() {
        let path = std::path::Path::new("/h/.irori/briefs/01ABC-3.md");
        let want = "Read(//h/.irori/briefs/01ABC-3.md)".to_string();
        for (choice, spawn) in [
            (Choice::Limited, Spawn::default()),
            (Choice::EditsOnly, Spawn::default()),
            (Choice::Bypass, review_spawn(true)),
            (
                Choice::Bypass,
                Spawn {
                    shared_board: true,
                    ..Default::default()
                },
            ),
        ] {
            let mut s = spawn_permissions(&board(choice), spawn).settings.unwrap();
            allow_brief_read(&mut s, path);
            allow_brief_read(&mut s, path);
            let got = SpawnPermissions {
                mode: ACCEPT_EDITS,
                settings: Some(s),
                fell_back: None,
            };
            let allow = list(&got, "allow");
            assert_eq!(allow.iter().filter(|r| **r == want).count(), 1, "{allow:?}");
            assert!(
                allow.iter().all(
                    |r| !r.starts_with("Edit(") && !(r.starts_with("Read(") && r.contains('*'))
                ),
                "{allow:?}"
            );
            assert!(!list(&got, "deny").iter().any(|r| r.starts_with("Read(")));
        }
        // A settings value with no permissions yet gets the list.
        let mut v = serde_json::json!({});
        allow_brief_read(&mut v, path);
        assert_eq!(v["permissions"]["allow"], serde_json::json!([want]));
    }

    #[test]
    fn a_review_card_reviews_only_while_the_setting_is_on() {
        let s = spawn_permissions(&board(Choice::Limited), review_spawn(false));
        let allow = list(&s, "allow");
        assert!(
            allow.iter().all(|r| !r.contains("gh pr review")),
            "{allow:?}"
        );
        assert!(allow.iter().any(|r| r.contains("gh pr comment")));
    }
}
