//! Shared boards in the tools (v0.5 §7.1, §7.3): board on board_create, the
//! autonomy presets, share requests as notifications, the Visible-to line,
//! scrub reasons and replies to bridged messages going on the card. New in
//! stage 2: no Go original. The board is a real board-log coord behind a
//! fake router; the people are made up.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::*;
use coord::{Coord, NewTask, Notice, Task};
use tools::{BoardInfo, SharedBoards, TaskRouter};

const USER: &str = "user:dev1";
const ME: &str = "person:pStephen";

/// One shared board, "MLG farming" (mlg), with Stephen and Blake.
struct Fake {
    _dir: tempfile::TempDir,
    info: BoardInfo,
    st: Arc<store::Store>,
    co: Arc<Coord>,
    private: Arc<Coord>,
    cards: Mutex<BTreeMap<String, String>>,
    shared: Mutex<Vec<(String, String)>>,
    withdrawn: Mutex<Vec<String>>,
}

impl Fake {
    fn new(private: Arc<Coord>, clk: &Clock) -> Arc<Fake> {
        let dir = tempfile::tempdir().unwrap();
        let c = clk.clone();
        let hc = hlc::Clock::with_now(Box::new(move || c.now().to_system()));
        let st = Arc::new(
            store::Store::open_with(
                dir.path().join("b.db"),
                "dev1",
                Arc::new(hc),
                store::OpenOptions {
                    board: true,
                    ..store::OpenOptions::default()
                },
            )
            .unwrap(),
        );
        st.append(
            ME,
            store::kinds::BOARD_CREATED,
            &json!({"boardId": "b0", "creator": {"personId": "pStephen"}, "name": "MLG farming", "tag": "mlg"}),
        )
        .unwrap();
        let c = clk.clone();
        let co = Arc::new(
            Coord::new(
                st.clone(),
                None,
                coord::Options {
                    now: Some(Arc::new(move || c.now())),
                    board: true,
                    person: ME.into(),
                    is_owner: Some(Arc::new(|k: &str| k == ME)),
                    tag: Some(Arc::new(|| "mlg".to_string())),
                    ..coord::Options::default()
                },
            )
            .unwrap(),
        );
        Arc::new(Fake {
            _dir: dir,
            info: BoardInfo {
                board_id: "b0".into(),
                tag: "mlg".into(),
                name: "MLG farming".into(),
                people: vec!["Stephen".into(), "Blake".into()],
            },
            st,
            co,
            private,
            cards: Mutex::default(),
            shared: Mutex::default(),
            withdrawn: Mutex::default(),
        })
    }

    /// A key as the board knows it, minting the agent's card.
    fn key(&self, k: &str) -> String {
        if k.is_empty() {
            return String::new();
        }
        if k.starts_with("user:") {
            return ME.into();
        }
        let mut m = self.cards.lock().unwrap();
        if let Some(c) = m.get(k) {
            return c.clone();
        }
        let card = format!("c{}", m.len() + 1);
        let bk = format!("agent:{card}");
        self.st
            .append(
                &bk,
                store::kinds::AGENT_CARD,
                &json!({"cardId": card, "personId": "pStephen", "status": "active"}),
            )
            .unwrap();
        m.insert(k.into(), bk.clone());
        bk
    }

    fn back(&self, k: &str) -> String {
        if k == ME {
            return USER.into();
        }
        let m = self.cards.lock().unwrap();
        m.iter()
            .find(|(_, v)| *v == k)
            .map_or_else(|| k.to_string(), |(a, _)| a.clone())
    }

    fn keys(&self, keys: &[String]) -> Vec<String> {
        keys.iter().map(|k| self.key(k)).collect()
    }
}

impl TaskRouter for Fake {
    fn task_coord(
        &self,
        id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        if self.co.get_task(id).is_some() {
            Ok((self.co.clone(), self.keys(keys)))
        } else {
            Ok((self.private.clone(), keys.to_vec()))
        }
    }
    fn private_notice(&self, mut n: Notice) -> Notice {
        n.to = n.to.iter().map(|k| self.back(k)).collect();
        n.by = self.back(&n.by);
        n
    }
    fn task_ref(&self, r: &str) -> Result<String, coord::Error> {
        if r.starts_with("mlg#") {
            return self.co.task_ref(r);
        }
        self.private.task_ref(r).or_else(|_| self.co.task_ref(r))
    }
}

impl SharedBoards for Fake {
    fn boards(&self) -> Vec<BoardInfo> {
        vec![self.info.clone()]
    }
    fn board_of(&self, task_id: &str) -> Option<BoardInfo> {
        self.co.get_task(task_id).map(|_| self.info.clone())
    }
    fn board_coord(
        &self,
        _board_id: &str,
        keys: &[String],
    ) -> Result<(Arc<Coord>, Vec<String>), coord::Error> {
        Ok((self.co.clone(), self.keys(keys)))
    }
    fn share(&self, task_id: &str, board_id: &str, by: &str) -> Result<Task, String> {
        let t = self.private.get_task(task_id).ok_or("no such card")?;
        let st = self
            .co
            .create_task(
                &self.key(by),
                NewTask {
                    title: t.title.clone(),
                    ..NewTask::default()
                },
            )
            .map_err(|e| e.to_string())?;
        self.shared
            .lock()
            .unwrap()
            .push((task_id.into(), board_id.into()));
        Ok(st)
    }
    fn withdraw(&self, task_id: &str) -> Result<(), String> {
        self.withdrawn.lock().unwrap().push(task_id.into());
        Ok(())
    }
}

struct Rig {
    h: Harness,
    fake: Arc<Fake>,
    book: Arc<coord::approvals::Book>,
}

fn rig() -> Rig {
    type Parts = (Arc<Fake>, Arc<coord::approvals::Book>);
    let cell: Arc<Mutex<Option<Parts>>> = Arc::default();
    let c2 = cell.clone();
    let h = new_harness_setup(None, move |b, clk| {
        let fake = Fake::new(b.co.clone(), clk);
        let _ = b.router.set(fake.clone());
        let _ = b.shared.set(fake.clone());
        let c = clk.clone();
        let mut o = coord::approvals::Options::new(b.st.clone(), "dev1");
        o.now = Some(Arc::new(move || c.now()));
        let book = coord::approvals::Book::new(o).unwrap();
        let _ = b.book.set(book.clone());
        *c2.lock().unwrap() = Some((fake, book));
    });
    let (fake, book) = cell.lock().unwrap().take().unwrap();
    Rig { h, fake, book }
}

impl Rig {
    fn autonomy(&self, preset: &str) {
        self.h
            .b
            .ib
            .set_autonomy(
                inbox::AutonomySetting {
                    board_id: "b0".into(),
                    preset: preset.into(),
                },
                USER,
            )
            .unwrap();
    }

    /// The one open request and its id.
    fn request(&self) -> coord::approvals::Ask {
        let open = self.book.open();
        assert_eq!(open.len(), 1, "{open:?}");
        open[0].clone()
    }

    fn answer(&self, choice: &str) {
        let a = self.request();
        self.book.answer(&a.id, choice, USER).unwrap();
    }

    /// The newest notice alpha has from Irori (notices lists newest first).
    fn told_a(&self) -> String {
        self.h
            .notices(KEY_A)
            .first()
            .map(|d| d.message.text.clone())
            .unwrap_or_default()
    }

    fn board_titles(&self) -> Vec<String> {
        self.fake
            .co
            .list_tasks(&coord::TaskFilter::default())
            .into_iter()
            .map(|t| t.title)
            .collect()
    }
}

#[test]
fn trusted_agents_create_on_the_board() {
    let r = rig();
    let out = r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Tune the loader", "board": "mlg", "steps": [{"text": "profile"}]}),
    );
    has(&out, &["Created mlg#1 (task:", "on MLG farming (mlg)."]);
    assert!(out.ends_with("Visible to: Stephen, Blake"), "{out}");
    assert_eq!(r.board_titles(), ["Tune the loader"]);
    assert!(
        r.h.b
            .co
            .list_tasks(&coord::TaskFilter::default())
            .is_empty()
    );
    // Its writer on the board is alpha's card, never alpha's session key.
    let t = &r.fake.co.list_tasks(&coord::TaskFilter::default())[0];
    assert_eq!(t.created_by, "agent:c1");
    // Without board the card is private, with no Visible-to line.
    let out =
        r.h.okj(&call_a(), "board_create", json!({"title": "Private"}));
    has_not(&out, &["Visible to"]);
    assert_eq!(r.h.b.co.list_tasks(&coord::TaskFilter::default()).len(), 1);
}

#[test]
fn scrub_blocks_a_write_and_names_why() {
    let r = rig();
    r.h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "Logs", "objective": "they are in /Users/fixture/proj/out.log", "board": "mlg"}),
        &["nothing was written", "MLG farming (mlg) is shared", "an absolute path or home directory"],
    );
    assert!(r.board_titles().is_empty());
    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Loader", "board": "mlg"}),
    ));
    r.h.failsj(
        &call_a(),
        "board_comment",
        json!({"task_id": id, "text": "use ghp_0123456789abcdefghijklmnopqrstuvwxyz12 for it"}),
        &["nothing was written", "secret"],
    );
    assert!(r.fake.co.comments(&id).unwrap_or_default().is_empty());
    // A clean comment goes on and says who sees it.
    let out = r.h.okj(
        &call_a(),
        "board_comment",
        json!({"task_id": "mlg#1", "text": "profiled: the decoder is the slow part"}),
    );
    assert!(out.ends_with("\nVisible to: Stephen, Blake"), "{out}");
    // A private card's comment is untouched by both.
    let p = r.h.new_card(&call_a(), json!({"title": "mine"}));
    let out = r.h.okj(
        &call_a(),
        "board_comment",
        json!({"task_id": p, "text": "notes in /Users/fixture/x"}),
    );
    has_not(&out, &["Visible to"]);
}

#[test]
fn ask_me_requests_a_card_and_the_answer_makes_it() {
    let r = rig();
    r.autonomy(inbox::autonomy::ASK);
    let out = r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Swap the codec", "board": "mlg"}),
    );
    has(&out, &["Asked your person first", "Ask me"]);
    assert!(r.board_titles().is_empty());
    let a = r.request();
    assert_eq!(a.actions, [tools::CHOICE_CREATE, tools::CHOICE_NO]);
    has(
        &a.text,
        &["MLG farming (mlg)", "Stephen, Blake", "Swap the codec"],
    );
    r.answer(tools::CHOICE_CREATE);
    assert_eq!(r.board_titles(), ["Swap the codec"]);
    has(
        &r.told_a(),
        &[
            "Your person said Create",
            "Created mlg#1",
            "Visible to: Stephen, Blake",
        ],
    );
    // No: nothing is made, and the agent hears it.
    r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Second", "board": "mlg"}),
    );
    r.answer(tools::CHOICE_NO);
    assert_eq!(r.board_titles().len(), 1);
    has(&r.told_a(), &["They chose No", "no card"]);
}

#[test]
fn share_follows_the_autonomy_setting() {
    let r = rig();
    let p = r.h.new_card(&call_a(), json!({"title": "Shareable"}));
    // Trusted (the default): a request with Share, Review and No.
    let out = r.h.okj(
        &call_a(),
        "board_share",
        json!({"task_id": p, "board": "mlg"}),
    );
    has(&out, &["Share, Review and No", "Trusted"]);
    assert!(r.fake.shared.lock().unwrap().is_empty());
    let a = r.request();
    assert_eq!(
        a.actions,
        [tools::CHOICE_SHARE, tools::CHOICE_REVIEW, tools::CHOICE_NO]
    );
    r.answer(tools::CHOICE_REVIEW);
    has(&r.told_a(), &["Review", "Nothing is shared yet"]);
    r.h.okj(
        &call_a(),
        "board_share",
        json!({"task_id": p, "board": "mlg"}),
    );
    r.answer(tools::CHOICE_SHARE);
    assert_eq!(
        *r.fake.shared.lock().unwrap(),
        [(p.clone(), "b0".to_string())]
    );
    has(
        &r.told_a(),
        &[
            "Your person said Share",
            "as mlg#1",
            "Visible to: Stephen, Blake",
        ],
    );
    // A shared card can't be shared again.
    r.h.failsj(
        &call_a(),
        "board_share",
        json!({"task_id": "mlg#1", "board": "mlg"}),
        &["already on"],
    );

    // Ask me: agents may not share.
    r.autonomy(inbox::autonomy::ASK);
    let q = r.h.new_card(&call_a(), json!({"title": "Second"}));
    r.h.failsj(
        &call_a(),
        "board_share",
        json!({"task_id": q, "board": "mlg"}),
        &["Ask me", "Share to board"],
    );

    // Autonomous: shared now, and the person gets Withdraw.
    r.autonomy(inbox::autonomy::AUTONOMOUS);
    let out = r.h.okj(
        &call_a(),
        "board_share",
        json!({"task_id": q, "board": "mlg"}),
    );
    has(
        &out,
        &["Shared", "as mlg#2", "after scrub", "notice with Withdraw"],
    );
    assert!(out.ends_with("Visible to: Stephen, Blake"), "{out}");
    assert_eq!(r.fake.shared.lock().unwrap().len(), 2);
    let a = r.request();
    assert_eq!(a.actions, [tools::CHOICE_WITHDRAW]);
    r.answer(tools::CHOICE_WITHDRAW);
    assert_eq!(r.fake.withdrawn.lock().unwrap().len(), 1);
    has(&r.told_a(), &["withdrew mlg#2"]);
}

#[test]
fn the_board_tool_lists_shared_boards() {
    let r = rig();
    let out = r.h.okj(&call_a(), "board", json!({}));
    has(
        &out,
        &[
            "Shared boards you may post on",
            "- mlg \"MLG farming\": Stephen, Blake; your autonomy there: trusted",
        ],
    );
    r.h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "x", "board": "nope"}),
        &["no shared board \"nope\"", "Yours: mlg."],
    );
}

#[test]
fn a_reply_to_a_bridged_message_goes_on_the_card() {
    let r = rig();
    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Loader", "board": "mlg"}),
    ));
    let ds =
        r.h.b
            .ib
            .bridge(&inbox::BridgeRow {
                from: inbox::BoardFrom {
                    board_id: "b0".into(),
                    tag: "mlg".into(),
                    person_id: "pBlake".into(),
                    card_id: "cBlake".into(),
                    row: "devB/4".into(),
                    kind: inbox::row_kind::COMMENT.into(),
                    wake: false,
                },
                name: "blake's agent mlg-lead".into(),
                to: vec![KEY_A.into()],
                text: "which decoder did you profile?".into(),
                ref_: format!("task:{id}"),
                hop: 1,
                attachments: vec![],
            })
            .unwrap();
    let did = ds[0].id.clone();
    // A secret in the reply is refused before anything is written.
    r.h.failsj(
        &call_a(),
        "reply",
        json!({"delivery_id": did, "text": "the one under ~/proj/dec"}),
        &["nothing was written", "home directory"],
    );
    let out = r.h.okj(
        &call_a(),
        "reply",
        json!({"delivery_id": did, "text": "the AV1 one"}),
    );
    has(
        &out,
        &[
            "Replied on mlg#1",
            "went on the card, not as a private message",
        ],
    );
    assert!(out.ends_with("Visible to: Stephen, Blake"), "{out}");
    let cs = r.fake.co.comments(&id).unwrap();
    assert_eq!(cs.last().unwrap().text, "the AV1 one");
    assert_eq!(r.h.b.ib.get(&did).unwrap().state, inbox::st::ANSWERED);
    // Nothing went to Blake as a private message.
    assert!(r.h.b.ib.list(0).iter().all(|d| !d.to.starts_with("board:")));
    // A send to a board: key goes on the card too (§7.1): the card their
    // last message was about, never a private message to Blake.
    let before = r.h.b.ib.list(0).len();
    let out = r.h.okj(
        &call_a(),
        "send",
        json!({"to": "board:b0/cBlake", "text": "and the VP9 one too"}),
    );
    has(
        &out,
        &[
            "Said on mlg#1",
            "went on the card, not as a private message",
        ],
    );
    assert!(out.ends_with("Visible to: Stephen, Blake"), "{out}");
    assert_eq!(cs_text(&r, &id).last().unwrap(), "and the VP9 one too");
    assert_eq!(r.h.b.ib.list(0).len(), before, "a private message was sent");
    // With task_id it goes on that card; scrub still applies.
    r.h.failsj(
        &call_a(),
        "send",
        json!({"to": "board:b0/cBlake", "text": "see /Users/fixture/x", "task_id": "mlg#1"}),
        &["nothing was written", "home directory"],
    );
    // Someone the caller never heard from, and no card: say which.
    r.h.failsj(
        &call_a(),
        "send",
        json!({"to": "board:b0/cNobody", "text": "hi"}),
        &["give task_id"],
    );
    // A private card isn't on that person's board.
    let p = r.h.new_card(&call_a(), json!({"title": "mine only"}));
    r.h.failsj(
        &call_a(),
        "send",
        json!({"to": "board:b0/cBlake", "text": "hi", "task_id": p}),
        &["isn't on that person's board"],
    );
}

fn cs_text(r: &Rig, id: &str) -> Vec<String> {
    r.fake
        .co
        .comments(id)
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.text)
        .collect()
}
