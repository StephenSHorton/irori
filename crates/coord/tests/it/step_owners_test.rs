//! Step owners (card #86): a step can name the agent doing it, which makes
//! that agent a co-holder of the card while the step is open.

use crate::common;

use common::*;
use coord::*;

fn nt(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        ..NewTask::default()
    }
}

fn get(c: &Coord, id: &str) -> Task {
    c.get_task(id).unwrap_or_else(|| panic!("no task {id}"))
}

fn step(text: &str, owner: &str) -> Step {
    Step {
        text: text.into(),
        owner: owner.into(),
        ..Step::default()
    }
}

fn tick() -> StepUpdate {
    StepUpdate {
        done: Some(true),
        doing: None,
    }
}

/// ALICE holds a card with three steps: s2 is BOB's.
fn card(e: &Env) -> (Task, Vec<Notice>) {
    let t = e.c.create_task(ALICE, nt("ship it")).unwrap();
    e.c.claim_task(ALICE, &t.id).unwrap();
    e.c.set_steps_notice(
        ALICE,
        &t.id,
        vec![
            step("build", ""),
            step("window half", BOB),
            step("land", ""),
        ],
    )
    .unwrap()
}

#[test]
fn test_owner_is_a_derived_co_holder() {
    let e = Env::new();
    let (t, ns) = card(&e);
    assert_eq!(t.steps[1].owner, BOB);
    assert_eq!(t.co_holders, vec![BOB.to_string()]);
    assert_eq!(t.holders, vec![ALICE.to_string()], "holders don't change");
    assert_eq!(ns.len(), 1, "{ns:?}");
    assert_eq!(ns[0].to, vec![BOB.to_string()]);
    assert_eq!(ns[0].kind, NOTICE_STEP_OWNER);
    assert!(ns[0].wake, "BOB is idle");
    assert!(ns[0].text.contains("s2 \"window half\""), "{}", ns[0].text);
}

#[test]
fn test_new_owner_busy_hears_next_turn_and_repeat_is_quiet() {
    let e = Env::new();
    e.ros.set(BOB, "busy");
    let (t, ns) = card(&e);
    assert!(!ns[0].wake);
    // Setting the same owners again tells nobody.
    let (_, ns) = e.c.set_steps_notice(ALICE, &t.id, t.steps.clone()).unwrap();
    assert!(ns.is_empty(), "{ns:?}");
}

#[test]
fn test_owner_ticks_own_step_and_others() {
    // Card #320: any agent may tick any step, a step owner included.
    let e = Env::new();
    let (t, _) = card(&e);
    e.c.set_step(BOB, &t.id, "s1", tick()).unwrap();
    e.c.set_step(BOB, &t.id, "s2", tick()).unwrap();
    let t = get(&e.c, &t.id);
    assert!(t.steps[1].done);
    assert!(t.co_holders.is_empty(), "done drops the co-holder");
    assert_eq!(t.holders, vec![ALICE.to_string()]);
}

#[test]
fn test_set_step_owner_and_clear() {
    let e = Env::new();
    let (t, _) = card(&e);
    let (t2, ns) = e.c.set_step_owner(ALICE, &t.id, "s3", CAROL).unwrap();
    assert_eq!(t2.co_holders, vec![BOB.to_string(), CAROL.to_string()]);
    assert_eq!(ns.len(), 1);
    assert_eq!(ns[0].to, vec![CAROL.to_string()]);
    let acts = e.c.activity(&t.id).unwrap();
    assert!(
        acts.iter()
            .any(|a| a.what.starts_with(&format!("gave “land” to {CAROL}"))),
        "{acts:?}"
    );
    let (t3, ns) = e.c.set_step_owner(ALICE, &t.id, "s3", "").unwrap();
    assert!(ns.is_empty());
    assert_eq!(t3.co_holders, vec![BOB.to_string()]);
    // Any agent may give steps out (card #320).
    e.c.set_step_owner(BOB, &t.id, "s1", BOB).unwrap();
}

#[test]
fn test_person_and_irori_cant_own_steps() {
    let e = Env::new();
    let (t, _) = card(&e);
    assert!(e.c.set_step_owner(ALICE, &t.id, "s1", USER).is_err());
    assert!(
        e.c.set_steps(ALICE, &t.id, vec![step("x", USER)]).is_err(),
        "the person owns no step"
    );
}

#[test]
fn test_holder_owning_a_step_is_not_a_co_holder() {
    let e = Env::new();
    let (t, _) = card(&e);
    e.c.set_step_owner(ALICE, &t.id, "s1", ALICE).unwrap();
    assert_eq!(get(&e.c, &t.id).co_holders, vec![BOB.to_string()]);
}

#[test]
fn test_co_holder_hears_comments_like_a_holder() {
    let e = Env::new();
    let (t, _) = card(&e);
    let (_, n) = e.c.add_comment(ALICE, &t.id, "status?", false).unwrap();
    assert!(n.to.contains(&BOB.to_string()), "{n:?}");
    let (_, ns) =
        e.c.person_comment(USER, &t.id, "how is it going", false, &[], None)
            .unwrap();
    assert!(ns.iter().any(|n| n.to.contains(&BOB.to_string())), "{ns:?}");
}

#[test]
fn test_co_holders_survive_a_claim_by_someone_else() {
    let e = Env::new();
    let (t, _) = card(&e);
    e.c.leave(ALICE, &t.id, ALICE).unwrap();
    e.c.claim_task(CAROL, &t.id).unwrap();
    let t = get(&e.c, &t.id);
    assert_eq!(t.holders, vec![CAROL.to_string()]);
    assert_eq!(t.co_holders, vec![BOB.to_string()]);
}
