//! Card #418: the merge desk never merges. Rows from the old holder's
//! merge run still read safely: `repo.desk`'s holder, check lines and
//! merge method are dropped, `desk.run`, `desk.hold` and `desk.pause` are
//! folded past, and nobody can take a `desk/<owner>/<name>` lease.

use std::time::Duration;

use crate::common;
use common::*;
use serde_json::json;
use store::kinds;

const OLD_USER: &str = "user:dev-b";
const OLD_HOLDER: &str = "dev-b/heron";

/// An Env whose person owns user:dev-a and user:dev-b.
fn env() -> Env {
    let owner: KeyFn = std::sync::Arc::new(|k: &str| k.starts_with("user:"));
    Env::with("dev-a", Some(owner))
}

/// What an older device wrote while the desk still had a holder.
fn old_rows(e: &Env) {
    let at = |m: u64| hlc_at(plus(fixed_now(), mins(m)));
    let t = "2026-11-05T10:00:00Z";
    e.remote(
        "dev-b",
        1,
        at(1),
        "user",
        kinds::REPO_DESK,
        &json!({"repoKey": REPO_A, "enabled": true, "takesPrs": true,
            "holder": OLD_HOLDER, "pollDevice": "dev-b",
            "checks": [{"command": "sh check.sh", "paths": ["src/**"], "runner": OLD_HOLDER}],
            "mergeMethod": "rebase", "by": OLD_USER, "at": t}),
    );
    e.remote(
        "dev-b",
        2,
        at(2),
        "user",
        kinds::DESK_HOLD,
        &json!({"repoKey": REPO_A, "pr": 7, "note": "not yet", "by": OLD_USER, "at": t}),
    );
    e.remote(
        "dev-b",
        3,
        at(3),
        "user",
        kinds::DESK_PAUSE,
        &json!({"repoKey": REPO_A, "paused": true, "by": OLD_USER, "at": t}),
    );
    e.remote(
        "dev-b",
        4,
        at(4),
        "agent:dev-b/heron",
        kinds::DESK_RUN,
        &json!({"repoKey": REPO_A, "pr": 7, "headSha": "abc123", "baseSha": "def456", "by": OLD_HOLDER, "at": t}),
    );
}

#[test]
fn old_holder_rows_still_read_and_the_desk_lists_prs() {
    let e = env();
    old_rows(&e);
    for c in [&e.c, &e.reopen()] {
        let d = c.desk(REPO_A).expect("the old row still sets the desk up");
        assert!(d.on(), "a paused, held old desk is just on now");
        assert!(!d.is_default());
        assert_eq!(d.settings.poll_device, "dev-b");
        let v = serde_json::to_value(&d).unwrap();
        for gone in ["holder", "checks", "mergeMethod"] {
            assert!(v["settings"].get(gone).is_none(), "{gone}: {v}");
        }
        for gone in ["holds", "pause", "run"] {
            assert!(v.get(gone).is_none(), "{gone}: {v}");
        }
    }
    // A save from the app writes a row with none of the old keys, so an
    // older device reading it sees the desk unheld.
    let s = e.c.desk(REPO_A).unwrap().settings;
    e.c.set_desk(USER, s).unwrap();
    let rows = e.rows(kinds::REPO_DESK);
    let last = &rows.last().unwrap().payload;
    let last: serde_json::Value = serde_json::from_slice(last).unwrap();
    for gone in ["holder", "checks", "mergeMethod"] {
        assert!(last.get(gone).is_none(), "{gone}: {last}");
    }
}

#[test]
fn nobody_takes_a_desk_lease_any_more() {
    let e = env();
    old_rows(&e);
    for steal in [false, true] {
        let err =
            e.c.acquire(ALICE, "desk/a/b", Duration::from_secs(600), steal)
                .unwrap_err()
                .to_string();
        assert!(err.contains("never merges"), "{err}");
        assert!(err.contains("Can I merge PR #N?"), "{err}");
    }
    // Other named leases are untouched.
    e.c.acquire(ALICE, "gpu/qwen", Duration::from_secs(600), false)
        .unwrap();
}
