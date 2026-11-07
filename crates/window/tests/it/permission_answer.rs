//! Card #682: answering a spawned agent's permission prompt in the window.
//! A pick resolves the typed item as the person (the agent's device hands
//! it to the waiting hook); Always allow also adds its rule to the card's
//! board's allow list, once; the card shows what waits and whether Irori
//! can answer it.

use crate::common;

use common::*;
use coord::{
    BlockerOption, NewBlocker, NewTask, PERMISSION_ALLOW_ONCE, PERMISSION_ALWAYS, PERMISSION_DENY,
    PermissionWait,
};

fn opt(a: &str) -> BlockerOption {
    BlockerOption {
        answer: a.into(),
        ..BlockerOption::default()
    }
}

#[test]
fn always_allow_resolves_the_prompt_and_adds_the_rule() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let me = r.w.core.device_id().to_string();
    let person = format!("user:{me}");
    let t = co
        .create_task(
            &person,
            NewTask {
                title: "Ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let raise = |request: &str| {
        co.raise(
            "irori",
            &t.id,
            NewBlocker {
                question: format!(
                    "Waiting for your permission: thyme wants to use Bash ({request})"
                ),
                options: vec![
                    opt(PERMISSION_ALLOW_ONCE),
                    opt(PERMISSION_ALWAYS),
                    opt(PERMISSION_DENY),
                ],
                permission: Some(PermissionWait {
                    agent: format!("{me}/sess"),
                    tool: "Bash".into(),
                    input: "cargo test".into(),
                    rule: "Bash(cargo test)".into(),
                    request: request.into(),
                }),
                ..NewBlocker::default()
            },
        )
        .unwrap()
        .1
    };

    // The card says what waits, and that the card can answer it.
    let b = raise("h1");
    let d = r.w.card(&t.id).unwrap();
    let cb = d.blockers.iter().find(|x| x.id == b.id).unwrap();
    let p = cb.permission.as_ref().expect("a typed item");
    assert_eq!(
        (
            p.tool.as_str(),
            p.input.as_str(),
            p.rule.as_str(),
            p.answerable
        ),
        ("Bash", "cargo test", "Bash(cargo test)", true)
    );
    assert!(
        r.w.cards()
            .iter()
            .any(|s| s.id == t.id && s.permission_wait),
        "the summary flags it"
    );

    let res = r.w.answer(&t.id, &b.id, 1, "", true, &[]).unwrap();
    assert!(res.resolved && res.irori_acts, "{res:?}");
    let done = co
        .get_task(&t.id)
        .unwrap()
        .blockers
        .into_iter()
        .find(|x| x.id == b.id)
        .unwrap();
    let rs = done.resolved.unwrap();
    assert_eq!((rs.option, rs.by.as_str()), (1, person.as_str()));
    assert_eq!(r.w.core.spawn_setting("").allow, vec!["Bash(cargo test)"]);

    // Again: the rule isn't added twice. Allow once adds nothing.
    let b2 = raise("h2");
    r.w.answer(&t.id, &b2.id, 1, "", true, &[]).unwrap();
    let b3 = raise("h3");
    r.w.answer(&t.id, &b3.id, 0, "", true, &[]).unwrap();
    assert_eq!(r.w.core.spawn_setting("").allow, vec!["Bash(cargo test)"]);

    // An item only presence saw (no request): nothing for the card to
    // answer, so no rule is offered.
    let (_, b4) = co
        .raise(
            "irori",
            &t.id,
            NewBlocker {
                question: "Waiting for your permission: thyme is stopped on a prompt".into(),
                permission: Some(PermissionWait {
                    agent: format!("{me}/sess"),
                    tool: "Bash".into(),
                    ..PermissionWait::default()
                }),
                ..NewBlocker::default()
            },
        )
        .unwrap();
    let d = r.w.card(&t.id).unwrap();
    let p = d
        .blockers
        .iter()
        .find(|x| x.id == b4.id)
        .and_then(|x| x.permission.clone())
        .unwrap();
    assert!(!p.answerable && p.rule.is_empty(), "{p:?}");
}
