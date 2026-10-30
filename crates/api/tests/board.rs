//! The Board browsing routes' viewer scoping and paging (I:internal/api
//! board_test.go TestBoardSearchViewerScoping, TestBoardSearchFiltersAndPaging,
//! TestBoardShowViewerScoping, TestBoardShowPaging, TestBoardOverview) on
//! the real Core behind the API.
//!
//! Go's fixture is a fake backend with two agents, the attached Claude
//! session dev1/s1 ("alpha") and the Grok member dev1/grok-1 ("gamma").
//! Here they are the rig's session A, attached over `/v1/attach`, and a
//! member joined over `/v1/members` with its stream open; the rig's
//! session B is a third live agent. The sessions' cwd is no git repo, so
//! the agents' repo key is "" and the seeded cards carry Go's repo key.

mod common;

use std::time::Duration;

use irori_api::client::{Opened, Response};
use serde_json::Value;

const BOARD_REPO: &str = "remote:github.com/owner/board";

struct Fixture {
    rig: common::Rig,
    /// A's attach secret (its stream stays open for the fixture's life).
    attach: String,
    member_key: String,
    member_secret: String,
    person: String,
    assigned: coord::Task,
    blocked: coord::Task,
    open: coord::Task,
    comment: coord::Comment,
    /// Message ids, the person to each agent.
    msg_to_member: String,
    msg_to_claude: String,
}

/// Opens a stream, returns its hello line and drains the rest on the
/// rig's runtime for the test's life.
fn open_stream(rig: &common::Rig, path: &str) -> Value {
    rig.rt.block_on(async {
        let Opened::Stream(mut lines, _) = rig.client.open(path, &[]).await.unwrap() else {
            panic!("{path}: refused")
        };
        let hello = lines
            .next_json(Duration::from_secs(5))
            .await
            .expect("hello");
        tokio::spawn(async move { while lines.next().await.is_some() {} });
        serde_json::from_slice(&hello).unwrap()
    })
}

fn new_fixture() -> Fixture {
    let rig = common::Rig::new();
    let joined = rig
        .rt
        .block_on(rig.client.call(
            "POST",
            "/v1/members",
            &[],
            Some(br#"{"name":"gamma","session":"g1"}"#.to_vec()),
        ))
        .unwrap();
    let v: Value = serde_json::from_slice(&joined).unwrap();
    let member_key = v["memberKey"].as_str().unwrap().to_string();
    let member_secret = v["memberSecret"].as_str().unwrap().to_string();
    let hello = open_stream(&rig, &format!("/v1/attach?member={member_secret}"));
    assert_eq!(hello["type"], "hello", "{hello}");
    let hello = open_stream(
        &rig,
        &format!("/v1/attach?session={}&mod=0.4.0", common::SESS_A),
    );
    let attach = hello["attachSecret"]
        .as_str()
        .expect("attach secret")
        .to_string();
    // The member's card is listed while its stream is open.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !rig.core.local_agents().iter().any(|c| c.key == member_key) {
        assert!(
            std::time::Instant::now() < deadline,
            "the member never listed"
        );
        std::thread::sleep(Duration::from_millis(20));
    }

    let person_key = format!("user:{}", rig.core.device_id());
    let co = rig.core.coord();
    let assigned = co
        .create_task(
            &person_key,
            coord::NewTask {
                title: "wire the board routes".into(),
                objective: "loopback reads".into(),
                repo_key: BOARD_REPO.into(),
                ..Default::default()
            },
        )
        .unwrap();
    let (assigned, _) = co.assign(&person_key, &assigned.id, &rig.key_a).unwrap();
    let (comment, _) = co
        .add_comment(
            &person_key,
            &assigned.id,
            "use the pineapple fixture",
            false,
        )
        .unwrap();
    let blocked = co
        .create_task(
            &person_key,
            coord::NewTask {
                title: "port 5173 is taken".into(),
                repo_key: BOARD_REPO.into(),
                block_reason: "dev server left running".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let open = co
        .create_task(
            &person_key,
            coord::NewTask {
                title: "tidy the docs".into(),
                repo_key: BOARD_REPO.into(),
                ..Default::default()
            },
        )
        .unwrap();

    let person = inbox::Sender::new(&person_key, "Stephen", "user");
    let ib = rig.core.inbox();
    let send = |to: &str, text: &str| {
        ib.send_with(
            &person,
            &inbox::Address::new(inbox::addr::AGENT, to),
            text,
            inbox::SendOptions::default(),
        )
        .unwrap()
        .0
        .id
    };
    let msg_to_member = send(&member_key, "secret mango for gamma only");
    let msg_to_claude = send(&rig.key_a, "kiwi note for alpha");
    Fixture {
        person: rig.server.person_secret(),
        rig,
        attach,
        member_key,
        member_secret,
        assigned,
        blocked,
        open,
        comment,
        msg_to_member,
        msg_to_claude,
    }
}

/// Who asks: Go's `Identity`.
#[derive(Clone, Copy, Debug)]
enum Who {
    Person,
    Member,
    Claude,
    /// The token alone (reads as an agent).
    Token,
}

impl Fixture {
    fn get(&self, who: Who, path: &str) -> Response {
        let hs: Vec<(&str, &str)> = match who {
            Who::Person => vec![(irori_api::HEADER_PERSON, self.person.as_str())],
            Who::Member => vec![(
                irori_api::attach::HEADER_MEMBER,
                self.member_secret.as_str(),
            )],
            Who::Claude => vec![(irori_api::attach::HEADER_ATTACH, self.attach.as_str())],
            Who::Token => vec![],
        };
        self.rig
            .rt
            .block_on(self.rig.client.raw("GET", path, &hs, None))
            .unwrap()
    }

    fn json(&self, who: Who, path: &str) -> Value {
        let r = self.get(who, path);
        assert_eq!(
            r.status,
            200,
            "{path}: {}",
            String::from_utf8_lossy(&r.body)
        );
        serde_json::from_slice(&r.body).unwrap()
    }
}

fn q(s: &str) -> String {
    irori_api::client::query_escape(s)
}

fn refs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|h| h["ref"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn msg(id: &str) -> String {
    search::make_ref(search::Kind::Msg, id)
}

fn task(t: &coord::Task) -> String {
    search::make_ref(search::Kind::Task, &t.id)
}

#[test]
fn board_search_viewer_scoping() {
    let f = new_fixture();
    let mango = msg(&f.msg_to_member);
    for (name, who, want) in [
        (
            "person sees every message",
            Who::Person,
            vec![mango.clone()],
        ),
        ("recipient", Who::Member, vec![mango.clone()]),
        ("another agent", Who::Claude, vec![]),
        ("the token alone", Who::Token, vec![]),
    ] {
        let res = f.json(who, "/v1/search?q=mango");
        assert!(
            refs(&res["hits"]) == want && res["more"] == false,
            "{name}: hits = {res}, want {want:?}"
        );
    }
}

#[test]
fn board_search_filters_and_paging() {
    let f = new_fixture();
    let res = f.json(Who::Person, "/v1/search?q=pineapple");
    assert_eq!(
        refs(&res["hits"]),
        [search::make_ref(search::Kind::Comment, &f.comment.id)],
        "comment search"
    );

    // kinds: repeated and comma-separated both narrow.
    for p in [
        "/v1/search?kinds=task",
        "/v1/search?kinds=task,comment&kinds=task",
        "/v1/search?kinds=TASK",
    ] {
        let r = f.json(Who::Token, p);
        let hits = r["hits"].as_array().unwrap();
        for h in hits {
            let k = h["kind"].as_str().unwrap();
            assert!(
                k == "task" || (p.contains("comment") && k == "comment"),
                "{p}: hit {} of kind {k}",
                h["ref"]
            );
        }
        assert!(
            hits.len() >= 3,
            "{p}: {} hits, want the 3 tasks",
            hits.len()
        );
    }

    // Paging: limit 2 over the three tasks, then the rest at next.
    let p1 = f.json(Who::Person, "/v1/search?kinds=task&limit=2");
    assert!(
        p1["hits"].as_array().unwrap().len() == 2 && p1["more"] == true && p1["next"] == 2,
        "page 1 = {p1}"
    );
    let p2 = f.json(Who::Person, "/v1/search?kinds=task&limit=2&offset=2");
    assert!(
        p2["hits"].as_array().unwrap().len() == 1
            && p2["more"] == false
            && p2.get("next").is_none(),
        "page 2 = {p2}"
    );
    let mut all = refs(&p1["hits"]);
    all.extend(refs(&p2["hits"]));
    all.sort();
    let mut want = vec![task(&f.assigned), task(&f.blocked), task(&f.open)];
    want.sort();
    assert_eq!(all, want, "pages");

    // repo and before filter.
    let r = f.json(Who::Person, "/v1/search?kinds=task&repo=elsewhere");
    assert!(r["hits"].as_array().unwrap().is_empty(), "other repo = {r}");
    let r = f.json(
        Who::Person,
        &format!("/v1/search?repo={}&kinds=task", q(BOARD_REPO)),
    );
    assert_eq!(r["hits"].as_array().unwrap().len(), 3, "board repo = {r}");
    let hour_ago = gocompat::time::GoTime::from_system(
        std::time::SystemTime::now() - Duration::from_secs(3600),
    );
    let r = f.json(
        Who::Person,
        &format!("/v1/search?before={}", q(&hour_ago.format_rfc3339())),
    );
    assert!(
        r["hits"].as_array().unwrap().is_empty(),
        "before an hour ago = {r}"
    );
    // limit is capped, not refused.
    f.json(Who::Token, "/v1/search?limit=100000");
}

#[test]
fn board_show_viewer_scoping() {
    let f = new_fixture();
    let (mango, kiwi) = (msg(&f.msg_to_member), msg(&f.msg_to_claude));
    let assigned = task(&f.assigned);
    for (name, who, rf, want) in [
        ("person reads gamma's", Who::Person, mango.as_str(), 200),
        ("gamma reads its own", Who::Member, &mango, 200),
        ("alpha can't read gamma's", Who::Claude, &mango, 404),
        ("alpha reads its own", Who::Claude, &kiwi, 200),
        ("gamma can't read alpha's", Who::Member, &kiwi, 404),
        (
            "the token alone can't read gamma's",
            Who::Token,
            &mango,
            404,
        ),
        ("tasks have no readers", Who::Member, &assigned, 200),
        ("unknown", Who::Person, "msg:nope", 404),
        ("bad ref", Who::Person, "nope", 400),
    ] {
        let r = f.get(who, &format!("/v1/show?ref={}", q(rf)));
        assert_eq!(
            r.status,
            want,
            "{name}: {}",
            String::from_utf8_lossy(&r.body)
        );
        if want == 200 {
            let sh: Value = serde_json::from_slice(&r.body).unwrap();
            assert!(
                sh["ref"] == rf && sh["text"].as_str().is_some_and(|t| !t.is_empty()),
                "{name}: shown = {sh}"
            );
        }
    }
    // The secret never leaks in a 404's body.
    let r = f.get(Who::Claude, &format!("/v1/show?ref={}", q(&mango)));
    let body = String::from_utf8_lossy(&r.body);
    assert!(
        r.status == 404 && !body.contains("mango"),
        "hidden show: {} {body}",
        r.status
    );
}

#[test]
fn board_show_paging() {
    let f = new_fixture();
    let rf = q(&task(&f.assigned));
    let full = f.json(Who::Person, &format!("/v1/show?ref={rf}"));
    let p1 = f.json(Who::Person, &format!("/v1/show?ref={rf}&limit=10"));
    let t1 = p1["text"].as_str().unwrap();
    let next = p1["next"].as_i64().unwrap_or(0);
    assert!(
        p1["more"] == true && next != 0 && t1.len() <= 10,
        "page 1 = {p1}"
    );
    let p2 = f.json(
        Who::Person,
        &format!("/v1/show?ref={rf}&offset={next}&limit={}", search::MAX_SHOW),
    );
    let t2 = p2["text"].as_str().unwrap();
    assert!(
        format!("{t1}{t2}") == full["text"].as_str().unwrap() && p2["more"] == false,
        "pages don't rebuild the text: {t1:?} + {t2:?} vs {}",
        full["text"]
    );
}

#[test]
fn board_overview() {
    let f = new_fixture();
    let key_a = f.rig.key_a.clone();
    let repo_a = f
        .rig
        .core
        .local_agents()
        .into_iter()
        .find(|c| c.key == key_a)
        .unwrap()
        .repo_key;
    let n_agents = f.rig.core.local_agents().len();

    // The attached agent's own overview, with or without agent=.
    for agent in ["".to_string(), key_a.clone()] {
        let ov = f.json(Who::Claude, &format!("/v1/overview?agent={}", q(&agent)));
        assert!(
            ov["for"] == key_a.as_str()
                && ov["repoKey"].as_str().unwrap_or("") == repo_a
                && ov["inboxOpen"] == 1,
            "overview = {ov}"
        );
        assert_eq!(refs(&ov["inbox"]), [msg(&f.msg_to_claude)], "inbox lines");
        assert_eq!(refs(&ov["blockers"]), [task(&f.blocked)], "blockers");
        // Its own task first.
        let tasks = refs(&ov["tasks"]);
        assert!(
            tasks.len() == 2 && tasks[0] == task(&f.assigned),
            "tasks = {tasks:?}"
        );
        let agents = ov["agents"].as_array().unwrap();
        assert!(
            agents.len() == n_agents
                && agents[0]["key"] == key_a.as_str()
                && ov["hint"].as_str().is_some_and(|h| !h.is_empty()),
            "agents = {agents:?} hint {}",
            ov["hint"]
        );
    }

    // The person picks the agent; gamma's inbox is gamma's.
    let ov = f.json(
        Who::Person,
        &format!("/v1/overview?agent={}", q(&f.member_key)),
    );
    assert!(
        ov["for"] == f.member_key.as_str() && refs(&ov["inbox"]) == [msg(&f.msg_to_member)],
        "person's view of gamma = {ov}"
    );

    // An agent can't read another's overview (it carries that inbox).
    let r = f.get(
        Who::Claude,
        &format!("/v1/overview?agent={}", q(&f.member_key)),
    );
    assert_eq!(r.status, 403, "alpha reading gamma's overview");
    let r = f.get(Who::Token, &format!("/v1/overview?agent={}", q(&key_a)));
    assert_eq!(r.status, 403, "the token alone reading alpha's overview");
    let r = f.get(Who::Person, "/v1/overview");
    assert_eq!(r.status, 400, "person without agent");
}

/// The person's board is the window's: For is "user:<device>", so a task
/// the person holds and one blocked on them are in their NeedsYou. A
/// "user:" key that names no device certified to the person is made up,
/// and refused (Go TestBoardPersonNeedsYou).
#[test]
fn board_person_needs_you() {
    let f = new_fixture();
    let co = f.rig.core.coord();
    let (key_a, person_key) = (
        f.rig.key_a.clone(),
        format!("user:{}", f.rig.core.device_id()),
    );
    let mine = co
        .create_task(
            &key_a,
            coord::NewTask {
                title: "review this".into(),
                repo_key: BOARD_REPO.into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.assign(&key_a, &mine.id, &person_key).unwrap();
    let on_me = co
        .create_task(
            &key_a,
            coord::NewTask {
                title: "need a key".into(),
                repo_key: BOARD_REPO.into(),
                block_reason: "an API key".into(),
                blocked_by: Some(coord::BlockedBy {
                    kind: coord::BLOCKED_ON_PERSON.into(),
                    reference: person_key.clone(),
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let err = co
        .create_task(
            &key_a,
            coord::NewTask {
                title: "ask a stranger".into(),
                repo_key: BOARD_REPO.into(),
                block_reason: "their call".into(),
                blocked_by: Some(coord::BlockedBy {
                    kind: coord::BLOCKED_ON_PERSON.into(),
                    reference: "user:dev2".into(),
                }),
                ..Default::default()
            },
        )
        .expect_err("blocked on a made-up user: key");
    assert!(
        err.is(coord::ErrorKind::Invalid),
        "err = {err}, want Invalid"
    );
    let v = f.json(Who::Person, &format!("/v1/board?repo={}", q(BOARD_REPO)));
    let needs: Vec<&str> = v["needsYou"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert!(
        needs.contains(&mine.id.as_str()) && needs.contains(&on_me.id.as_str()),
        "person's NeedsYou = {needs:?}, want {} and {}",
        mine.id,
        on_me.id
    );
}
