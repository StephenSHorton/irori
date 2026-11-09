//! Card #1240: a removed device is forgotten. Two real Cores of one person
//! on loopback iroh: B has a name the person gave it, a side pin, a pause
//! and a host row, and made a card. Once A removes B and the removal has
//! settled, nothing A shows or keeps per device names B: not by its name,
//! the computer's, its id or the start of its id. The chain's revoke stays
//! (A still refuses B's later rows).

use std::path::Path;
use std::sync::Arc;

use crate::common::mesh::{Dev, person};
use crate::common::wait_for;
use coord::NewTask;
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::spawn_why::Why;

const REPO: &str = "remote:github.com/stephen/api";
const WAIT: u64 = 90;

fn idle_spawner() -> Spawner {
    Spawner {
        spawn: Arc::new(|_: &SpawnReq| {
            Ok(SpawnStarted {
                name: "never".into(),
                prefix: "never".into(),
            })
        }),
        end: Arc::new(|_: &str| Ok(String::new())),
        trusted: Arc::new(|_: &Path| true),
        list: None,
        ready: None,
        stop: None,
    }
}

fn seen(d: &Dev, repo: &str, dir: &Path) {
    d.core
        .capture()
        .emit_row(
            "irori",
            capture::KIND_REPO_SEEN,
            &capture::RepoSeen {
                repo_key: repo.into(),
                root: dir.to_string_lossy().into_owned(),
                at: gocompat::time::GoTime::unix(1_790_000_000, 0),
            },
        )
        .unwrap();
}

/// Every way B could be named: the person's name for it, the computer's,
/// its id and the start of its id.
struct Names(Vec<String>);

impl Names {
    fn check(&self, what: &str, text: &str) {
        for n in &self.0 {
            assert!(!text.contains(n.as_str()), "{what} names B ({n}): {text}");
        }
    }
}

#[test]
fn a_removed_device_is_named_nowhere_once_its_removal_settles() {
    let mut st = person("stephen", 2);
    let b = st.pop().unwrap();
    let a = st.pop().unwrap();
    let b_id = b.core.device_id().to_string();
    // A's Agents switch on, so a card's line is about its device.
    a.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    wait_for(WAIT, "A's switch is on in the coord", || {
        a.core.coord().spawning()
    });

    // B as the person knows it: renamed, pinned to a side only it runs,
    // paused, and able to run agents in a repo.
    a.core.set_device_name(&b_id, "Studio Mac").unwrap();
    let checkout = tempfile::tempdir().unwrap();
    seen(&b, REPO, checkout.path());
    b.core.set_spawner(Some(idle_spawner()));
    let t = b
        .core
        .coord()
        .create_task(
            &b.user(),
            NewTask {
                title: "Made on B".into(),
                repo_key: REPO.into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    // After the card, so the card carries no side: a side only B's pin
    // names goes with B, a card's side stays.
    b.core.set_device_sides(&["Lab".into()]).unwrap();
    b.core.set_agents_paused(true).unwrap();
    wait_for(
        WAIT,
        "A knows B's name, side, pause, host row and card",
        || {
            a.core.device_label(&b_id) == "Studio Mac"
                && a.core.device_sides().contains_key(&b_id)
                && a.core.side_names().iter().any(|s| s == "Lab")
                && a.core.agents_pauses().get(&b_id).is_some_and(|p| p.on)
                && a.core.spawn_hosts().contains_key(&b_id)
                && a.core.coord().get_task(&t.id).is_some()
                && a.core
                    .mesh_devices(false)
                    .iter()
                    .any(|r| r.device_id == b_id)
        },
    );
    let computer = a
        .core
        .chain()
        .cert(&b_id)
        .map(|c| c.label)
        .unwrap_or_default();
    assert!(!computer.is_empty(), "B's certificate has a label");
    let names = Names(vec![
        "Studio Mac".into(),
        computer,
        b_id.clone(),
        b_id[..8].to_string(),
    ]);

    let rt = a.core.pairing_runtime().clone();
    rt.block_on(a.core.remove_device(&b_id)).unwrap();
    // Two devices: A is the only one left, so the removal settles at once.
    wait_for(WAIT, "B's row leaves A's Devices list", || {
        !a.core
            .mesh_devices(false)
            .iter()
            .any(|r| r.device_id == b_id)
    });
    assert!(a.core.chain().cut(&b_id).is_some(), "the revoke stays");
    assert!(a.core.device_removed(&b_id));

    // The Devices list and the mesh status, with every detail.
    let rows = serde_json::to_string(&a.core.mesh_devices(true)).unwrap();
    names.check("Devices", &rows);
    assert!(!rows.contains("Removed."), "no \"Removed.\" row: {rows}");
    names.check(
        "status",
        &serde_json::to_string(&a.core.net_status()).unwrap(),
    );

    // Names, sides, pauses and what can run agents.
    let labels = a.core.device_labels();
    names.check("device labels", &format!("{labels:?}"));
    assert_eq!(a.core.device_label(&b_id), "");
    names.check("device sides", &format!("{:?}", a.core.device_sides()));
    assert!(
        !a.core.side_names().iter().any(|s| s == "Lab"),
        "a side only B ran is gone: {:?}",
        a.core.side_names()
    );
    names.check("pauses", &format!("{:?}", a.core.agents_pauses().keys()));
    names.check("hosts", &format!("{:?}", a.core.spawn_hosts().keys()));
    names.check("agent devices", &format!("{:?}", a.core.agent_devices()));
    names.check(
        "file holders",
        &a.core.device_names(std::slice::from_ref(&b_id)),
    );

    // B's card says its device was removed, without naming it.
    wait_for(WAIT, "A's line for B's card says B was removed", || {
        matches!(a.core.why_no_agent(&t.id, true), Some(Why::DeviceRemoved))
    });
    names.check("why", &Why::DeviceRemoved.to_string());

    // Per-device files: the peers file forgets B, the settled file keeps
    // the removal settled after a restart.
    let read = |f: &str| std::fs::read_to_string(a.data.join(f)).unwrap_or_default();
    names.check(mesh::PEERS_FILE, &read(mesh::PEERS_FILE));
    assert!(
        read(mesh::REMOVED_FILE).contains(&b_id),
        "A records the settled removal"
    );
}
