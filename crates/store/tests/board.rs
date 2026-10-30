//! The Go board-log tests (I:internal/store/board_test.go, store.md §6
//! "store boards").

mod common;

use std::sync::Arc;

use common::*;
use serde_json::{Value, json};
use store::kinds::*;
use store::{Clock, ErrorKind, Event, Side, Store, signing_bytes};

/// Go `boardEnv`: a board log on dev-a, with dev-b's key to forge
/// replicated rows from.
struct Env {
    dir: tempfile::TempDir,
    s: Option<Store>,
    ver: Arc<FakeVerifier>,
    a: KeySigner,
    b: KeySigner,
    clock_b: Clock,
    seq_b: i64,
}

impl Env {
    fn new() -> Env {
        let a = KeySigner::random();
        let b = KeySigner::random();
        let ver = FakeVerifier::new(&[("dev-a", a.public()), ("dev-b", b.public())]);
        let mut e = Env {
            dir: tempdir(),
            s: None,
            ver,
            a,
            b,
            clock_b: Clock::new(),
            seq_b: 0,
        };
        e.open();
        e
    }

    fn path(&self) -> std::path::PathBuf {
        self.dir
            .path()
            .join("boards")
            .join("0123456789abcdef0123456789abcdef.db")
    }

    fn open(&mut self) {
        let mut o = opts(Some(self.a.clone()), Some(self.ver.clone()));
        o.board = true;
        self.s = Some(Store::open_with(self.path(), "dev-a", clock(), o).unwrap());
    }

    fn s(&self) -> &Store {
        self.s.as_ref().unwrap()
    }

    /// Go `fromB`: a row dev-b signed, the next of dev-b's stream.
    fn row_b(&self, actor: &str, kind: &str, payload: &Value) -> Event {
        let mut ev = Event {
            device_id: "dev-b".into(),
            seq: self.seq_b + 1,
            hlc: self.clock_b.now(),
            actor: actor.into(),
            kind: kind.into(),
            payload: gocompat::json::marshal(payload).unwrap(),
            sig: vec![],
        };
        ev.sig = self.b.sign(&signing_bytes(&ev));
        ev
    }

    fn insert_b(&mut self, actor: &str, kind: &str, payload: &Value) -> (Event, store::Result<()>) {
        let ev = self.row_b(actor, kind, payload);
        let r = self.s().insert(&ev);
        if r.is_ok() {
            self.seq_b += 1;
        }
        (ev, r)
    }

    fn append(&self, actor: &str, kind: &str, payload: &Value) -> store::Result<()> {
        self.s().append(actor, kind, payload).map(|_| ())
    }

    fn must_append(&self, actor: &str, kind: &str, payload: &Value) {
        if let Err(e) = self.append(actor, kind, payload) {
            panic!("append {kind} as {actor}: {e}");
        }
    }
}

#[test]
fn board_refuses_kinds() {
    let mut e = Env::new();
    let pd = tempdir();
    let private = open_test(pd.path(), "dev-a");
    let mut refused: Vec<&str> = REGISTRY
        .iter()
        .filter(|k| k.side == Side::Replicated && k.board == store::Board::Refuses)
        .map(|k| k.kind)
        .collect();
    refused.sort();
    refused.push("future.kind");
    for want in [
        "message.sent",
        "capture.turn",
        "agent.seen",
        "policy.set",
        "person.device.added",
        "task.shared",
        "node.addr",
        "ask.posted",
        "board.joined",
        "receive.set",
        "board.sharedcopy",
        "board.cardof",
    ] {
        assert!(
            refused.contains(&want) || store::is_local(want),
            "{want} should be refused by a board log"
        );
    }
    let empty = json!({});
    for (i, kind) in refused.iter().enumerate() {
        assert_eq!(
            e.append("irori", kind, &empty).unwrap_err().kind(),
            ErrorKind::NotBoardKind,
            "append {kind}"
        );
        assert_eq!(
            e.insert_b("irori", kind, &empty).1.unwrap_err().kind(),
            ErrorKind::NotBoardKind,
            "insert {kind}"
        );
        assert_eq!(e.s().board_refused().kind, 2 * (i as i64 + 1));
        private.append("irori", kind, &empty).unwrap();
    }
    for kind in [POLICY_SET, MEMBER_JOINED, BOARD_CARD_KEY] {
        assert_eq!(
            e.append("irori", kind, &empty).unwrap_err().kind(),
            ErrorKind::LocalKind
        );
    }
    assert!(all(e.s()).is_empty(), "a refused row was stored");
}

#[test]
fn board_actors() {
    let mut e = Env::new();
    e.must_append(
        "agent:c1",
        AGENT_CARD,
        &json!({"cardId": "c1", "name": "lead", "kind": "claude", "personId": "p1", "status": "active"}),
    );
    let cases = [
        ("irori", true),
        ("person:p1", true),
        ("agent:c1", true),
        ("user", false),
        ("user:dev-a", false),
        ("user:anything", false),
        ("api:dev-a/s1", false),
        ("agent:dev-a/s1", false),
        ("agent:", false),
        ("person:", false),
        ("person:p1/x", false),
        ("", false),
    ];
    for (actor, ok) in cases {
        let row = json!({"taskId": format!("t-{actor}"), "title": "x", "by": ""});
        let r = e.append(actor, TASK_CREATED, &row);
        assert_eq!(r.is_ok(), ok, "append as {actor:?}: {r:?}");
        if !ok {
            assert_eq!(r.unwrap_err().kind(), ErrorKind::BoardActor);
        }
        let (_, r) = e.insert_b(actor, TASK_CREATED, &row);
        assert_eq!(r.is_ok(), ok, "insert as {actor:?}: {r:?}");
        if !ok {
            assert_eq!(r.unwrap_err().kind(), ErrorKind::BoardActor);
        }
    }
    assert_eq!(
        e.append("agent:c1", AGENT_CARD, &json!({"cardId": "c2"}))
            .unwrap_err()
            .kind(),
        ErrorKind::BoardActor
    );
    assert_eq!(
        e.insert_b("person:p1", AGENT_CARD, &json!({"cardId": "c3"}))
            .1
            .unwrap_err()
            .kind(),
        ErrorKind::BoardActor
    );
}

#[test]
fn board_closure() {
    let mut e = Env::new();
    let sha = "a".repeat(64);
    let refused: Vec<(&str, &str, Value)> = vec![
        (
            "a private task",
            "task.comment",
            json!({"taskId": "private-task", "commentId": "m1", "by": "person:p1", "text": "hi"}),
        ),
        (
            "a private agent key",
            "task.created",
            json!({"taskId": "t9", "by": "dev-a/session-1"}),
        ),
        (
            "a user key",
            "task.assigned",
            json!({"taskId": "t1", "to": "user:dev-a", "by": "person:p1"}),
        ),
        (
            "an unknown card",
            "task.joined",
            json!({"taskId": "t1", "who": "agent:nobody", "by": "agent:nobody"}),
        ),
        (
            "a blocker of another card",
            "blocker.option",
            json!({"taskId": "t1", "blockerId": "b1", "answer": "x", "by": "person:p1"}),
        ),
        (
            "an unknown step",
            "task.step",
            json!({"taskId": "t1", "stepId": "s9", "by": "person:p1"}),
        ),
        (
            "an unoffered attachment",
            "task.comment",
            json!({"taskId": "t1", "commentId": "m2", "by": "person:p1", "attachments": [{"sha256": sha}]}),
        ),
        (
            "a private message",
            "task.comment",
            json!({"taskId": "t1", "commentId": "m3", "msgId": "m3", "by": "person:p1"}),
        ),
        (
            "a private blocked-on task",
            "task.status",
            json!({"taskId": "t1", "state": "blocked", "by": "person:p1", "blockedBy": {"kind": "task", "ref": "private-task"}}),
        ),
        (
            "an unmirrored lease",
            "lease.released",
            json!({"resource": "gpu/qwen", "holder": "agent:c1", "by": "agent:c1"}),
        ),
        (
            "a private parent",
            "task.parent",
            json!({"taskId": "t1", "parent": "private-task", "by": "person:p1"}),
        ),
        (
            "a subtask of a private card",
            "task.created",
            json!({"taskId": "t5", "parent": "private-task", "by": "person:p1"}),
        ),
        (
            "a person not on the board",
            "task.assigned",
            json!({"taskId": "t1", "to": "agent:c1", "by": "person:p9"}),
        ),
        (
            "an outcome naming a private card",
            "task.status",
            json!({"taskId": "t1", "state": "completed", "by": "person:p1", "outcome": {"kind": "edit", "task": "private-task"}}),
        ),
        (
            "a lease stolen from a private key",
            "lease.acquired",
            json!({"resource": "gpu/x", "holder": "agent:c1", "stolenFrom": "agent:c1,dev-a/session-1"}),
        ),
    ];
    e.must_append(
        "person:p1",
        BOARD_CREATED,
        &json!({"boardId": "b", "name": "mlg", "tag": "mlg", "creator": {"personId": "p1"}}),
    );
    assert_eq!(
        e.append(
            "person:p1",
            TASK_CREATED,
            &json!({"taskId": "t0", "by": "person:p2"})
        )
        .unwrap_err()
        .kind(),
        ErrorKind::OutsideBoard
    );
    e.must_append(
        "person:p1",
        BOARD_MEMBER_ADDED,
        &json!({"personId": "p2", "by": "p1", "role": "member"}),
    );
    e.must_append(
        "person:p1",
        BOARD_MEMBER_ROLE,
        &json!({"personId": "p2", "by": "p1", "role": "owner"}),
    );
    e.must_append(
        "person:p1",
        BOARD_MEMBER_REMOVED,
        &json!({"personId": "p2", "by": "p1", "cut": {"dev-b": 0}}),
    );
    e.must_append(
        "person:p1",
        TASK_CREATED,
        &json!({"taskId": "t1", "title": "one", "by": "person:p1"}),
    );
    e.must_append(
        "agent:c1",
        AGENT_CARD,
        &json!({"cardId": "c1", "personId": "p1", "status": "active"}),
    );
    for (name, kind, row) in &refused {
        let err = e.append("person:p1", kind, row).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::OutsideBoard, "{name}: {err}");
    }
    assert_eq!(e.s().board_refused().outside, refused.len() as i64 + 1);
    e.must_append(
        "person:p1",
        TASK_STEPS,
        &json!({"taskId": "t1", "steps": [{"id": "s1", "text": "go"}], "by": "person:p1"}),
    );
    e.must_append(
        "person:p1",
        TASK_STEP,
        &json!({"taskId": "t1", "stepId": "s1", "by": "person:p1"}),
    );
    e.must_append(
        "person:p1",
        TASK_ASSIGNED,
        &json!({"taskId": "t1", "to": "agent:c1", "by": "person:p1"}),
    );
    e.must_append("agent:c1", TASK_STATUS, &json!({"taskId": "t1", "state": "blocked", "by": "agent:c1", "blocker": {"id": "b1", "question": "which?"}}));
    e.must_append(
        "person:p1",
        BLOCKER_OPTION,
        &json!({"taskId": "t1", "blockerId": "b1", "answer": "x", "by": "person:p1"}),
    );
    e.must_append(
        "irori",
        ATTACH_OFFERED,
        &json!({"sha256": sha, "size": 1, "name": "a.txt"}),
    );
    e.must_append("person:p1", TASK_COMMENT, &json!({"taskId": "t1", "commentId": "m2", "by": "person:p1", "attachments": [{"sha256": sha}]}));
    e.must_append(
        "person:p1",
        TASK_LINKED,
        &json!({"taskId": "t1", "link": {"kind": "commit", "ref": "abc1234"}, "by": "person:p1"}),
    );
    e.must_append(
        "agent:c1",
        LEASE_ACQUIRED,
        &json!({"resource": "gpu/qwen", "holder": "agent:c1"}),
    );
    e.must_append(
        "agent:c1",
        LEASE_RELEASED,
        &json!({"resource": "gpu/qwen", "holder": "agent:c1", "by": "agent:c1"}),
    );

    let (early, r) = e.insert_b(
        "agent:c2",
        TASK_COMMENT,
        &json!({"taskId": "t2", "commentId": "m9", "by": "agent:c2", "text": "early"}),
    );
    r.unwrap();
    assert_eq!(
        e.s().unresolved(&early).unwrap_err().kind(),
        ErrorKind::OutsideBoard
    );
    e.insert_b(
        "person:p2",
        TASK_CREATED,
        &json!({"taskId": "t2", "title": "two", "by": "person:p2"}),
    )
    .1
    .unwrap();
    assert_eq!(
        e.s().unresolved(&early).unwrap_err().kind(),
        ErrorKind::OutsideBoard
    );
    e.insert_b(
        "agent:c2",
        AGENT_CARD,
        &json!({"cardId": "c2", "personId": "p2", "status": "active"}),
    )
    .1
    .unwrap();
    e.s().unresolved(&early).unwrap();

    e.s().close().unwrap();
    e.open();
    e.must_append(
        "agent:c2",
        TASK_COMMENT,
        &json!({"taskId": "t2", "commentId": "m10", "by": "agent:c2", "text": "after reopen"}),
    );
    assert_eq!(
        e.append(
            "person:p1",
            TASK_COMMENT,
            &json!({"taskId": "private-task", "commentId": "m11", "by": "person:p1"})
        )
        .unwrap_err()
        .kind(),
        ErrorKind::OutsideBoard
    );
    let pd = tempdir();
    let private = open_test(pd.path(), "dev-a");
    private.unresolved(&early).unwrap();
    assert!(!private.is_board() && e.s().is_board());
}

#[test]
fn board_verifier_hook() {
    let mut e = Env::new();
    e.must_append(
        "irori",
        MEMBER_PROFILE,
        &json!({"personId": "p1", "name": "stephen"}),
    );
    e.must_append(
        "irori",
        TASK_CREATED,
        &json!({"taskId": "t1", "by": "irori"}),
    );
    e.insert_b(
        "irori",
        BOARD_RENAMED,
        &json!({"name": "mlg", "by": "person:p2"}),
    )
    .1
    .unwrap();
    assert_eq!(e.ver.observed_kinds(), [MEMBER_PROFILE, BOARD_RENAMED]);
    e.s().close().unwrap();
    e.ver.reset_for_test();
    e.open();
    assert_eq!(e.ver.observed_kinds().len(), 2, "observed at open");
    let x = KeySigner::random();
    let mut ev = Event {
        device_id: "dev-x".into(),
        seq: 1,
        hlc: Clock::new().now(),
        actor: "person:p9".into(),
        kind: TASK_CREATED.into(),
        payload: br#"{"taskId":"t9"}"#.to_vec(),
        sig: vec![],
    };
    ev.sig = x.sign(&signing_bytes(&ev));
    assert_eq!(e.s().insert(&ev).unwrap_err().kind(), ErrorKind::NoCert);
}

trait ResetForTest {
    fn reset_for_test(&self);
}

impl ResetForTest for FakeVerifier {
    fn reset_for_test(&self) {
        store::Verifier::reset(self);
    }
}
