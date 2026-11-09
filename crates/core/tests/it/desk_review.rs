//! What the desk and `prs` show of a PR's review (merge desk §12.9, card
//! #1080): the pick state, the claimer, the related agents and the claim
//! counts, read from the boards with nothing written.

use crate::pr_cards::{REPO, url};
use crate::review_pick::{alices_gh, missed_world};

fn verdict(c: &irori_core::Core) -> irori_core::deskmap::DeskAuthor {
    c.desk_verdicts(REPO)
        .into_iter()
        .find(|v| v.pr == 1)
        .expect("PR #1 on the desk")
}

/// Asked, then claimed: the line goes from "asked" to the claimer, the
/// pinged agent shows as related with its reason, and the claim counts
/// follow. `prs` words the same with no capture ledger needed.
#[test]
fn the_desk_and_prs_show_the_ask_the_claim_and_the_related_agent() {
    let gh = alices_gh();
    let (t, _card, asked, pinged, _) = missed_world(&gh);
    let v = verdict(t.a());
    assert_eq!(v.review.pick, "asked", "{v:?}");
    assert!(!v.review.asked.is_empty(), "{v:?}");
    assert!(v.review.claimer.is_none());
    assert_eq!(v.review.related.len(), 1, "{v:?}");
    assert_eq!(v.review.related[0].why, "same card", "{v:?}");
    assert!(t.a().desk_claim_counts().is_empty());

    t.b()
        .board_coord(&t.board)
        .unwrap()
        .claim_pr(&asked, &url(1))
        .unwrap();
    t.sync();
    crate::common::wait_for(10, "alice sees the claim", || {
        verdict(t.a()).review.claimer.is_some()
    });
    let v = verdict(t.a());
    assert_eq!(v.review.pick, "claimed", "{v:?}");
    let c = v.review.claimer.unwrap();
    assert!(!c.agent.is_empty() && !c.until.is_empty(), "{c:?}");
    assert_eq!(v.review.why_none, "");
    assert_ne!(asked, pinged);

    let counts = t.a().desk_claim_counts();
    assert_eq!(counts.len(), 1, "{counts:?}");
    assert_eq!((counts[0].open, counts[0].recent), (1, 1));

    let caller = tools::Caller {
        sender: inbox::Sender::new(&t.ra.key_a, "agent", "claude"),
        path: inbox::path::MOD.into(),
    };
    let args = serde_json::json!({"repo": REPO});
    let text = t
        .a()
        .tools()
        .call(&caller, "prs", &serde_json::to_vec(&args).unwrap())
        .unwrap();
    assert!(
        text.contains(&format!("#1 {}", url(1)))
            && text.contains(&format!("claimed by {}", c.agent))
            && text.contains("related"),
        "{text}"
    );
}
