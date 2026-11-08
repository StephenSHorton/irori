//! Card #963: a card's weight in the window. Settings › Agents shows and
//! sets the light and heavy rows; a card shows its weight with the model
//! and effort its agent starts with; the person sets and clears it, and a
//! weight the person set is refused to an agent.

use crate::common;

use common::*;
use irori_window::agents::AgentsChange;

#[test]
fn settings_show_and_set_the_light_and_heavy_rows() {
    let r = Rig::new();
    let v = r.w.agents_settings("");
    assert_eq!(
        (
            v.light_model.as_str(),
            v.light_effort.as_str(),
            v.heavy_model.as_str(),
            v.heavy_effort.as_str()
        ),
        ("sonnet", "medium", "opus", "high")
    );
    assert!(!v.models.iter().any(|m| m == "haiku"), "{:?}", v.models);

    let heavy = |m: &str, e: &str| AgentsChange {
        heavy_model: Some(m.into()),
        heavy_effort: Some(e.into()),
        ..AgentsChange::default()
    };
    let v = r.w.agents_set(heavy("opus", "max")).unwrap();
    assert_eq!(
        (v.heavy_model.as_str(), v.heavy_effort.as_str()),
        ("opus", "max")
    );
    let s = r.w.core.spawn_setting("");
    // Opus is what heavy works out by itself: nothing is stored for it.
    assert_eq!(
        (s.heavy_model.as_str(), s.heavy_effort.as_str()),
        ("", "max")
    );

    // Back to its default: stored as "", so it follows the normal pick.
    let v = r.w.agents_set(heavy("opus", "high")).unwrap();
    assert_eq!(
        (v.heavy_model.as_str(), v.heavy_effort.as_str()),
        ("opus", "high")
    );
    let s = r.w.core.spawn_setting("");
    assert_eq!((s.heavy_model.as_str(), s.heavy_effort.as_str()), ("", ""));
    let v =
        r.w.agents_set(AgentsChange {
            effort: Some("xhigh".into()),
            ..AgentsChange::default()
        })
        .unwrap();
    assert_eq!(v.heavy_effort, "xhigh", "an empty heavy row follows normal");

    // Haiku is refused, on any row.
    let e = r.w.agents_set(heavy("haiku", "")).unwrap_err();
    assert!(e.contains("unknown model"), "{e}");
    assert!(
        r.w.agents_set(AgentsChange {
            light_effort: Some("huge".into()),
            ..AgentsChange::default()
        })
        .is_err()
    );
}

#[test]
fn a_card_shows_its_weight_with_the_mapped_model_and_effort() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let new = |title: &str| {
        co.create_task(
            &r.key_a,
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let (plain, heavy) = (new("plain"), new("heavy"));
    co.set_weight(&r.key_a, &heavy.id, "heavy", "planning the split")
        .unwrap();
    r.w.agents_set(AgentsChange {
        heavy_effort: Some("max".into()),
        ..AgentsChange::default()
    })
    .unwrap();

    let cards = r.w.cards();
    let of = |id: &str| cards.iter().find(|c| c.id == id).unwrap().clone();
    assert_eq!(of(&plain.id).weight, None);
    let w = of(&heavy.id).weight.expect("a weight on the summary");
    assert_eq!(
        (
            w.level.as_str(),
            w.reason.as_str(),
            w.set_by.as_str(),
            w.by_name.as_str(),
            w.model.as_str(),
            w.effort.as_str()
        ),
        (
            "heavy",
            "planning the split",
            "agent",
            "alpha",
            "opus",
            "max"
        )
    );

    let d = r.w.card(&heavy.id).unwrap();
    assert_eq!(d.summary.weight, Some(w));
    assert_eq!(r.w.card(&plain.id).unwrap().summary.weight, None);
    let j = serde_json::to_value(&d).unwrap();
    assert_eq!(j["weight"]["byName"], "alpha");
    assert!(
        serde_json::to_value(r.w.card(&plain.id).unwrap())
            .unwrap()
            .get("weight")
            .is_none()
    );
}

#[test]
fn the_person_sets_and_clears_a_weight_and_an_agent_cant_change_it() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "split it".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let got = r.w.set_weight(&t.id, "heavy", "").unwrap();
    let w = got.weight.expect("the weight it set");
    assert_eq!((w.level.as_str(), w.set_by.as_str()), ("heavy", "person"));
    let shown = r.w.card(&t.id).unwrap().summary.weight.unwrap();
    assert_eq!(
        (
            shown.set_by.as_str(),
            shown.by_name.as_str(),
            shown.model.as_str()
        ),
        ("person", "you", "opus")
    );

    let e = co
        .set_weight(&r.key_a, &t.id, "light", "busy work")
        .unwrap_err()
        .to_string();
    assert!(e.contains("set by the person"), "{e}");
    assert_eq!(co.get_task(&t.id).unwrap().weight.unwrap().level, "heavy");

    // An unknown level is refused.
    assert!(r.w.set_weight(&t.id, "huge", "").is_err());

    let got = r.w.set_weight(&t.id, "", "").unwrap();
    assert_eq!(got.weight, None);
    assert_eq!(r.w.card(&t.id).unwrap().summary.weight, None);
    // Cleared, an agent may weigh it again.
    co.set_weight(&r.key_a, &t.id, "light", "busy work")
        .unwrap();
    assert_eq!(co.get_task(&t.id).unwrap().weight.unwrap().set_by, "agent");
}

#[test]
fn on_a_shared_board_the_person_weighs_as_themselves_and_its_mapping_shows() {
    let r = Rig::new();
    let c = &r.w.core;
    let b = c.create_board("one", "one").unwrap();
    let co = c.board_coord(&b).unwrap();
    let me = c.board_key(&b, &c.user_sender().key).unwrap();
    let t = co
        .create_task(
            &me,
            coord::NewTask {
                title: "shared work".into(),
                ..Default::default()
            },
        )
        .unwrap();
    // The board maps light to Opus; the private board keeps the default.
    r.w.agents_set(AgentsChange {
        board_id: b.clone(),
        light_model: Some("opus".into()),
        ..AgentsChange::default()
    })
    .unwrap();
    let got = r.w.set_weight(&t.id, "light", "busy work").unwrap();
    assert_eq!(got.weight.unwrap().set_by, "person");
    assert!(
        c.coord().get_task(&t.id).is_none(),
        "it went to the board's log"
    );
    let w = r.w.card(&t.id).unwrap().summary.weight.unwrap();
    assert_eq!(
        (w.by_name.as_str(), w.model.as_str(), w.effort.as_str()),
        ("you", "opus", "medium")
    );
    let listed =
        r.w.cards()
            .into_iter()
            .find(|s| s.id == t.id)
            .and_then(|s| s.weight)
            .unwrap();
    assert_eq!(listed.model, "opus");
}
