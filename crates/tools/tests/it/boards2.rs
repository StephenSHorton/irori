//! The lease tools' shared-board hooks (v0.5 §5.4): a lease goes on the
//! boards the router names before the private lease is taken, another
//! person's lease there refuses it, and a release mirrors.

use crate::common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use coord::Coord;

/// A router whose one board holds another person's lease on `locked/**`.
struct FakeRouter {
    co: Arc<Coord>,
    calls: Mutex<Vec<String>>,
}

impl tools::TaskRouter for FakeRouter {
    fn task_coord(
        &self,
        _task_id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        Ok((self.co.clone(), keys.to_vec()))
    }
    fn private_notice(&self, n: coord::Notice) -> coord::Notice {
        n
    }
    fn task_ref(&self, r: &str) -> Result<String, coord::Error> {
        self.co.task_ref(r)
    }
    fn mirror_acquire(
        &self,
        key: &str,
        resource: &str,
        _ttl: Duration,
        steal: bool,
    ) -> Result<Vec<String>, coord::Error> {
        let s = if steal { " steal" } else { "" };
        self.calls
            .lock()
            .unwrap()
            .push(format!("acquire {key} {resource}{s}"));
        if resource.contains("locked") {
            // What the board's coord answers for another person's lease.
            return Err(coord::Error::raw(
                coord::ErrorKind::Held,
                format!(
                    "{resource} is leased in the shared space by agent:c2, another person's agent"
                ),
            ));
        }
        if resource.starts_with("path:") {
            return Ok(vec!["mlg".into()]);
        }
        Ok(Vec::new())
    }
    fn mirror_release(&self, key: &str, resource: &str) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("release {key} {resource}"));
    }
}

const SRC: &str = "path:remote:github.com/acme/app:src/**";
const LOCKED: &str = "path:remote:github.com/acme/app:locked/**";

#[test]
fn a_lease_mirrors_through_the_router() {
    let h = new_harness();
    let r = Arc::new(FakeRouter {
        co: h.b.co.clone(),
        calls: Mutex::new(Vec::new()),
    });
    assert!(h.b.router.set(r.clone()).is_ok());
    let a = call_a();
    let out = h.ok(&a, "lease", &format!(r#"{{"resource":"{SRC}"}}"#));
    has(&out, &["Leased", "Mirrored on mlg"]);
    let out = h.ok(&a, "lease", r#"{"resource":"port:5173"}"#);
    has_not(&out, &["Mirrored"]);
    h.fails(
        &a,
        "lease",
        &format!(r#"{{"resource":"{LOCKED}"}}"#),
        &["another person's agent"],
    );
    assert!(
        !h.b.co
            .leases()
            .iter()
            .any(|l| l.resource == LOCKED && l.holder == KEY_A),
        "a lease the board refused was taken privately"
    );
    h.ok(
        &a,
        "lease",
        &format!(r#"{{"resource":"{SRC}","steal":true}}"#),
    );
    h.ok(&a, "release", &format!(r#"{{"resource":"{SRC}"}}"#));
    let calls = r.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![
            format!("acquire {KEY_A} {SRC}"),
            format!("acquire {KEY_A} port:5173"),
            format!("acquire {KEY_A} {LOCKED}"),
            format!("acquire {KEY_A} {SRC} steal"),
            format!("release {KEY_A} {SRC}"),
        ]
    );
}

/// Card #418: the desk never merges. An old-style desk lease call, with the
/// run's pr and head_sha, gets the refusal that says to ask "Can I merge
/// PR #N?", not an unknown-argument error.
#[test]
fn an_old_desk_lease_call_points_at_the_merge_question() {
    let h = new_harness();
    let a = call_a();
    for args in [
        r#"{"resource":"desk/acme/app","pr":7,"head_sha":"abc123","base_sha":"def"}"#,
        r#"{"resource":"desk/acme/app"}"#,
    ] {
        h.fails(&a, "lease", args, &["never merges", "Can I merge PR #N?"]);
    }
}

// ---- prs shows who holds a review with capture off (card #1080) ----------

struct Reviews(Vec<tools::PrReviewLine>);

impl tools::TaskRouter for Reviews {
    fn task_coord(
        &self,
        _task_id: &str,
        _keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        unreachable!()
    }
    fn private_notice(&self, n: coord::Notice) -> coord::Notice {
        n
    }
    fn task_ref(&self, r: &str) -> Result<String, coord::Error> {
        Ok(r.into())
    }
    fn pr_reviews(&self, repo: &str) -> Vec<tools::PrReviewLine> {
        if repo == "acme/app" {
            self.0.clone()
        } else {
            Vec::new()
        }
    }
}

#[test]
fn prs_shows_the_claimer_with_capture_off() {
    let h = new_harness();
    let line = tools::PrReviewLine {
        number: 7,
        url: "https://github.com/acme/app/pull/7".into(),
        pick: "claimed".into(),
        review: "CHANGES_REQUESTED".into(),
        claimer: "fern (Maya)".into(),
        until: "2026-11-08T15:00:00Z".into(),
        related: vec!["kestrel (Kit): same files".into()],
        ..Default::default()
    };
    let waiting = tools::PrReviewLine {
        number: 8,
        url: "https://github.com/acme/app/pull/8".into(),
        pick: "waiting".into(),
        why_none: "nobody in the space is free to review this PR yet".into(),
        ..Default::default()
    };
    assert!(
        h.b.router
            .set(Arc::new(Reviews(vec![line, waiting])))
            .is_ok()
    );
    let a = call_a();
    let out = h.ok(&a, "prs", r#"{"repo":"acme/app"}"#);
    has(
        &out,
        &[
            "#7 https://github.com/acme/app/pull/7",
            "claimed by fern (Maya) until 2026-11-08T15:00:00Z",
            "GitHub CHANGES_REQUESTED",
            "related kestrel (Kit): same files",
            "#8",
            "nobody picked",
            "why: nobody in the space is free",
            "capture is off",
        ],
    );
    // A repo with nothing to say still answers "capture is off".
    h.fails(&a, "prs", r#"{"repo":"acme/other"}"#, &["capture is off"]);
}
