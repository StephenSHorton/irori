//! Card #137: a card says where its work is (the holder's worktree,
//! branch, last commit, pushed or not) and where the holder left off, so
//! whoever takes it over can continue.

use crate::common;

use common::*;
use coord::*;
use store::kinds;

fn held(e: &Env) -> Task {
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Port the parser".into(),
                objective: "why it matters".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.claim_task(BOB, &t.id).unwrap();
    e.c.set_steps(
        BOB,
        &t.id,
        vec![
            Step {
                text: "lexer".into(),
                ..Step::default()
            },
            Step {
                text: "parser".into(),
                ..Step::default()
            },
        ],
    )
    .unwrap()
}

fn place(head: &str, pushed: bool) -> Where {
    Where {
        who: BOB.into(),
        cwd: "/w/parser/crates".into(),
        worktree: "/w/parser".into(),
        branch: "feat/parser".into(),
        head: head.into(),
        pushed,
        ..Where::default()
    }
}

#[test]
fn where_is_written_only_when_the_place_changes() {
    let e = Env::new();
    let t = held(&e);
    let before = e.c.get_task(&t.id).unwrap();
    assert!(e.c.set_where(&t.id, place("1a2b3c4d5e", false)).unwrap());
    // The same place again writes nothing, whatever the time.
    e.clk.add(mins(1));
    assert!(!e.c.set_where(&t.id, place("1a2b3c4d5e", false)).unwrap());
    assert_eq!(e.rows(kinds::TASK_WHERE).len(), 1);
    // Pushing it is a change.
    assert!(e.c.set_where(&t.id, place("1a2b3c4d5e", true)).unwrap());
    assert_eq!(e.rows(kinds::TASK_WHERE).len(), 2);
    let got = e.c.get_task(&t.id).unwrap();
    let w = got.where_.clone().unwrap();
    assert_eq!((w.branch.as_str(), w.pushed), ("feat/parser", true));
    assert_eq!(
        w.line(),
        "branch feat/parser @ 1a2b3c4 (pushed) in /w/parser"
    );
    // No thread line, and the card doesn't move up the grid.
    assert_eq!(got.updated_at, before.updated_at);
    assert_eq!(
        e.c.activity(&t.id).unwrap().len(),
        e.c.activity(&before.id).unwrap().len()
    );
    // A replay folds the latest.
    assert_eq!(e.reopen().get_task(&t.id).unwrap().where_, got.where_);
}

#[test]
fn outside_a_repo_it_says_so() {
    let w = Where {
        who: BOB.into(),
        cwd: "/Users/me".into(),
        ..Where::default()
    };
    assert_eq!(w.line(), "/Users/me (not a git repo)");
}

#[test]
fn a_step_tick_carries_left_off() {
    let e = Env::new();
    let t = held(&e);
    let s1 = t.steps[0].id.clone();
    let got =
        e.c.set_step_left_off(
            BOB,
            &t.id,
            &s1,
            StepUpdate {
                done: Some(true),
                ..StepUpdate::default()
            },
            "lexer done and pushed; next: the parser's error recovery",
        )
        .unwrap();
    let l = got.left_off.clone().unwrap();
    assert_eq!(
        (l.text.as_str(), l.by.as_str()),
        (
            "lexer done and pushed; next: the parser's error recovery",
            BOB
        )
    );
    // A new line on its own (the step already ticked) still writes.
    let again =
        e.c.set_step_left_off(
            BOB,
            &t.id,
            &s1,
            StepUpdate {
                done: Some(true),
                ..StepUpdate::default()
            },
            "next: recovery tests",
        )
        .unwrap();
    assert_eq!(again.left_off.unwrap().text, "next: recovery tests");
    // A plain tick keeps the last line.
    let s2 = t.steps[1].id.clone();
    let plain =
        e.c.set_step(
            BOB,
            &t.id,
            &s2,
            StepUpdate {
                doing: Some(true),
                ..StepUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(plain.left_off.unwrap().text, "next: recovery tests");
    assert_eq!(
        e.reopen().get_task(&t.id).unwrap().left_off.unwrap().text,
        "next: recovery tests"
    );
}

#[test]
fn whoever_takes_it_over_hears_where_and_left_off() {
    let e = Env::new();
    let t = held(&e);
    e.c.set_where(&t.id, place("1a2b3c4d5e", true)).unwrap();
    e.c.set_step_left_off(
        BOB,
        &t.id,
        &t.steps[0].id,
        StepUpdate {
            done: Some(true),
            ..StepUpdate::default()
        },
        "lexer done; next: the parser",
    )
    .unwrap();
    // BOB stops halfway; the card stays where it got to, unheld.
    e.c.leave(BOB, &t.id, BOB).unwrap();
    let left = e.c.get_task(&t.id).unwrap();
    assert!(left.holders.is_empty());
    assert!(
        left.where_.is_some() && left.left_off.is_some(),
        "kept for whoever's next"
    );
    // Handing it to CAROL tells her where it is and where BOB left off.
    let (_, an) = e.c.assign(ALICE, &t.id, CAROL).unwrap();
    assert!(
        an.text.contains(
            "Where: branch feat/parser @ 1a2b3c4 (pushed) in /w/parser; continue from that branch"
        ),
        "{}",
        an.text
    );
    assert!(an.text.contains("Where: branch feat/parser"), "{}", an.text);
    assert!(an.text.contains("Left off: lexer done"), "{}", an.text);
}

#[test]
fn an_unheld_cards_offer_carries_them_too() {
    // A card put back before anyone started it (submitted, unheld) goes
    // out by offer; its notice carries what is known.
    let e = Env::new();
    let t =
        e.c.create_task(
            ALICE,
            NewTask {
                title: "Port the lexer".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    e.c.set_where(&t.id, place("abcdef1234", false)).unwrap();
    let at = gocompat::time::GoTime::unix(fixed_now().secs - 3600, 0);
    let carol = coord::roster::AgentCard {
        key: CAROL.into(),
        device_id: "dev-a".into(),
        status: "idle".into(),
        updated_at: at,
        last_activity: at,
        ..coord::roster::AgentCard::default()
    };
    let n = e.c.offer_card(ALICE, &t.id, &[carol]).unwrap();
    assert!(
        n.text.contains("Where: branch feat/parser @ abcdef1 (not pushed) in /w/parser; not pushed, so the last commits may be only on that machine"),
        "{}",
        n.text
    );
}

#[test]
fn a_place_outside_git_never_hides_a_git_one() {
    let e = Env::new();
    let t = held(&e);
    e.c.set_where(&t.id, place("1a2b3c4d5e", true)).unwrap();
    // CAROL takes over from her home folder: the branch stays on the card.
    let home = Where {
        who: CAROL.into(),
        cwd: "/Users/carol".into(),
        ..Where::default()
    };
    assert!(!e.c.set_where(&t.id, home).unwrap());
    assert_eq!(
        e.c.get_task(&t.id).unwrap().where_.unwrap().branch,
        "feat/parser"
    );
    // Once she is in a worktree, hers replaces it.
    let hers = Where {
        who: CAROL.into(),
        cwd: "/w/parser2".into(),
        worktree: "/w/parser2".into(),
        branch: "feat/parser".into(),
        head: "9f9f9f9f9f".into(),
        ..Where::default()
    };
    assert!(e.c.set_where(&t.id, hers).unwrap());
    assert_eq!(e.c.get_task(&t.id).unwrap().where_.unwrap().who, CAROL);
}

// ---- card #490: Where only from the card's own branch or worktree ----

fn at(worktree: &str, branch: &str) -> Where {
    Where {
        who: BOB.into(),
        cwd: worktree.into(),
        worktree: worktree.into(),
        branch: branch.into(),
        head: "1a2b3c4d5e".into(),
        ..Where::default()
    }
}

fn where_of(e: &Env, id: &str) -> Where {
    e.c.get_task(id).unwrap().where_.unwrap()
}

#[test]
fn a_command_in_another_worktree_does_not_move_where() {
    // #468: the holder ran one command in an unrelated audit worktree.
    let e = Env::new();
    let t = held(&e);
    assert!(e.c.set_where(&t.id, place("1a2b3c4d5e", false)).unwrap());
    let audit = at("/w/.claude/worktrees/audit-cards", "worktree-audit-cards");
    assert!(!e.c.set_where(&t.id, audit).unwrap());
    assert_eq!(where_of(&e, &t.id).worktree, "/w/parser");
    // Its own worktree still moves on: a new commit, a push.
    assert!(e.c.set_where(&t.id, place("9f8e7d6c5b", true)).unwrap());
    assert_eq!(where_of(&e, &t.id).head, "9f8e7d6c5b");
    // So does a replay.
    assert_eq!(
        e.reopen().get_task(&t.id).unwrap().where_.unwrap().worktree,
        "/w/parser"
    );
}

#[test]
fn from_the_main_checkout_it_moves_into_its_own_worktree() {
    // This card's own history: the agent's first command ran in the main
    // checkout, then it entered its own worktree.
    let e = Env::new();
    let t = held(&e);
    let n = e.c.get_task(&t.id).unwrap().short;
    assert!(e.c.set_where(&t.id, at("/w/irori", "main")).unwrap());
    let own = format!("/w/irori/.claude/worktrees/card-{n}-ids-holders");
    assert!(
        e.c.set_where(&t.id, at(&own, &format!("worktree-card-{n}-ids-holders")))
            .unwrap()
    );
    // An unnamed worktree from main is fine too: the first one it enters.
    let e2 = Env::new();
    let t2 = held(&e2);
    assert!(e2.c.set_where(&t2.id, at("/w/irori", "main")).unwrap());
    assert!(
        e2.c.set_where(
            &t2.id,
            at(
                "/w/irori/.claude/worktrees/sleepy-fox",
                "worktree-sleepy-fox"
            )
        )
        .unwrap()
    );
    // ...but from there a detour doesn't count, and neither does main.
    assert!(
        !e2.c
            .set_where(
                &t2.id,
                at("/w/irori/.claude/worktrees/audit", "worktree-audit")
            )
            .unwrap()
    );
    assert!(!e2.c.set_where(&t2.id, at("/w/irori", "main")).unwrap());
    assert_eq!(where_of(&e2, &t2.id).branch, "worktree-sleepy-fox");
}

#[test]
fn a_worktree_named_for_the_card_wins() {
    let e = Env::new();
    let t = held(&e);
    let n = e.c.get_task(&t.id).unwrap().short;
    assert!(e.c.set_where(&t.id, place("1a2b3c4d5e", false)).unwrap());
    // card-<n>9 is another card's.
    assert!(
        !e.c.set_where(
            &t.id,
            at(&format!("/w/card-{n}9-x"), &format!("card-{n}9-x"))
        )
        .unwrap()
    );
    assert!(
        e.c.set_where(&t.id, at(&format!("/w/card-{n}-x"), "feat/x"))
            .unwrap()
    );
    // Once in it, its old unnamed worktree is a detour.
    assert!(!e.c.set_where(&t.id, place("1a2b3c4d5e", false)).unwrap());
    assert_eq!(where_of(&e, &t.id).worktree, format!("/w/card-{n}-x"));
}

#[test]
fn a_new_holder_brings_its_own_where() {
    let e = Env::new();
    let t = held(&e);
    assert!(e.c.set_where(&t.id, place("1a2b3c4d5e", false)).unwrap());
    let carol = Where {
        who: CAROL.into(),
        ..at("/w/elsewhere", "feat/other")
    };
    assert!(e.c.set_where(&t.id, carol).unwrap());
    assert_eq!(where_of(&e, &t.id).who, CAROL);
}

#[test]
fn another_cards_worktree_is_never_where() {
    let e = Env::new();
    let t = held(&e);
    let n: i64 = e.c.get_task(&t.id).unwrap().short.parse().unwrap();
    let other = format!("/w/.claude/worktrees/card-{}-audit", n + 7);
    // Not as its first place...
    assert!(!e.c.set_where(&t.id, at(&other, "worktree-x")).unwrap());
    assert!(e.c.get_task(&t.id).unwrap().where_.is_none());
    // ...nor as the one move from main.
    assert!(e.c.set_where(&t.id, at("/w", "main")).unwrap());
    assert!(!e.c.set_where(&t.id, at(&other, "worktree-x")).unwrap());
    assert_eq!(where_of(&e, &t.id).worktree, "/w");
}

#[test]
fn from_a_main_checkout_on_another_branch_it_moves_into_a_nested_worktree() {
    // A repo whose default branch isn't main: the worktree it enters sits
    // inside the checkout.
    let e = Env::new();
    let t = held(&e);
    assert!(e.c.set_where(&t.id, at("/w/irori", "develop")).unwrap());
    assert!(
        e.c.set_where(
            &t.id,
            at(
                "/w/irori/.claude/worktrees/sleepy-fox",
                "worktree-sleepy-fox"
            )
        )
        .unwrap()
    );
    assert!(
        !e.c.set_where(
            &t.id,
            at("/w/irori/.claude/worktrees/audit", "worktree-audit")
        )
        .unwrap()
    );
    assert_eq!(where_of(&e, &t.id).branch, "worktree-sleepy-fox");
    // But a feature branch elsewhere is its own place: a detour from it doesn't count.
    let e2 = Env::new();
    let t2 = held(&e2);
    assert!(e2.c.set_where(&t2.id, at("/w/irori", "feat/x")).unwrap());
    assert!(!e2.c.set_where(&t2.id, at("/w/other", "feat/y")).unwrap());
}
