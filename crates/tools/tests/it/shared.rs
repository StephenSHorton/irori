//! Shared boards in the tools (v0.5 §7.1, §7.3): board on board_create, the
//! autonomy presets, share requests as notifications, the Visible-to line,
//! scrub reasons and replies to bridged messages going on the card. New in
//! stage 2: no Go original. The board is a real board-log coord behind a
//! fake router; the people are made up.

use crate::common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::*;
use coord::{Coord, NewTask, Notice, Task};
use tools::{BoardInfo, SharedBoards, TaskRouter};

const USER: &str = "user:dev1";
const ME: &str = "person:pStephen";

/// One shared board, "MLG farming" (mlg), with Stephen and Blake.
struct Fake {
    info: BoardInfo,
    st: Arc<store::Store>,
    co: Arc<Coord>,
    private: Arc<Coord>,
    cards: Mutex<BTreeMap<String, String>>,
    shared: Mutex<Vec<(String, String)>>,
    withdrawn: Mutex<Vec<String>>,
    // Last: fields drop in order, and Windows can't delete an open file.
    _dir: tempfile::TempDir,
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
        // Blake is on the board, with three agent cards (card #186).
        st.append(
            ME,
            store::kinds::BOARD_MEMBER_ADDED,
            &json!({"personId": "pBlake", "name": "Blake", "role": "member"}),
        )
        .unwrap();
        for (card, name) in [("cb1", "mlg-lead"), ("cb2", "twin"), ("cb3", "twin")] {
            st.append(
                &format!("agent:{card}"),
                store::kinds::AGENT_CARD,
                &json!({"cardId": card, "name": name, "personId": "pBlake", "status": "active"}),
            )
            .unwrap();
        }
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
                    // Whose a board key is (card #264: who decides).
                    key_person: Some(Arc::new(|k: &str| match k {
                        ME => "pStephen".into(),
                        "person:pBlake" | "agent:cb1" | "agent:cb2" | "agent:cb3" => {
                            "pBlake".into()
                        }
                        k if k.starts_with("agent:c") => "pStephen".into(),
                        _ => String::new(),
                    })),
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
                ..Default::default()
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
        // Keys already in board form are kept, as the Core's board_key does.
        if k.starts_with("agent:") || k.starts_with("person:") {
            return k.into();
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
    fn private_key(&self, key: &str) -> String {
        self.back(key)
    }
    fn task_ref(&self, r: &str) -> Result<String, coord::Error> {
        if r.starts_with("mlg#") {
            return self.co.task_ref(r);
        }
        self.private.task_ref(r).or_else(|_| self.co.task_ref(r))
    }
    fn comment_coord(&self, id: &str) -> Option<Arc<Coord>> {
        self.co.get_comment(id).map(|_| self.co.clone())
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
    fn share(&self, task_id: &str, board_id: &str, _: &str, by: &str) -> Result<Task, String> {
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
    /// Blake and his agents (card #186); two of them share a name.
    fn board_mentionables(&self, _board_id: &str) -> Vec<coord::roster::AgentCard> {
        let card = |key: &str, name: &str| coord::roster::AgentCard {
            key: key.into(),
            name: name.into(),
            status: "idle".into(),
            ..Default::default()
        };
        vec![
            card("person:pBlake", "Blake"),
            card("agent:cb1", "mlg-lead"),
            card("agent:cb2", "twin"),
            card("agent:cb3", "twin"),
        ]
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

/// talk_post on a space's card is a comment that replicates to its members,
/// so it passes the same fence as board_comment (card #558).
#[test]
fn talk_post_on_a_space_card_keeps_the_fence() {
    let r = rig();
    r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Speed", "board": "mlg"}),
    );
    let before = cs_text(&r, "mlg#1").len();
    r.h.failsj(
        &call_a(),
        "talk_post",
        json!({"conv": "mlg#1", "text": "notes in /Users/fixture/x"}),
        &["nothing was written", "home directory"],
    );
    r.h.failsj(
        &call_a(),
        "talk_post",
        json!({"conv": "mlg#1", "text": "key sk-ant-api03-abcdefghijklmnopqrstuvwxyz012345"}),
        &["nothing was written", "secret"],
    );
    assert_eq!(cs_text(&r, "mlg#1").len(), before);
    let out = r.h.okj(
        &call_a(),
        "talk_post",
        json!({"conv": "mlg#1", "text": "profiled: the decoder is the slow part"}),
    );
    assert!(out.ends_with("\nVisible to: Stephen, Blake"), "{out}");
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
        &["Ask me", "Share to a space"],
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

/// Card #570: a card made Not yet, or after another, on a shared board
/// says so, and isn't offered to be taken now.
#[test]
fn a_card_made_not_yet_on_a_board_isnt_taken_now() {
    let r = rig();
    r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Tune the loader", "board": "mlg"}),
    );
    let out = r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Ship the loader", "board": "mlg", "after": ["mlg#1"]}),
    );
    has(&out, &["Created mlg#2", "It comes after mlg#1"]);
    has_not(&out, &["Take it up with board_join"]);
    let out = r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Voice calls", "board": "mlg", "assign_to": KEY_B,
               "later": {"why": "after the move"}}),
    );
    has(
        &out,
        &[
            "Created mlg#3",
            "It is Not yet: it comes back when the person says.",
            "It isn't assigned",
        ],
    );
    has_not(&out, &["Take it up with board_join", "Assigning it failed"]);
    assert!(out.ends_with("Visible to: Stephen, Blake"), "{out}");
    let t = r
        .fake
        .co
        .list_tasks(&coord::TaskFilter::default())
        .into_iter()
        .find(|t| t.title == "Voice calls")
        .unwrap();
    assert!(t.parked().is_some() && t.holders.is_empty(), "{t:?}");
}

#[test]
fn the_board_tool_lists_shared_boards() {
    let r = rig();
    let out = r.h.okj(&call_a(), "board", json!({}));
    has(
        &out,
        &[
            "Spaces you may post on",
            "- \"MLG farming\": Stephen, Blake; your autonomy there: trusted\n  boards: mlg \"MLG farming\"",
        ],
    );
    r.h.failsj(
        &call_a(),
        "board_create",
        json!({"title": "x", "board": "nope"}),
        &["no board \"nope\" here", "Shared spaces': mlg."],
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
        &["isn't in a space with that person"],
    );
}

#[test]
fn a_message_mention_watches_a_shared_card() {
    let r = rig();
    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Loader", "board": "mlg"}),
    ));
    let out = r.h.okj(
        &call_a(),
        "send",
        json!({"to": "beta", "text": "@gamma fyi", "task_id": id}),
    );
    has(&out, &["Mentioned: gamma (woke)."]);
    has_not(&out, &["don't watch"]);
    let gamma = r.fake.key(KEY_C);
    assert!(
        r.fake.co.get_task(&id).unwrap().watchers.contains(&gamma),
        "{out}"
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

/// Card #186: on a shared card an agent's @name also reaches the board's
/// other people and their agents, by their board keys; ambiguous and
/// unknown names behave as on the private board.
#[test]
fn mentions_on_a_shared_card_name_other_people_and_their_agents() {
    let r = rig();
    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Loader", "board": "mlg"}),
    ));
    let out = r.h.okj(
        &call_a(),
        "board_comment",
        json!({"task_id": id, "text": "@blake can @mlg-lead take the loader? @gamma too"}),
    );
    has(
        &out,
        &["Mentioned: blake (in the space), mlg-lead (in the space), gamma (woke)."],
    );
    has_not(&out, &["Not delivered"]);
    // The comment row names them by board key. (Board rows keep their
    // mentions once #177 lands; until then the fold drops them.)
    let cm = r.fake.co.comments(&id).unwrap().pop().unwrap();
    let stored: Vec<&str> = cm.mentions.iter().map(|m| m.key.as_str()).collect();
    assert!(
        stored
            .iter()
            .all(|k| k.starts_with("agent:") || k.starts_with("person:")),
        "{stored:?}"
    );
    if !stored.is_empty() {
        for k in ["person:pBlake", "agent:cb1"] {
            assert!(stored.contains(&k), "{k} in {stored:?}");
        }
    }
    // Their own devices tell them: nothing lands in this device's inbox.
    let stray: Vec<String> =
        r.h.b
            .ib
            .list(200)
            .into_iter()
            .map(|d| d.to)
            .filter(|t| t.starts_with("person:") || t.starts_with("agent:cb"))
            .collect();
    assert!(stray.is_empty(), "{stray:?}");
    // The mention arg takes a board key or a name.
    let out = r.h.okj(
        &call_a(),
        "board_comment",
        json!({"task_id": id, "text": "see above", "mention": ["agent:cb1", "Blake"]}),
    );
    has(&out, &["mlg-lead (in the space)", "Blake (in the space)"]);
    // Ambiguous and unknown, as on the private board.
    let out = r.h.okj(
        &call_a(),
        "board_comment",
        json!({"task_id": id, "text": "@twin and @nobody", "mention": ["person:pNobody"]}),
    );
    has(
        &out,
        &[
            "twin is ambiguous: mention one by key (agent:cb2; agent:cb3)",
            "no agent has key person:pNobody",
        ],
    );
    // On a private card the board's people are nobody.
    let pid = created_id(&r.h.okj(&call_a(), "board_create", json!({"title": "Mine"})));
    let out = r.h.okj(
        &call_a(),
        "board_comment",
        json!({"task_id": pid, "text": "@Blake?"}),
    );
    has_not(&out, &["Blake (in the space)"]);
}

/// Card #189: show opens a shared board's card and its comments, which live
/// in the board's log (capture indexes only the private one).
/// Card #573: on a shared card a member can be named: the card is given to
/// their `person:` key, and a name that is no member's or agent's fails.
#[test]
fn a_shared_card_can_be_given_to_a_member_by_name() {
    let r = rig();
    r.autonomy("trusted");
    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Loader", "board": "mlg", "assign_to": "blake"}),
    ));
    let t = r.fake.co.get_task(&id).unwrap();
    assert_eq!(t.assigned, vec!["person:pBlake".to_string()]);
    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Saver", "board": "mlg"}),
    ));
    r.h.okj(
        &call_a(),
        "board_assign",
        json!({"task_id": id, "to": "person:pBlake"}),
    );
    assert_eq!(
        r.fake.co.get_task(&id).unwrap().assigned,
        vec!["person:pBlake".to_string()]
    );
    r.h.failsj(
        &call_a(),
        "board_assign",
        json!({"task_id": id, "to": "nobody-here"}),
        &["nobody-here"],
    );
}

/// Card #570 with #573: a shared card made Not yet with a member named
/// isn't given to them (nobody takes a held card), and making it doesn't
/// fail half way with the card already on the board.
#[test]
fn a_shared_card_made_not_yet_isnt_given() {
    let r = rig();
    r.autonomy("trusted");
    let out = r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Voice calls", "board": "mlg", "assign_to": "blake", "later": {"why": "after the move"}}),
    );
    has(&out, &["It isn't assigned"]);
    let t = r.fake.co.get_task(&created_id(&out)).unwrap();
    assert!(t.parked().is_some() && t.assigned.is_empty() && t.given.is_none());
}

#[test]
fn show_opens_a_shared_card_and_its_comments() {
    let r = rig();
    let (claude_dir, data_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut opts = capture::Options::new(
        &canon(claude_dir.path()),
        &canon(data_dir.path()),
        "dev1",
        Arc::new(capture::NoScrub),
        Arc::new(capture::MemBlobs::default()),
    );
    let clk = r.h.clk.clone();
    opts.now = Arc::new(move || clk.now());
    let cp = capture::Capture::new(r.h.b.st.clone(), None, opts).unwrap();
    cp.add_source(r.h.b.co.clone());
    assert!(r.h.b.cp.set(cp).is_ok());

    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Dolphin rig", "objective": "boot the disc", "board": "mlg"}),
    ));
    r.h.okj(
        &call_a(),
        "board_comment",
        json!({"task_id": id, "text": "fixtures are in the handoff notes"}),
    );
    for rf in ["mlg#1".to_string(), format!("task:{id}")] {
        let out = r.h.okj(&call_b(), "show", json!({"ref": rf}));
        has(
            &out,
            &[
                "Dolphin rig",
                "boot the disc",
                "fixtures are in the handoff notes",
            ],
        );
    }
    let cm = r.fake.co.comments(&id).unwrap()[0].id.clone();
    let out =
        r.h.okj(&call_b(), "show", json!({"ref": format!("comment:{cm}")}));
    has(&out, &["fixtures are in the handoff notes", "Dolphin rig"]);
    // A private card still opens from the private log.
    let p = r.h.new_card(&call_a(), json!({"title": "mine only"}));
    has(
        &r.h.okj(&call_a(), "show", json!({"ref": format!("task:{p}")})),
        &["mine only"],
    );
}

/// Card #264: board_block's for names who decides a shared card's question
/// (a member's name or person: key); without it the question is for the
/// raiser's own person. A private card takes no for.
#[test]
fn a_question_is_for_whoever_for_names() {
    let r = rig();
    let id = created_id(&r.h.okj(
        &call_a(),
        "board_create",
        json!({"title": "Loader", "board": "mlg"}),
    ));
    let on = |bid: &str| {
        r.fake
            .co
            .get_task(&id)
            .unwrap()
            .blockers
            .into_iter()
            .find(|b| b.id == bid)
            .and_then(|b| b.on)
            .map(|o| o.reference)
            .unwrap_or_default()
    };
    let out = r.h.okj(
        &call_a(),
        "board_block",
        json!({"task_id": id, "question": "Rename the loader?", "for": "blake"}),
    );
    has(&out, &["a question for Blake", "anyone can still answer"]);
    assert_eq!(on("b1"), "person:pBlake");
    r.h.okj(
        &call_a(),
        "board_block",
        json!({"task_id": id, "question": "Ship it?"}),
    );
    assert_eq!(on("b2"), ME, "default: the raiser's own person");
    r.h.failsj(
        &call_a(),
        "board_block",
        json!({"task_id": id, "question": "Who?", "for": "Nobody"}),
        &["nobody on MLG farming is named \"Nobody\"", "\"Blake\""],
    );
    r.h.failsj(
        &call_a(),
        "board_block",
        json!({"task_id": id, "question": "Wait?", "for": "Blake", "on": "external CI"}),
        &["for names the person who decides"],
    );
    let pid = created_id(&r.h.okj(&call_a(), "board_create", json!({"title": "Mine"})));
    r.h.failsj(
        &call_a(),
        "board_block",
        json!({"task_id": pid, "question": "Mine?", "for": "Blake"}),
        &["leave for out"],
    );
}

/// Card #574: a wait on a shared card lives in the waiter's own log, so
/// nothing of it reaches the board, and it lists and cancels like any wait.
#[test]
fn a_wait_on_a_shared_card_stays_off_the_board() {
    let r = rig();
    let a = call_a();
    let t = r
        .fake
        .co
        .create_task(
            ME,
            NewTask {
                title: "Ship it".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    let rows = || r.fake.st.list(&store::Filter::default()).unwrap().len();
    let before = rows();
    let out = r.h.okj(
        &a,
        "wait",
        json!({"task_id": t.id, "until": ["done", "unblocked"], "note": "the API I build on"}),
    );
    has(
        &out,
        &[
            "woken once when mlg#1 is done or unblocked",
            "Only you see this wait",
        ],
    );
    assert!(!out.contains("Visible to"), "{out}");
    assert_eq!(
        rows(),
        before,
        "nothing reached the board, not even the agent"
    );
    let ws = r.h.b.co.waits("");
    assert_eq!(ws.len(), 1, "{ws:?}");
    assert_eq!(ws[0].board, "b0");
    assert_eq!(ws[0].title, "Ship it");
    assert_eq!(ws[0].from_state, t.state);
    assert!(r.fake.co.waits("").is_empty(), "nothing in the board's log");
    has(
        &r.h.okj(&a, "waits", json!({})),
        &["mlg#1 until done or unblocked"],
    );
    has(
        &r.h.okj(&a, "wait_cancel", json!({"id": ws[0].id})),
        &["Canceled your wait on mlg#1"],
    );
}

#[test]
fn your_own_shared_card_is_no_wait() {
    let r = rig();
    let a = call_a();
    let t = r
        .fake
        .co
        .create_task(
            ME,
            NewTask {
                title: "Mine".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    r.h.okj(&a, "board_join", json!({"task_id": t.id}));
    r.h.failsj(&a, "wait", json!({"task_id": t.id}), &["your own card"]);
}
