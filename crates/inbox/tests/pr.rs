//! I:internal/inbox/pr_test.go.

mod common;

use std::sync::Arc;

use common::*;
use inbox::{
    Address, BYPASS_MODE, DIRECT_LIMIT, Delivery, ErrorKind, GONE_GRACE, Policy, UNENGAGED_LIMIT,
    addr, hold, normalize_pr, path, st,
};

const PR_KEY: &str = "pr:acme/app#7";
const PR_REPO: &str = "remote:github.com/acme/app";
const KEY_D: &str = "dev1/dddddddd-4444-4444-8444-444444444444";
const KEY_S: &str = "dev1/ssssssss-5555-4555-8555-555555555555";
const KEY_GONE: &str = "dev1/eeeeeeee-6666-4666-8666-666666666666";

fn to_pr(v: &str) -> Address {
    Address::new(addr::PR, v)
}

fn pr_harness(owners: Option<Vec<&str>>, with_desk: bool) -> Harness {
    let owners = owners.map(|o| o.into_iter().map(String::from).collect::<Vec<_>>());
    let hooks = Hooks {
        owners: owners.map(|o| {
            Arc::new(move |k: &str| if k == PR_KEY { o.clone() } else { vec![] })
                as inbox::PrOwnersFn
        }),
        ..Hooks::default()
    };
    let h = Harness::with(hooks);
    h.ros.put(&[card(KEY_S, "sigma", "repo9", BYPASS_MODE)]);
    if with_desk {
        h.ros.put(&[card(KEY_D, "delta", PR_REPO, BYPASS_MODE)]);
        h.ib.set_reachable(KEY_D, path::MOD, true);
    }
    h
}

fn next_of(h: &Harness, d: &Delivery) -> Delivery {
    h.ib.list(0)
        .into_iter()
        .rfind(|x| x.message_id == d.message_id && x.id != d.id)
        .unwrap_or_default()
}

#[test]
fn normalize_pr_forms() {
    for (i, want) in [
        ("pr:acme/app#7", PR_KEY),
        (" PR:Acme/App#7 ", PR_KEY),
        ("Acme/App#7", PR_KEY),
        ("https://github.com/Acme/App/pull/7", PR_KEY),
        ("github.com/acme/app/pull/7/files", PR_KEY),
        ("http://www.github.com/acme/app/pull/7?w=1", PR_KEY),
    ] {
        assert_eq!(normalize_pr(i).unwrap(), want, "{i}");
    }
    for bad in [
        "",
        " ",
        "7",
        "#7",
        "app#7",
        "pr:acme/app",
        "pr:acme/app#0",
        "acme/app#x",
        "https://github.com/acme/app/issues/7",
        "https://gitlab.com/acme/app/pull/7",
        "pr:../app#7",
    ] {
        want_err(normalize_pr(bad), ErrorKind::BadPr);
    }
}

#[test]
fn pr_routing() {
    struct Case {
        name: &'static str,
        owners: Option<Vec<&'static str>>,
        live: Vec<&'static str>,
        gone_card: Vec<&'static str>,
        desk: bool,
        from: &'static str,
        want_to: &'static str,
        want_repo: &'static str,
        want_for: Option<Vec<String>>,
    }
    let c = |name,
             owners: Option<Vec<&'static str>>,
             live: Vec<&'static str>,
             desk,
             want_to,
             want_repo,
             want_for: Option<Vec<String>>| Case {
        name,
        owners,
        live,
        gone_card: vec![],
        desk,
        from: "",
        want_to,
        want_repo,
        want_for,
    };
    let mut cases = vec![
        c(
            "live author",
            Some(vec![KEY_A, KEY_B]),
            vec![KEY_A, KEY_B],
            true,
            KEY_A,
            "",
            None,
        ),
        c(
            "author gone, toucher live",
            Some(vec![KEY_A, KEY_B]),
            vec![KEY_B],
            true,
            KEY_B,
            "",
            None,
        ),
        c(
            "ledger order wins",
            Some(vec![KEY_C, KEY_B]),
            vec![KEY_B, KEY_C],
            false,
            KEY_C,
            "",
            None,
        ),
        c(
            "owner not reachable but live",
            Some(vec![KEY_B]),
            vec![KEY_B],
            true,
            KEY_B,
            "",
            None,
        ),
        c(
            "all owners gone: desk, For names the author",
            Some(vec![KEY_A, KEY_B]),
            vec![],
            true,
            KEY_D,
            PR_REPO,
            Some(vec![format!("alpha ({KEY_A})")]),
        ),
        c(
            "owner never seen: For has its key",
            Some(vec![KEY_GONE]),
            vec![],
            true,
            KEY_D,
            PR_REPO,
            Some(vec![KEY_GONE.into()]),
        ),
        c(
            "no owners: desk, no For",
            Some(vec![]),
            vec![],
            true,
            KEY_D,
            PR_REPO,
            None,
        ),
        c("no ledger: desk", None, vec![], true, KEY_D, PR_REPO, None),
        c(
            "no desk: unowned with For",
            Some(vec![KEY_A]),
            vec![],
            false,
            "",
            PR_REPO,
            Some(vec!["alpha".into()]),
        ),
    ];
    let mut gc = c(
        "a gone card isn't live",
        Some(vec![KEY_A, KEY_B]),
        vec![KEY_B],
        false,
        KEY_B,
        "",
        None,
    );
    gc.gone_card = vec![KEY_A];
    cases.push(gc);
    let mut so = c(
        "sender owns it: passed over",
        Some(vec![KEY_A, KEY_B]),
        vec![KEY_A, KEY_B],
        false,
        KEY_B,
        "",
        None,
    );
    so.from = KEY_A;
    cases.push(so);
    let mut sd = c(
        "sender is the only owner and the desk: unowned",
        Some(vec![KEY_D]),
        vec![],
        true,
        "",
        PR_REPO,
        None,
    );
    sd.from = KEY_D;
    cases.push(sd);

    let names = [(KEY_A, "alpha"), (KEY_B, "beta"), (KEY_C, "gamma")];
    let name_of = |k: &str| names.iter().find(|(x, _)| *x == k).unwrap().1;
    for tc in cases {
        let h = pr_harness(tc.owners.clone(), tc.desk);
        for (k, n) in names {
            h.ros.put(&[card(k, n, "repo1", BYPASS_MODE)]);
        }
        h.send(&claude(KEY_S, "sigma"), &to("alpha"), "hello");
        h.ros.remove(&[KEY_A, KEY_B, KEY_C]);
        for k in &tc.live {
            h.ros.put(&[card(k, name_of(k), "repo1", BYPASS_MODE)]);
        }
        for k in &tc.gone_card {
            let mut cd = card(k, name_of(k), "repo1", BYPASS_MODE);
            cd.status = "gone".into();
            h.ros.put(&[cd]);
        }
        let from = if tc.from.is_empty() {
            claude(KEY_S, "sigma")
        } else {
            claude(tc.from, "owner")
        };
        let (m, d) =
            h.ib.send(
                &from,
                &to_pr("https://github.com/Acme/App/pull/7"),
                "is this ready?",
                "",
            )
            .unwrap();
        assert!(m.to.kind == addr::PR && m.to.value == PR_KEY, "{}", tc.name);
        assert!(
            d.to == tc.want_to && d.repo_key == tc.want_repo,
            "{}: routed to {:?} repo {:?}",
            tc.name,
            d.to,
            d.repo_key
        );
        let (ws, wh) = if tc.want_to.is_empty() {
            (st::HELD, hold::UNOWNED)
        } else {
            (st::QUEUED, "")
        };
        assert!(d.state == ws && d.hold == wh, "{}", tc.name);
        match &tc.want_for {
            None => assert!(d.for_.is_empty(), "{}: For {:?}", tc.name, d.for_),
            Some(v) => {
                for s in v {
                    assert!(
                        d.for_.contains(s.as_str()),
                        "{}: For {:?} lacks {s}",
                        tc.name,
                        d.for_
                    );
                }
            }
        }
    }
}

#[test]
fn pr_invalid_targets() {
    let h = pr_harness(None, true);
    for bad in [
        "",
        "7",
        "acme#7",
        "pr:acme",
        "https://github.com/acme/app/issues/7",
    ] {
        let e =
            h.ib.send(&claude(KEY_S, "sigma"), &to_pr(bad), "x", "")
                .unwrap_err();
        assert!(e.is(ErrorKind::BadPr));
        if !bad.is_empty() {
            assert!(e.to_string().contains("pr:<owner>/<name>#<number>"), "{e}");
        }
    }
    assert!(h.ib.list(0).is_empty());
}

#[test]
fn pr_unowned_then_desk() {
    let mut h = pr_harness(Some(vec![KEY_GONE]), false);
    let (_, u) = h.send(
        &claude(KEY_S, "sigma"),
        &to_pr("acme/app#7"),
        "who has this?",
    );
    assert!(
        u.to.is_empty()
            && u.state == st::HELD
            && u.hold == hold::UNOWNED
            && u.repo_key == PR_REPO
            && u.for_.contains(KEY_GONE)
    );
    want_err(h.ib.release(&u.id, USER), ErrorKind::BadState);
    h.hooks.owners = None;
    h.reopen();
    let got = h.want_state(&u.id, st::HELD);
    assert!(got.repo_key == PR_REPO && got.for_ == u.for_ && got.message.to.value == PR_KEY);
    h.ros.put(&[card(KEY_D, "delta", PR_REPO, BYPASS_MODE)]);
    h.ib.set_reachable(KEY_D, path::MOD, true);
    h.sweep();
    let got = h.want_state(&u.id, st::QUEUED);
    assert!(got.to == KEY_D && got.for_ == u.for_);
    assert!(h.wake(KEY_D).prompt.contains("who has this?"));
}

#[test]
fn pr_desk_gone_keeps_note() {
    let h = pr_harness(Some(vec![KEY_GONE]), true);
    let (_, d) = h.send(&claude(KEY_S, "sigma"), &to_pr(PR_KEY), "who has this?");
    assert!(d.to == KEY_D && !d.for_.is_empty());
    h.ros.remove(&[KEY_D]);
    h.sweep();
    h.clk.advance(GONE_GRACE + secs(1));
    h.sweep();
    let moved = h.want_state(&d.id, st::MOVED);
    let next = next_of(&h, &d);
    assert!(
        moved.moved_to.is_empty()
            && next.state == st::HELD
            && next.hold == hold::UNOWNED
            && next.for_ == d.for_
            && next.repo_key == PR_REPO
    );
}

#[test]
fn pr_owner_routed_stays_put() {
    let h = pr_harness(Some(vec![KEY_A]), true);
    h.ros.put(&[card(KEY_A, "alpha", PR_REPO, BYPASS_MODE)]);
    let (_, d) = h.send(
        &claude(KEY_S, "sigma"),
        &to_pr("pr:acme/app#7"),
        "rebase please",
    );
    assert!(d.to == KEY_A && d.repo_key.is_empty());
    h.clk.advance(UNENGAGED_LIMIT + GONE_GRACE + mins(1));
    h.sweep();
    h.sweep();
    assert_eq!(h.want_state(&d.id, st::QUEUED).to, KEY_A);
}

#[test]
fn pr_policy_and_rate() {
    let h = pr_harness(Some(vec![KEY_A]), true);
    h.ros.put(&[card(KEY_A, "alpha", PR_REPO, BYPASS_MODE)]);
    let from = claude(KEY_S, "sigma");
    for i in 0..DIRECT_LIMIT {
        h.ib.send(&from, &to_pr(PR_KEY), "ping", "")
            .unwrap_or_else(|e| panic!("send {i}: {e}"));
    }
    want_err(
        h.ib.send(&from, &to_pr(PR_KEY), "ping", ""),
        ErrorKind::RateLimited,
    );
    let blk = Policy {
        block: vec!["sigma".into()],
        ..Policy::default()
    };
    h.ib.set_policy(blk.clone(), USER).unwrap();
    let (_, d) =
        h.ib.send(&person(), &to_pr(PR_KEY), "from the GUI", "")
            .unwrap();
    assert!(d.state == st::QUEUED && d.to == KEY_A);
    let h2 = pr_harness(None, true);
    h2.ib.set_policy(blk, USER).unwrap();
    let (_, d) = h2.ib.send(&from, &to_pr(PR_KEY), "blocked?", "").unwrap();
    assert!(d.state == st::HELD && d.hold == hold::BLOCKED && d.to == KEY_D);
}

#[test]
fn pr_wake_names_pr_and_owner() {
    let h = pr_harness(Some(vec![KEY_GONE]), true);
    h.send(
        &claude(KEY_S, "sigma"),
        &to_pr("acme/app#7"),
        "who has this?",
    );
    let p = h.wake(KEY_D).prompt;
    for s in [
        format!("pr=\"{PR_KEY}\""),
        format!("for=\"{KEY_GONE}"),
        "who has this?".into(),
    ] {
        assert!(p.contains(&s), "wake lacks {s}:\n{p}");
    }
    let h = pr_harness(Some(vec![KEY_A]), true);
    h.ros.put(&[card(KEY_A, "alpha", PR_REPO, BYPASS_MODE)]);
    h.send(&claude(KEY_S, "sigma"), &to_pr(PR_KEY), "rebase please");
    let p = h.wake(KEY_A).prompt;
    assert!(
        p.contains(&format!("pr=\"{PR_KEY}\"")) && !p.contains(" for=\""),
        "{p}"
    );
}

#[test]
fn pr_owner_gone_moves_to_desk() {
    let h = pr_harness(Some(vec![KEY_A]), true);
    h.ros.put(&[card(KEY_A, "alpha", PR_REPO, BYPASS_MODE)]);
    let (_, d) = h.send(&claude(KEY_S, "sigma"), &to_pr(PR_KEY), "rebase please");
    assert_eq!(d.to, KEY_A);
    h.ros.remove(&[KEY_A]);
    h.sweep();
    h.clk.advance(GONE_GRACE + mins(60));
    h.sweep();
    h.want_state(&d.id, st::MOVED);
    let next = next_of(&h, &d);
    assert!(
        next.to == KEY_D
            && next.state == st::QUEUED
            && next.repo_key == PR_REPO
            && next.for_.contains(&format!("alpha ({KEY_A})")),
        "{next:?}"
    );
    let p = h.wake(KEY_D).prompt;
    assert!(p.contains("rebase please") && p.contains(KEY_A));
}

#[test]
fn pr_defer_keeps_owner() {
    let h = pr_harness(Some(vec![KEY_GONE]), true);
    let (_, d) = h.send(&claude(KEY_S, "sigma"), &to_pr(PR_KEY), "who has this?");
    h.deliver_engage(KEY_D, "t1");
    h.ib.defer(KEY_D, &d.id, 5, "later").unwrap();
    let got = h.state(&d.id);
    assert!(got.note == "later" && got.for_ == d.for_ && d.for_.contains(KEY_GONE));
    h.ros.remove(&[KEY_D]);
    h.sweep();
    h.clk.advance(GONE_GRACE + secs(1));
    h.sweep();
    let next = next_of(&h, &d);
    assert!(
        next.for_ == d.for_ && next.note.is_empty() && next.state == st::HELD,
        "{next:?}"
    );
    let h = pr_harness(Some(vec![KEY_GONE]), true);
    let (_, d) = h.send(&claude(KEY_S, "sigma"), &to_pr(PR_KEY), "who has this?");
    h.ib.skip(KEY_D, &d.id, "not mine").unwrap();
    let got = h.state(&d.id);
    assert!(got.note == "not mine" && got.for_ == d.for_);
}
