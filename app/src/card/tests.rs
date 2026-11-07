//! The card page's pure helpers (card-detail.test.ts and the page's own).

use irori_window::cards::{
    CardBoard, CardDetail, CardOption, CardSummary, CardThreadItem, Holding,
};

use super::blockers::{by_recommendation, gist};
use super::side::card_files;
use super::thread::startable;

fn card(state: &str) -> CardDetail {
    CardDetail {
        summary: CardSummary {
            state: state.into(),
            ..CardSummary::default()
        },
        ..CardDetail::default()
    }
}

#[test]
fn a_private_card_on_a_personal_board_can_start_an_agent() {
    let mut d = card("submitted");
    assert!(startable(&d));
    d.summary.tag = "home".into();
    assert!(startable(&d));
}

#[test]
fn a_shared_card_cant_start_an_agent() {
    let mut d = card("submitted");
    d.summary.tag = "mlg".into();
    d.on_board = Some(CardBoard {
        id: "s".into(),
        ..CardBoard::default()
    });
    assert!(!startable(&d));
}

#[test]
fn an_ended_or_held_card_cant_start_an_agent() {
    assert!(!startable(&card("completed")));
    assert!(!startable(&card("canceled")));
    let mut d = card("working");
    d.summary.holder.state = Holding::Active;
    assert!(!startable(&d));
}

#[test]
fn recommended_options_come_first_keeping_their_index() {
    let o = |a: &str, rec: bool| CardOption {
        answer: a.into(),
        recommended: rec,
        ..CardOption::default()
    };
    let opts = vec![o("a", false), o("b", true), o("c", false), o("d", true)];
    let got: Vec<(&str, usize)> = by_recommendation(&opts)
        .into_iter()
        .map(|(o, i)| (o.answer.as_str(), i))
        .collect();
    assert_eq!(got, [("b", 1), ("d", 3), ("a", 0), ("c", 2)]);
}

#[test]
fn gist_cuts_a_long_answer_to_one_line() {
    assert_eq!(gist("short\nanswer"), "short answer");
    let long = "x".repeat(300);
    let g = gist(&long);
    assert_eq!(g.chars().count(), 200);
    assert!(g.ends_with('…'));
}

#[test]
fn card_files_lists_each_file_once() {
    let item = |shas: &[&str]| -> CardThreadItem {
        let mut it = CardThreadItem {
            attachments: vec![Default::default(); shas.len()],
            ..CardThreadItem::default()
        };
        for (a, sha) in it.attachments.iter_mut().zip(shas) {
            a.sha256 = sha.to_string();
            a.name = format!("{sha}.png");
        }
        it
    };
    let mut d = card("working");
    d.thread = vec![item(&["a", "b"]), item(&["b", "c"])];
    let got: Vec<String> = card_files(&d).into_iter().map(|f| f.sha256).collect();
    assert_eq!(got, ["a", "b", "c"]);
}

#[test]
fn an_items_meta_line_joins_only_what_it_knows() {
    use super::blockers::meta_line;
    assert_eq!(meta_line(Some("oak"), "", None, ""), "Raised by oak");
    assert_eq!(
        meta_line(Some("oak"), "2m ago", None, ""),
        "Raised by oak · 2m ago"
    );
    assert_eq!(
        meta_line(None, "2m ago", Some("ash"), "closed by You"),
        "2m ago · for ash to decide · closed by You"
    );
    assert_eq!(meta_line(None, "", None, ""), "");
}

#[test]
fn each_blocker_gets_an_answer_box_and_a_resolve_note() {
    use irori_window::cards::CardBlocker;
    let mut d = card("submitted");
    d.summary.id = "c1".into();
    d.blockers = vec![CardBlocker {
        id: "b1".into(),
        ..CardBlocker::default()
    }];
    assert_eq!(
        super::box_keys(&d),
        ["c1/comment", "c1/blocker/b1", "c1/resolve/b1"]
    );
}

#[test]
fn an_items_files_show_each_file_once() {
    let mut it = CardThreadItem {
        attachments: vec![Default::default(); 3],
        ..CardThreadItem::default()
    };
    for (a, sha) in it.attachments.iter_mut().zip(["x", "y", "x"]) {
        a.sha256 = sha.to_string();
    }
    let got: Vec<String> = super::files_once(&it.attachments)
        .into_iter()
        .map(|f| f.sha256)
        .collect();
    assert_eq!(got, ["x", "y"]);
}
