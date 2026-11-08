//! Card #492 (docs/design/work-personal-separation.md §7.4): the spawn
//! setting and "Offer it cards" count per device. Two paired Cores of one
//! person, A and B, on loopback: B's `spawn.set` never changes A's
//! setting or switch; a row from before per-device settings is taken once
//! and kept as A's own; B's `agent.offer` for A's agent is ignored on A,
//! and A refuses to set it for B's agent.

use crate::common;

use common::pair::{Dev, paired};
use common::wait_for;
use irori_core::spawn::{AgentOffer, SpawnSetting};
use store::{Filter, kinds};

/// The `spawn.set` rows in `on`'s store that `from` signed.
fn spawn_rows(on: &Dev, from: &Dev) -> Vec<serde_json::Value> {
    rows(on, from, kinds::SPAWN_SET)
}

fn rows(on: &Dev, from: &Dev, kind: &str) -> Vec<serde_json::Value> {
    on.core
        .store()
        .list(&Filter {
            kinds: vec![kind.into()],
            limit: 1000,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .filter(|ev| ev.device_id == from.core.device_id())
        .map(|ev| serde_json::from_slice(&ev.payload).unwrap())
        .collect()
}

/// Waits until `on` holds `n` rows of `kind` signed by `from`, then runs
/// its sweep, which reads the spawn rows again.
fn synced(on: &Dev, from: &Dev, kind: &str, n: usize) {
    wait_for(
        45,
        &format!("{n} {kind} row(s) reach the other device"),
        || rows(on, from, kind).len() >= n,
    );
    on.core.sweep_coord().unwrap();
}

/// A `spawn.set` as builds before card #492 wrote it: the same row for
/// every device, with no `device`.
fn legacy_set(d: &Dev, s: &SpawnSetting) {
    d.core.store().append("user", kinds::SPAWN_SET, s).unwrap();
}

fn setting(board_id: &str, on: bool, model: &str) -> SpawnSetting {
    SpawnSetting {
        board_id: board_id.into(),
        on,
        model: model.into(),
        ..SpawnSetting::default()
    }
}

#[test]
fn another_devices_spawn_setting_leaves_this_ones_alone() {
    let (a, b) = paired();
    // The work computer (A) spawns with opus; the Mac (B) turns its own
    // spawning off and picks opus.
    a.core.set_spawn_setting(setting("", true, "opus")).unwrap();
    b.core
        .set_spawn_setting(setting("", false, "opus"))
        .unwrap();
    synced(&a, &b, kinds::SPAWN_SET, 1);
    let s = a.core.spawn_setting("");
    assert!(s.on && s.model == "opus", "A keeps its own: {s:?}");
    // B turns it on and back off: A still counts only its own rows.
    b.core
        .set_spawn_setting(setting("", true, "sonnet"))
        .unwrap();
    b.core
        .set_spawn_setting(setting("", false, "sonnet"))
        .unwrap();
    synced(&a, &b, kinds::SPAWN_SET, 3);
    let s = a.core.spawn_setting("");
    assert!(s.on && s.model == "opus", "A keeps its own: {s:?}");
    // And the other way round: A's newest row doesn't change B's.
    a.core.set_spawn_setting(setting("", true, "opus")).unwrap();
    synced(&b, &a, kinds::SPAWN_SET, 2);
    let s = b.core.spawn_setting("");
    assert!(!s.on && s.model == "sonnet", "B keeps its own: {s:?}");
    // B's rows name B; A's name A.
    assert!(
        spawn_rows(&a, &b)
            .iter()
            .all(|v| v["device"] == b.core.device_id()),
        "{:?}",
        spawn_rows(&a, &b)
    );
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// A device with no setting of its own (the first load after the
/// upgrade) keeps running on the shared row it ran on before, written as
/// its own; later shared rows from an older build on the other device no
/// longer change it.
#[test]
fn a_shared_row_from_before_is_kept_and_written_as_this_devices_own() {
    let (a, b) = paired();
    // B, as an older build, turned spawning on for both.
    legacy_set(&b, &setting("", true, "opus"));
    synced(&a, &b, kinds::SPAWN_SET, 1);
    let s = a.core.spawn_setting("");
    assert!(s.on && s.model == "opus", "A inherits it: {s:?}");
    assert!(a.core.spawn_switch_set(""), "it counts as set");
    // A wrote it as its own row, naming A.
    let own = spawn_rows(&a, &a);
    assert!(
        own.len() == 1
            && own[0]["device"] == a.core.device_id()
            && own[0]["on"] == true
            && own[0]["model"] == "opus",
        "{own:?}"
    );
    // A second load writes nothing more.
    a.core.sweep_coord().unwrap();
    assert_eq!(spawn_rows(&a, &a).len(), 1);

    // B, still old, turns spawning off: A has its own row now, and keeps
    // spawning.
    legacy_set(&b, &setting("", false, "haiku"));
    synced(&a, &b, kinds::SPAWN_SET, 2);
    let s = a.core.spawn_setting("");
    assert!(s.on && s.model == "opus", "A keeps its own: {s:?}");
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// A device with no setting of its own takes the newest row of any
/// device once, even one an upgraded device wrote: an older build ran on
/// it, so upgrading changes nothing. From then on it keeps its own.
#[test]
fn a_device_with_no_setting_starts_from_the_newest_row_of_any_device() {
    let (a, b) = paired();
    b.core.set_spawn_setting(setting("", true, "opus")).unwrap();
    synced(&a, &b, kinds::SPAWN_SET, 1);
    let s = a.core.spawn_setting("");
    assert!(s.on && s.model == "opus", "A starts from B's: {s:?}");
    let own = spawn_rows(&a, &a);
    assert!(
        own.len() == 1 && own[0]["device"] == a.core.device_id(),
        "{own:?}"
    );
    b.core
        .set_spawn_setting(setting("", false, "opus"))
        .unwrap();
    synced(&a, &b, kinds::SPAWN_SET, 2);
    let s = a.core.spawn_setting("");
    assert!(s.on && s.model == "opus", "A keeps its own: {s:?}");
    a.core.close().unwrap();
    b.core.close().unwrap();
}

/// The seed is written once even when the spawn rows run past one page
/// of the store's list (1000): a load that saw only the first page would
/// write it again on every sweep.
#[test]
fn the_seed_is_written_once_past_a_page_of_rows() {
    let r = common::Rig::new();
    let st = r.core.store();
    let me = r.core.device_id().to_string();
    for i in 0..1000 {
        let cap = serde_json::json!({"device": format!("dev-{i}"), "cap": 3});
        st.append("user", kinds::SPAWN_CAP, &cap).unwrap();
    }
    st.append("user", kinds::SPAWN_SET, &setting("", true, "opus"))
        .unwrap();
    let own = || {
        st.list(&Filter {
            kinds: vec![kinds::SPAWN_SET.into()],
            limit: 1000,
            ..Default::default()
        })
        .unwrap()
        .into_iter()
        .filter(|ev| ev.device_id == me)
        .count()
    };
    for _ in 0..3 {
        r.core.sweep_coord().unwrap();
    }
    assert_eq!(own(), 2, "the old row and one seed");
    let s = r.core.spawn_setting("");
    assert!(s.on && s.model == "opus", "{s:?}");
    r.core.close().unwrap();
}

#[test]
fn another_devices_offer_for_this_devices_agent_is_ignored() {
    let (a, b) = paired();
    // An older build on B could offer A's agent cards; B's own agent it
    // may.
    let offer = |key: &str| AgentOffer {
        key: key.into(),
        on: true,
        ..AgentOffer::default()
    };
    b.core
        .store()
        .append("user", kinds::AGENT_OFFER, &offer(&a.agent))
        .unwrap();
    b.core.set_agent_offer(&b.agent, true).unwrap();
    synced(&a, &b, kinds::AGENT_OFFER, 2);
    assert!(!a.core.agent_offered(&a.agent), "B can't offer A's agent");
    assert!(a.core.agent_offered(&b.agent), "B's own agent counts on A");
    assert!(!b.core.agent_offered(&a.agent), "nor on B");

    // A's own row for its agent counts on both.
    a.core.set_agent_offer(&a.agent, true).unwrap();
    assert!(a.core.agent_offered(&a.agent));
    synced(&b, &a, kinds::AGENT_OFFER, 1);
    assert!(b.core.agent_offered(&a.agent), "A's row counts on B");
    a.core.close().unwrap();
    b.core.close().unwrap();
}

#[test]
fn offer_it_cards_for_another_devices_agent_is_refused() {
    let (a, b) = paired();
    let e = a.core.set_agent_offer(&b.agent, true).unwrap_err();
    assert!(e.contains("another device"), "{e}");
    assert!(
        rows(&a, &a, kinds::AGENT_OFFER).is_empty(),
        "nothing written"
    );
    assert!(!a.core.agent_offered(&b.agent));
    a.core.close().unwrap();
    b.core.close().unwrap();
}
