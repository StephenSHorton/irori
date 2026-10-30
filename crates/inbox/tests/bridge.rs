//! The bridge from shared boards into the inbox and the receive and
//! autonomy settings (v0.5 §7.1-7.3, B2). New in stage 2: no Go original.

mod common;

use common::*;
use inbox::{
    AutonomySetting, BOARD_KIND, BYPASS_MODE, BoardFrom, BridgeRow, Delivery, MAX_HOP,
    ReceiveSetting, autonomy, board_sender_key, hold, mode, receive, receive_decision, row_kind,
    st, trust,
};

const BOARD: &str = "0123456789abcdef0123456789abcdef";
const BLAKE: &str = "pBlake";
const CASEY: &str = "pCasey";
const CARD: &str = "cardBlake1";
/// An agent of this person on another device.
const KEY_D2: &str = "dev2/dddddddd-4444-4444-8444-444444444444";
/// A default-mode (not bypass) session here.
const KEY_N: &str = "dev1/nnnnnnnn-5555-4555-8555-555555555555";

fn from(person: &str, card: &str, kind: &str, wake: bool, row: &str) -> BoardFrom {
    BoardFrom {
        board_id: BOARD.into(),
        tag: "mlg".into(),
        person_id: person.into(),
        card_id: card.into(),
        row: row.into(),
        kind: kind.into(),
        wake,
    }
}

fn row(f: BoardFrom, to: &[&str]) -> BridgeRow {
    BridgeRow {
        name: if f.card_id.is_empty() {
            "blake".into()
        } else {
            "blake's agent mlg-lead".into()
        },
        from: f,
        to: to.iter().map(|s| s.to_string()).collect(),
        text: "can you look at the loader?".into(),
        ref_: "task:01TASK".into(),
        hop: 1,
        attachments: vec![],
    }
}

fn set(preset: &str) -> ReceiveSetting {
    ReceiveSetting {
        preset: preset.into(),
        ..ReceiveSetting::default()
    }
}

#[track_caller]
fn one(h: &Harness, r: &BridgeRow) -> Delivery {
    let mut ds = h.ib.bridge(r).unwrap();
    assert_eq!(ds.len(), 1, "{ds:?}");
    ds.remove(0)
}

fn harness() -> Harness {
    let h = Harness::new();
    h.abc(); // alpha, beta, gamma: bypass sessions
    h.ros.put(&[card(KEY_N, "normal", "repo1", "default")]);
    h
}

#[test]
fn receive_table() {
    use mode::{NEXT_TURN as NT, WAKE as W};
    // (preset, person, card, kind, wake, bypass) -> (mode, hold, asked)
    type Case = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        bool,
        bool,
    );
    let cases: Vec<(Case, (&str, &str, bool))> = vec![
        // Comments, blocker changes: next turn under every setting.
        (
            (receive::OPEN, BLAKE, "", row_kind::COMMENT, false, false),
            (NT, "", false),
        ),
        (
            (receive::ASK, BLAKE, "", row_kind::COMMENT, false, false),
            (NT, "", false),
        ),
        (
            (
                receive::CLOSED,
                BLAKE,
                CARD,
                row_kind::BLOCKER,
                false,
                false,
            ),
            (NT, "", false),
        ),
        // A request to wake: Open wakes, Ask me holds, Closed marks it.
        (
            (receive::OPEN, BLAKE, "", row_kind::COMMENT, true, false),
            (W, "", false),
        ),
        (
            (receive::ASK, BLAKE, "", row_kind::COMMENT, true, false),
            (W, hold::MEMBER_WAKE, false),
        ),
        (
            (receive::CLOSED, BLAKE, "", row_kind::COMMENT, true, false),
            (NT, "", true),
        ),
        (
            (receive::OPEN, BLAKE, CARD, row_kind::COMMENT, true, false),
            (W, "", false),
        ),
        (
            (receive::ASK, BLAKE, CARD, row_kind::COMMENT, true, false),
            (W, hold::MEMBER_WAKE, false),
        ),
        // Their agents assigning work: Ask me holds; Open next turn or wake.
        (
            (receive::ASK, BLAKE, CARD, row_kind::ASSIGN, false, false),
            (NT, hold::CROSS_PERSON, false),
        ),
        (
            (receive::OPEN, BLAKE, CARD, row_kind::ASSIGN, false, false),
            (NT, "", false),
        ),
        (
            (receive::OPEN, BLAKE, CARD, row_kind::ASSIGN, true, false),
            (W, "", false),
        ),
        (
            (receive::CLOSED, BLAKE, CARD, row_kind::ASSIGN, true, false),
            (NT, "", true),
        ),
        // B2: bypass sessions get rows on Open, held on Ask me and Closed.
        (
            (receive::OPEN, BLAKE, CARD, row_kind::COMMENT, false, true),
            (NT, "", false),
        ),
        (
            (receive::OPEN, BLAKE, CARD, row_kind::COMMENT, true, true),
            (W, "", false),
        ),
        (
            (receive::ASK, BLAKE, CARD, row_kind::COMMENT, false, true),
            (NT, hold::CROSS_PERSON, false),
        ),
        (
            (receive::CLOSED, BLAKE, "", row_kind::COMMENT, false, true),
            (NT, hold::CROSS_PERSON, false),
        ),
        // Members: picked people as on Open, the rest as Ask me.
        (
            (receive::MEMBERS, BLAKE, "", row_kind::COMMENT, true, false),
            (W, "", false),
        ),
        (
            (
                receive::MEMBERS,
                BLAKE,
                CARD,
                row_kind::COMMENT,
                false,
                true,
            ),
            (NT, "", false),
        ),
        (
            (receive::MEMBERS, CASEY, "", row_kind::COMMENT, true, false),
            (W, hold::MEMBER_WAKE, false),
        ),
        (
            (receive::MEMBERS, CASEY, "", row_kind::COMMENT, false, true),
            (NT, hold::CROSS_PERSON, false),
        ),
        // No row, or a bad one: Open.
        (
            ("", BLAKE, "", row_kind::COMMENT, true, true),
            (W, "", false),
        ),
        (
            ("sometimes", BLAKE, "", row_kind::COMMENT, true, true),
            (W, "", false),
        ),
    ];
    for ((preset, person, card, kind, wake, bypass), want) in cases {
        let mut s = set(preset);
        s.people = vec![BLAKE.into()];
        let f = from(person, card, kind, wake, "devB/1");
        let got = receive_decision(&s, &f, bypass);
        assert_eq!(
            got, want,
            "{preset} {person} card={card:?} {kind} wake={wake} bypass={bypass}"
        );
    }
}

#[test]
fn open_wakes_a_bypass_session() {
    let h = harness();
    let d = one(
        &h,
        &row(
            from(BLAKE, CARD, row_kind::COMMENT, true, "devB/7"),
            &[KEY_A],
        ),
    );
    assert_eq!(
        (d.state.as_str(), d.mode.as_str()),
        (st::QUEUED, mode::WAKE)
    );
    let m = &d.message;
    assert_eq!(m.from.key, board_sender_key(BOARD, CARD, BLAKE));
    assert_eq!(m.from.key, format!("board:{BOARD}/{CARD}"));
    assert_eq!(m.from.kind, BOARD_KIND);
    assert_eq!(m.trust, trust::UNTRUSTED);
    assert_eq!((m.ref_.as_str(), m.hop), ("task:01TASK", 1));
    let w = h.wake(KEY_A);
    assert!(
        w.prompt.contains(&format!(
            "from=\"blake's agent mlg-lead\" key=\"board:{BOARD}/{CARD}\""
        )) && w.prompt.contains(" board=\"mlg\"")
            && w.prompt
                .contains("A message with a board attribute came from another person"),
        "{}",
        w.prompt
    );
    // The person's own key is board:<id>/person:<id>.
    assert_eq!(
        board_sender_key(BOARD, "", BLAKE),
        format!("board:{BOARD}/person:{BLAKE}")
    );
}

#[test]
fn board_sender_is_never_the_person_or_an_own_agent() {
    // Every user: key and even board: keys count as owners here: a board
    // sender is still neither the person nor an own agent, so Ask me holds
    // it at the bypass session.
    let h = Harness::with(Hooks {
        is_owner: Some(std::sync::Arc::new(|k: &str| {
            k.starts_with("user:") || k.starts_with("board:")
        })),
        ..Hooks::default()
    });
    h.abc();
    h.ib.set_receive(set(receive::ASK), USER).unwrap();
    let d = one(
        &h,
        &row(
            from(BLAKE, "", row_kind::COMMENT, false, "devB/1"),
            &[KEY_B],
        ),
    );
    assert_eq!(
        (d.state.as_str(), d.hold.as_str()),
        (st::HELD, hold::CROSS_PERSON)
    );
    assert_eq!(d.message.trust, trust::UNTRUSTED);
    // A board key naming this device in its path doesn't pass either.
    let mut f = from(BLAKE, "", row_kind::COMMENT, false, "devB/2");
    f.card_id = "dev1/aaaaaaaa-1111-4111-8111-111111111111".into();
    let d = one(&h, &row(f, &[KEY_B]));
    assert_eq!(d.hold, hold::CROSS_PERSON);
}

#[test]
fn ask_me_holds_and_release_wakes() {
    let h = harness();
    h.ib.set_receive(
        ReceiveSetting {
            board_id: BOARD.into(),
            preset: receive::ASK.into(),
            ..ReceiveSetting::default()
        },
        USER,
    )
    .unwrap();
    let d = one(
        &h,
        &row(
            from(BLAKE, CARD, row_kind::COMMENT, true, "devB/3"),
            &[KEY_N],
        ),
    );
    assert_eq!(
        (d.state.as_str(), d.hold.as_str()),
        (st::HELD, hold::MEMBER_WAKE)
    );
    h.no_wake(KEY_N);
    h.ib.release(&d.id, USER).unwrap();
    let w = h.wake(KEY_N);
    assert_eq!(w.delivery_ids, vec![d.id.clone()]);
    // Another board still has the default, Open.
    let mut f = from(BLAKE, CARD, row_kind::COMMENT, true, "devB/4");
    f.board_id = "fedcba9876543210fedcba9876543210".into();
    let d = one(&h, &row(f, &[KEY_N]));
    assert_eq!(
        (d.state.as_str(), d.mode.as_str()),
        (st::QUEUED, mode::WAKE)
    );
}

#[test]
fn closed_turns_a_wake_into_next_turn() {
    let h = harness();
    h.ib.set_receive(
        ReceiveSetting {
            person_id: BLAKE.into(),
            preset: receive::CLOSED.into(),
            ..ReceiveSetting::default()
        },
        USER,
    )
    .unwrap();
    let d = one(
        &h,
        &row(from(BLAKE, "", row_kind::COMMENT, true, "devB/5"), &[KEY_N]),
    );
    assert_eq!(
        (d.state.as_str(), d.mode.as_str()),
        (st::QUEUED, mode::NEXT_TURN)
    );
    assert_eq!(d.message.reason, "asked to wake");
    h.no_wake(KEY_N);
    let w = h.append_for(KEY_N);
    assert!(
        w.prompt.contains("reason=\"asked to wake\""),
        "{}",
        w.prompt
    );
    // Closed holds rows to bypass sessions.
    let d = one(
        &h,
        &row(
            from(BLAKE, "", row_kind::COMMENT, false, "devB/6"),
            &[KEY_A],
        ),
    );
    assert_eq!(d.hold, hold::CROSS_PERSON);
}

#[test]
fn only_the_host_bridges_and_once() {
    let mut h = harness();
    let r = row(
        from(BLAKE, CARD, row_kind::COMMENT, false, "devB/9"),
        &[KEY_A, KEY_D2, KEY_A, "", USER],
    );
    let ds = h.ib.bridge(&r).unwrap();
    assert_eq!(ds.len(), 1, "{ds:?}");
    assert_eq!(ds[0].to, KEY_A);
    assert!(h.ib.bridged(BOARD, "devB/9", KEY_A));
    assert!(!h.ib.bridged(BOARD, "devB/9", KEY_D2));
    // Again: nothing new.
    assert!(h.ib.bridge(&r).unwrap().is_empty());
    // After a replay, and from a fresh fold of the same log.
    h.reopen();
    assert!(h.ib.bridge(&r).unwrap().is_empty());
    assert!(h.fresh().bridged(BOARD, "devB/9", KEY_A));
    // The same row adds a recipient later: only the new one gets it.
    let mut r2 = r.clone();
    r2.to = vec![KEY_A.into(), KEY_B.into()];
    let ds = h.ib.bridge(&r2).unwrap();
    assert_eq!(
        ds.iter().map(|d| d.to.as_str()).collect::<Vec<_>>(),
        [KEY_B]
    );
}

#[test]
fn the_other_device_hosts_its_own_agent() {
    // dev2 bridges for its agent; dev1, folding the same rows, doesn't.
    let h2 = Harness::on_device("dev2", Hooks::default());
    h2.ros.put(&[card(KEY_D2, "delta", "repo1", BYPASS_MODE)]);
    let r = row(
        from(BLAKE, CARD, row_kind::COMMENT, true, "devB/11"),
        &[KEY_A, KEY_D2],
    );
    let ds = h2.ib.bridge(&r).unwrap();
    assert_eq!(
        ds.iter().map(|d| d.to.as_str()).collect::<Vec<_>>(),
        [KEY_D2]
    );
    assert_eq!(ds[0].state, st::QUEUED);
}

#[test]
fn hop_limit_crosses_the_bridge() {
    let h = harness();
    let mut r = row(
        from(BLAKE, CARD, row_kind::COMMENT, true, "devB/20"),
        &[KEY_A],
    );
    r.hop = MAX_HOP;
    assert_eq!(one(&h, &r).state, st::QUEUED);
    r.from.row = "devB/21".into();
    r.hop = MAX_HOP + 1;
    let d = one(&h, &r);
    assert_eq!(
        (d.state.as_str(), d.hold.as_str()),
        (st::HELD, hold::HOP_LIMIT)
    );
}

#[test]
fn current_hop_follows_the_running_wake() {
    let h = harness();
    assert_eq!(h.ib.current_hop(KEY_A), 0);
    let mut r = row(
        from(BLAKE, CARD, row_kind::COMMENT, true, "devB/30"),
        &[KEY_A],
    );
    r.hop = 3;
    one(&h, &r);
    h.deliver_engage(KEY_A, "turn-1");
    assert_eq!(h.ib.current_hop(KEY_A), 4);
}

#[test]
fn rate_counts_per_person_and_board() {
    let h = harness();
    // Two of Blake's agents and Blake share one count to alpha.
    for n in 0..inbox::DIRECT_LIMIT {
        let card = if n % 2 == 0 { "cardB1" } else { "cardB2" };
        let d = one(
            &h,
            &row(
                from(
                    BLAKE,
                    card,
                    row_kind::COMMENT,
                    true,
                    &format!("devB/{}", 100 + n),
                ),
                &[KEY_A],
            ),
        );
        assert_eq!(d.mode, mode::WAKE, "row {n}");
    }
    let d = one(
        &h,
        &row(
            from(BLAKE, "", row_kind::COMMENT, true, "devB/200"),
            &[KEY_A],
        ),
    );
    assert_eq!(
        (d.state.as_str(), d.mode.as_str()),
        (st::QUEUED, mode::NEXT_TURN)
    );
    assert_eq!(d.message.reason, "rate limited");
    // Casey has a count of her own; beta has its own too.
    let d = one(
        &h,
        &row(from(CASEY, "", row_kind::COMMENT, true, "devC/1"), &[KEY_A]),
    );
    assert_eq!(d.mode, mode::WAKE);
    let d = one(
        &h,
        &row(
            from(BLAKE, "", row_kind::COMMENT, true, "devB/201"),
            &[KEY_B],
        ),
    );
    assert_eq!(d.mode, mode::WAKE);
    // Past the window, Blake wakes alpha again.
    h.clk.advance(inbox::DIRECT_WINDOW + secs(1));
    let d = one(
        &h,
        &row(
            from(BLAKE, "", row_kind::COMMENT, true, "devB/202"),
            &[KEY_A],
        ),
    );
    assert_eq!(d.mode, mode::WAKE);
}

#[test]
fn block_list_and_pause_still_apply() {
    let h = harness();
    let mut p = h.ib.policy();
    p.block = vec!["blake".into()];
    h.ib.set_policy(p, USER).unwrap();
    let d = one(
        &h,
        &row(
            from(BLAKE, "", row_kind::COMMENT, true, "devB/40"),
            &[KEY_A],
        ),
    );
    assert_eq!(d.hold, hold::BLOCKED);
    let mut p = h.ib.policy();
    p.block = vec![];
    p.paused = true;
    h.ib.set_policy(p, USER).unwrap();
    let d = one(
        &h,
        &row(
            from(BLAKE, "", row_kind::COMMENT, true, "devB/41"),
            &[KEY_A],
        ),
    );
    assert_eq!(d.hold, hold::PAUSED);
}

#[test]
fn bad_rows_are_refused() {
    let h = harness();
    let mut r = row(from(BLAKE, "", row_kind::COMMENT, false, ""), &[KEY_A]);
    assert!(h.ib.bridge(&r).is_err());
    r.from.row = "devB/1".into();
    r.from.person_id.clear();
    assert!(h.ib.bridge(&r).is_err());
    r.from.person_id = BLAKE.into();
    r.text = "  ".into();
    want_err(h.ib.bridge(&r), inbox::ErrorKind::Empty);
}

#[test]
fn settings_are_the_persons_and_most_specific_wins() {
    let mut h = harness();
    assert_eq!(h.ib.receive_setting(BOARD, BLAKE).preset, receive::OPEN);
    assert_eq!(h.ib.autonomy(BOARD), autonomy::TRUSTED);
    // Agents can't change either.
    assert!(h.ib.set_receive(set(receive::CLOSED), KEY_A).is_err());
    assert!(
        h.ib.set_autonomy(
            AutonomySetting {
                preset: autonomy::AUTONOMOUS.into(),
                ..AutonomySetting::default()
            },
            KEY_A
        )
        .is_err()
    );
    let e = h.ib.set_receive(set("sometimes"), USER).unwrap_err();
    assert_eq!(
        e.to_string(),
        "inbox: unknown receive setting \"sometimes\" (closed, ask, members or open)"
    );
    h.ib.set_receive(set(receive::CLOSED), USER).unwrap();
    h.ib.set_receive(
        ReceiveSetting {
            board_id: BOARD.into(),
            preset: receive::ASK.into(),
            ..ReceiveSetting::default()
        },
        USER,
    )
    .unwrap();
    h.ib.set_receive(
        ReceiveSetting {
            board_id: BOARD.into(),
            person_id: BLAKE.into(),
            preset: receive::MEMBERS.into(),
            people: vec![BLAKE.into(), " ".into(), BLAKE.into()],
        },
        USER,
    )
    .unwrap();
    h.ib.set_autonomy(
        AutonomySetting {
            board_id: BOARD.into(),
            preset: autonomy::ASK.into(),
        },
        USER,
    )
    .unwrap();
    let check = |h: &Harness| {
        let s = h.ib.receive_setting(BOARD, BLAKE);
        assert_eq!(
            (s.preset.as_str(), s.people.clone()),
            (receive::MEMBERS, vec![BLAKE.to_string()])
        );
        assert_eq!(h.ib.receive_setting(BOARD, CASEY).preset, receive::ASK);
        assert_eq!(h.ib.receive_setting("other", CASEY).preset, receive::CLOSED);
        assert_eq!(h.ib.autonomy(BOARD), autonomy::ASK);
        assert_eq!(h.ib.autonomy("other"), autonomy::TRUSTED);
        assert_eq!(h.ib.receive_settings().len(), 3);
    };
    check(&h);
    h.reopen();
    check(&h);
    // Clearing a row falls back to what it inherits.
    h.ib.set_receive(
        ReceiveSetting {
            board_id: BOARD.into(),
            person_id: BLAKE.into(),
            ..ReceiveSetting::default()
        },
        USER,
    )
    .unwrap();
    assert_eq!(h.ib.receive_setting(BOARD, BLAKE).preset, receive::ASK);
}

#[test]
fn a_forged_setting_row_is_ignored() {
    let h = harness();
    // An agent-actor row, or one whose by isn't the person, doesn't count.
    h.st.append(
        &format!("agent:{KEY_A}"),
        store::kinds::RECEIVE_SET,
        &serde_json::json!({"preset": "closed", "by": USER}),
    )
    .unwrap();
    h.st.append(
        "user",
        store::kinds::AUTONOMY_SET,
        &serde_json::json!({"preset": "autonomous", "by": KEY_A}),
    )
    .unwrap();
    let ib = h.fresh();
    assert_eq!(ib.receive_setting(BOARD, BLAKE).preset, receive::OPEN);
    assert_eq!(ib.autonomy(BOARD), autonomy::TRUSTED);
}
