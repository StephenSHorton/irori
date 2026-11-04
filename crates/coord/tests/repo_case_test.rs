//! Card #480: the irori repo was split in two by letter case. A card
//! created with `repo: "remote:github.com/StephenSHorton/irori"` kept that
//! spelling, while every checkout and agent card is lowercase, so no agent
//! was ever offered it and no device ever found its checkout. Keys are now
//! canonical when written, and rows written before are read back canonical.

mod common;

use common::*;
use coord::roster::AgentCard;
use coord::*;
use gocompat::time::GoTime;
use store::kinds;

const MIXED: &str = "remote:github.com/StephenSHorton/irori";
const LOWER: &str = "remote:github.com/stephenshorton/irori";

fn nt(title: &str, repo: &str) -> NewTask {
    NewTask {
        title: title.into(),
        objective: "why it matters".into(),
        repo_key: repo.into(),
        ..NewTask::default()
    }
}

fn idle_in(key: &str, repo: &str) -> AgentCard {
    let at = GoTime::unix(fixed_now().secs - 60 * 30, 0);
    AgentCard {
        key: key.into(),
        device_id: "dev-a".into(),
        status: "idle".into(),
        updated_at: at,
        last_activity: at,
        repo_key: repo.into(),
        ..AgentCard::default()
    }
}

#[test]
fn a_new_mixed_case_card_is_stored_in_the_checkouts_spelling() {
    let e = Env::new();
    let t =
        e.c.create_task(ALICE, nt("Port the parser", MIXED))
            .unwrap();
    assert_eq!(t.repo_key, LOWER);
    let rows = e.rows(kinds::TASK_CREATED);
    let raw = String::from_utf8_lossy(&rows.last().unwrap().payload).to_string();
    assert!(raw.contains(LOWER) && !raw.contains(MIXED), "{raw}");
    // A trailing ".git" or "/" is the same repo too.
    let t2 =
        e.c.create_task(
            ALICE,
            nt("Two", "remote:github.com/StephenSHorton/irori.git/"),
        )
        .unwrap();
    assert_eq!(t2.repo_key, LOWER);
}

/// The migration: a task.created row from before #480 with the mixed-case
/// key (the live log has 101 of them) folds to the lowercase key, on a
/// fresh replay and live, and then matches a lowercase agent.
#[test]
fn an_old_mixed_case_row_reads_back_lowercase_and_gets_an_agent() {
    let e = Env::new();
    let made = e.c.create_task(ALICE, nt("Template", LOWER)).unwrap();
    let tmpl = e.rows(kinds::TASK_CREATED).pop().unwrap();
    let mut p: serde_json::Value = serde_json::from_slice(&tmpl.payload).unwrap();
    let old_id = "01M6JC5YEDEER78WWE350Z6A97";
    p["taskId"] = old_id.into();
    p["num"] = 447.into();
    p["title"] = "An old card from the PC".into();
    p["repoKey"] = MIXED.into();
    let at = tmpl.hlc.0 + 1_000_000;
    e.remote("dev-b", 1, at, &tmpl.actor, kinds::TASK_CREATED, &p);

    // Live fold.
    let t = e.c.get_task(old_id).expect("the old card folds");
    assert_eq!(t.repo_key, LOWER);
    // A replay from the store, as after a restart.
    let c2 = e.reopen();
    assert_eq!(c2.get_task(old_id).unwrap().repo_key, LOWER);
    // The signed row itself is untouched.
    let raw = e
        .rows(kinds::TASK_CREATED)
        .into_iter()
        .find(|r| String::from_utf8_lossy(&r.payload).contains(old_id))
        .unwrap();
    assert!(String::from_utf8_lossy(&raw.payload).contains(MIXED));

    // A board filter in either spelling finds it.
    for f in [MIXED, LOWER] {
        let ids: Vec<String> = c2
            .list_tasks(&TaskFilter {
                repo_key: f.into(),
                ..TaskFilter::default()
            })
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert!(
            ids.contains(&old_id.to_string()) && ids.contains(&made.id),
            "{f}: {ids:?}"
        );
    }

    // A free agent in the lowercase repo is offered it.
    let bob = idle_in(BOB, LOWER);
    let n = c2
        .offer_card(ALICE, old_id, std::slice::from_ref(&bob))
        .unwrap();
    assert_eq!(n.to, vec![BOB.to_string()], "{n:?}");
}

#[test]
fn an_agent_spelled_in_mixed_case_still_matches() {
    let e = Env::new();
    let t =
        e.c.create_task(ALICE, nt("Port the parser", LOWER))
            .unwrap();
    let carol = idle_in(CAROL, MIXED);
    let n =
        e.c.offer_card(ALICE, &t.id, std::slice::from_ref(&carol))
            .unwrap();
    assert_eq!(n.to, vec![CAROL.to_string()], "{n:?}");
}

#[test]
fn path_leases_in_either_case_are_one_lease() {
    let a = parse_resource(&format!("path:{MIXED}:src/**")).unwrap();
    let b = parse_resource(&format!("path:{LOWER}:src/**")).unwrap();
    assert_eq!(a.name, b.name);
    assert_eq!(a.repo_key, LOWER);
    // A root sha in capitals is the same lowercase root (it was refused
    // before #480).
    assert_eq!(
        normalize_resource("path:root:ABC:x").unwrap(),
        "path:root:abc:x"
    );
}

/// The reason a card wasn't handed out names the repo, not its key.
#[test]
fn the_idle_report_names_the_repo_not_its_key() {
    let e = Env::new();
    let t =
        e.c.create_task(ALICE, nt("Port the parser", MIXED))
            .unwrap();
    let other = idle_in(DAVE_KEY, REPO_B);
    e.c.offer_card(ALICE, &t.id, std::slice::from_ref(&other))
        .unwrap();
    let r = e.c.idle_report(&[other]);
    assert_eq!(r.unoffered.len(), 1, "{r:?}");
    assert_eq!(r.unoffered[0].why, "no free agent works in irori");
}

const DAVE_KEY: &str = "dev-a/dave-session";
