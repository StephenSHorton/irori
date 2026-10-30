//! The lease tools' shared-board hooks (v0.5 §5.4): a lease goes on the
//! boards the router names before the private lease is taken, another
//! person's lease there refuses it, and a release mirrors.

mod common;

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
    ) -> Result<Vec<String>, coord::Error> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("acquire {key} {resource}"));
        if resource.contains("locked") {
            // What the board's coord answers for another person's lease.
            return Err(coord::Error::raw(
                coord::ErrorKind::Held,
                format!(
                    "{resource} is leased on the shared board by agent:c2, another person's agent"
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
    h.ok(&a, "release", &format!(r#"{{"resource":"{SRC}"}}"#));
    let calls = r.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec![
            format!("acquire {KEY_A} {SRC}"),
            format!("acquire {KEY_A} port:5173"),
            format!("acquire {KEY_A} {LOCKED}"),
            format!("release {KEY_A} {SRC}"),
        ]
    );
}
