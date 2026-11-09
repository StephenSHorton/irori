//! Assistants talk, card 3: another member's assistant is labelled and
//! mentioned only when it really is that member's: kind `assistant` and
//! their own profile points at it. Nobody else's `agent.card` row can
//! change its name or kind, and nothing can squat on the names @-mentions
//! use for it.

use crate::common;

use std::sync::Arc;

use common::mesh::{invited, person};
use common::wait_for;
use irori_core::spawn::{SpawnStarted, Spawner};
use irori_identity::kinds;

fn ready_spawner() -> Spawner {
    Spawner {
        ready: Some(Arc::new(|| true)),
        spawn: Arc::new(|r| {
            Ok(SpawnStarted {
                name: r.name.clone(),
                prefix: String::new(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_| true),
        list: None,
    }
}

#[test]
fn only_the_owners_real_assistant_is_labelled_and_mentioned() {
    let (a, b) = (
        person("stephen", 1).pop().unwrap(),
        person("eve", 1).pop().unwrap(),
    );
    a.core.set_chat_turn(Some(common::chat::idle_turn()));
    a.core.set_spawner(Some(ready_spawner()));
    let id = invited(&a, &b);
    let (a_id, b_id) = (a.core.person().id, b.core.person().id);
    // Eve's Core sees Stephen's assistant card and his profile pointing at it.
    wait_for(
        90,
        "Stephen's assistant is pointed at, on Eve's side",
        || {
            !b.core
                .board_verifier(&id)
                .unwrap()
                .profile_assistant_card(&a_id)
                .is_empty()
        },
    );
    let card = b
        .core
        .board_verifier(&id)
        .unwrap()
        .profile_assistant_card(&a_id);
    let key = format!("agent:{card}");
    let owner = b
        .core
        .board_verifier(&id)
        .unwrap()
        .profile(&a_id)
        .unwrap_or_default();
    let assistants = |c: &irori_core::Core| {
        c.board_agents()
            .into_iter()
            .filter(|x| x.assistant)
            .map(|x| (x.key, x.name))
            .collect::<Vec<_>>()
    };
    wait_for(30, "the assistant is labelled", || {
        assistants(&b.core).iter().any(|(k, _)| *k == key)
    });
    let label = format!("{owner}'s assistant");
    assert_eq!(
        assistants(&b.core)
            .into_iter()
            .find(|(k, _)| *k == key)
            .unwrap()
            .1,
        label
    );

    // Eve writes rows on Stephen's card id (their own personId), and
    // lookalike cards: one of kind claude named like his assistant, and one
    // of kind assistant that her profile doesn't point at.
    let st = b.core.board_store(&id).unwrap();
    st.append(
        &key,
        kinds::AGENT_CARD,
        &serde_json::json!({"cardId": card, "name": "Mallory", "kind": "claude",
            "personId": b_id, "status": "idle"}),
    )
    .unwrap();
    for (cid, name, kind) in [
        ("cLookalike", label.as_str(), "claude"),
        ("cUnpointed", label.as_str(), "assistant"),
    ] {
        st.append(
            &format!("agent:{cid}"),
            kinds::AGENT_CARD,
            &serde_json::json!({"cardId": cid, "name": name, "kind": kind,
                "personId": b_id, "status": "idle"}),
        )
        .unwrap();
    }
    // Stephen's side folds all three rows.
    wait_for(60, "Stephen sees Eve's lookalike cards", || {
        a.core
            .board_agents()
            .iter()
            .any(|x| x.key == "agent:cLookalike")
    });
    for c in [&a.core, &b.core] {
        // The rewritten row changed nothing: still the assistant, still
        // its label, under no other name.
        let got = assistants(c);
        assert!(got.iter().any(|(k, n)| *k == key && *n == label), "{got:?}");
        assert!(
            c.board_agents()
                .iter()
                .all(|x| x.name != "Mallory" && !(x.key != key && x.assistant)),
            "a lookalike or a rewritten row took the label"
        );
    }
    // Mentions: Stephen's assistant is listed under his name's forms for
    // Eve, the lookalikes are not, and @name resolves to the assistant and
    // not to the person.
    let list = b.core.board_mentionables(&id);
    let named: Vec<(&str, &str)> = list
        .iter()
        .map(|x| (x.key.as_str(), x.name.as_str()))
        .collect();
    assert!(named.contains(&(key.as_str(), label.as_str())), "{named:?}");
    assert!(
        named
            .iter()
            .all(|(k, _)| !k.ends_with("cLookalike") && !k.ends_with("cUnpointed")),
        "{named:?}"
    );
    let r = coord::mentions::resolve(&list, "", &format!("@{label} hello"), &[]);
    let keys: Vec<&str> = r.mentions.iter().map(|m| m.key.as_str()).collect();
    assert_eq!(keys, vec![key.as_str()], "{r:?}");
    // With a chosen name: "Juno · Addam's assistant", and @Juno finds it.
    a.core.set_assistant_name("Juno").unwrap();
    wait_for(60, "the chosen name reaches Eve", || {
        let l = coord::assistant_name::label(&owner, "Juno");
        b.core
            .board_agents()
            .iter()
            .any(|x| x.key == key && x.assistant && x.name == l)
    });
    let list = b.core.board_mentionables(&id);
    for text in ["@Juno hello", &format!("@{label} hello")] {
        let r = coord::mentions::resolve(&list, "", text, &[]);
        let keys: Vec<&str> = r.mentions.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, vec![key.as_str()], "{text}: {r:?}");
    }
    a.core.close().unwrap();
    b.core.close().unwrap();
}
