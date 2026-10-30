//! Ported from I:internal/identity/membership_test.go: the membership
//! fold's rules with fake keys, every row set folded in every arrival order
//! (up to seven rows) to the same Membership.

mod support;

use std::collections::BTreeMap;

use irori_identity::*;
use serde::Serialize;
use support::*;

/// MembershipKeys without crypto: a device belongs to the person its name
/// starts with ("s-pc" is s's), and a row is signed when its sig is
/// "ok:<person>".
struct FakeKeys {
    devs: BTreeMap<&'static str, Vec<&'static str>>,
    cuts: BTreeMap<String, i64>,
}

impl MembershipKeys for FakeKeys {
    fn person_of(&self, dev: &str) -> String {
        self.devs
            .iter()
            .find(|(_, ds)| ds.contains(&dev))
            .map(|(p, _)| p.to_string())
            .unwrap_or_default()
    }
    fn signed(&self, p: &str, epoch: i64, _: &RowAt, sig: &str, _: Option<&[u8]>) -> bool {
        epoch == 1 && sig == format!("ok:{p}")
    }
    fn has_key(&self, _: &str, _: i64, _: &str) -> bool {
        false
    }
    fn devices(&self, p: &str) -> Vec<String> {
        self.devs
            .get(p)
            .map(|d| d.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default()
    }
    fn cut(&self, dev: &str) -> Option<i64> {
        self.cuts.get(dev).copied()
    }
}

fn test_keys() -> FakeKeys {
    FakeKeys {
        devs: [
            ("s", vec!["s-pc", "s-mac"]),
            ("b", vec!["b-pc"]),
            ("c", vec!["c-pc"]),
            ("d", vec!["d-pc"]),
        ]
        .into_iter()
        .collect(),
        cuts: BTreeMap::new(),
    }
}

fn pin_of(p: &str) -> MemberPin {
    MemberPin {
        person_id: p.into(),
        person_pub: "a".repeat(64),
        root_device_id: format!("{p}-pc"),
        person_epoch: 1,
    }
}

fn anchor_s() -> BTreeMap<String, MemberPin> {
    [("s".to_string(), pin_of("s"))].into_iter().collect()
}

#[derive(Default)]
struct Mb {
    hlc: u64,
    seq: BTreeMap<String, i64>,
    rows: Vec<MembershipRow>,
}

type Cut = Option<BTreeMap<String, i64>>;

fn cut(pairs: &[(&str, i64)]) -> Cut {
    Some(pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect())
}

impl Mb {
    fn add<T: Serialize>(&mut self, dev: &str, kind: &str, payload: &T) {
        self.hlc += 1;
        *self.seq.entry(dev.into()).or_default() += 1;
        self.rows.push(MembershipRow {
            at: RowAt {
                hlc: self.hlc,
                device: dev.into(),
                seq: self.seq[dev],
            },
            kind: kind.into(),
            payload: to_json(payload),
        });
    }
    fn created(&mut self, by: &str) {
        let p = BoardCreated {
            board_id: "bd".into(),
            name: "mlg".into(),
            tag: "mlg".into(),
            creator: pin_of(by),
            person_sig: format!("ok:{by}"),
            ..BoardCreated::default()
        };
        self.add(&format!("{by}-pc"), kinds::BOARD_CREATED, &p);
    }
    fn added(&mut self, dev: &str, by: &str, who: &str, role: &str) {
        let p = pin_of(who);
        let r = BoardMemberAdded {
            person_id: who.into(),
            person_pub: p.person_pub,
            root_device_id: p.root_device_id,
            person_epoch: 1,
            name: who.into(),
            role: role.into(),
            by: by.into(),
            by_epoch: 1,
            person_sig: format!("ok:{by}"),
            ..BoardMemberAdded::default()
        };
        self.add(dev, kinds::BOARD_MEMBER_ADDED, &r);
    }
    fn role(&mut self, dev: &str, by: &str, who: &str, role: &str) {
        let r = BoardMemberRole {
            person_id: who.into(),
            role: role.into(),
            by: by.into(),
            person_epoch: 1,
            person_sig: format!("ok:{by}"),
            ..Default::default()
        };
        self.add(dev, kinds::BOARD_MEMBER_ROLE, &r);
    }
    fn removed(&mut self, dev: &str, by: &str, who: &str, cut: Cut) {
        let r = BoardMemberRemoved {
            person_id: who.into(),
            by: by.into(),
            cut,
            person_epoch: 1,
            person_sig: format!("ok:{by}"),
            ..Default::default()
        };
        self.add(dev, kinds::BOARD_MEMBER_REMOVED, &r);
    }
    fn left(&mut self, dev: &str, who: &str, cut: Cut) {
        let r = BoardMemberLeft {
            person_id: who.into(),
            cut,
            person_epoch: 1,
            person_sig: format!("ok:{who}"),
            ..Default::default()
        };
        self.add(dev, kinds::BOARD_MEMBER_LEFT, &r);
    }
    fn renamed(&mut self, dev: &str, by: &str, name: &str) {
        let r = BoardRenamed {
            name: name.into(),
            by: by.into(),
            person_epoch: 1,
            person_sig: format!("ok:{by}"),
            ..Default::default()
        };
        self.add(dev, kinds::BOARD_RENAMED, &r);
    }
}

fn roles(m: &Membership) -> String {
    let mut out: Vec<String> = m
        .members
        .iter()
        .map(|p| format!("{}={}/{}", p.person_id, p.role, p.state))
        .collect();
    out.sort();
    out.join(" ")
}

fn reasons(m: &Membership) -> Vec<String> {
    m.conflicts
        .iter()
        .map(|c| format!("{} {}: {}", c.kind, c.person_id, c.reason))
        .collect()
}

struct Case {
    name: &'static str,
    rows: fn(&mut Mb),
    roles: &'static str,
    cuts: &'static [(&'static str, i64)],
    conflicts: &'static [&'static str],
    board_name: &'static str,
}

#[test]
fn fold_membership_rules() {
    let cases = [
        Case {
            name: "the creator is the first owner",
            rows: |b| b.created("s"),
            roles: "s=owner/active",
            cuts: &[],
            conflicts: &[],
            board_name: "",
        },
        Case {
            name: "an owner adds members and owners",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.added("s-mac", "s", "c", "owner");
            },
            roles: "b=member/active c=owner/active s=owner/active",
            cuts: &[],
            conflicts: &[],
            board_name: "",
        },
        Case {
            name: "a non-owner's add, role and rename are ignored",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.added("b-pc", "b", "c", "member");
                b.role("b-pc", "b", "b", "owner");
                b.renamed("b-pc", "b", "blake's");
            },
            roles: "b=member/active s=owner/active",
            cuts: &[],
            conflicts: &[
                "board.member.added c: only an owner adds",
                "board.member.role b: only an owner changes",
                "board.renamed : only an owner renames",
            ],
            board_name: "mlg",
        },
        Case {
            name: "membership chains back to board.created: a promoted owner adds",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.role("s-pc", "s", "b", "owner");
                b.added("b-pc", "b", "c", "member");
                b.renamed("b-pc", "b", "ours");
            },
            roles: "b=owner/active c=member/active s=owner/active",
            cuts: &[],
            conflicts: &[],
            board_name: "ours",
        },
        Case {
            name: "the last owner can't demote themselves; with two owners one can",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.role("s-pc", "s", "s", "member");
                b.role("s-pc", "s", "b", "owner");
                b.role("b-pc", "b", "b", "member");
            },
            roles: "b=member/active s=owner/active",
            cuts: &[],
            conflicts: &["board.member.role s: the last owner"],
            board_name: "",
        },
        Case {
            name: "a removal cuts every device of the person, named or not",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.removed("s-pc", "s", "b", cut(&[("b-pc", 7)]));
            },
            roles: "b=member/removed s=owner/active",
            cuts: &[("b-pc", 7)],
            conflicts: &[],
            board_name: "",
        },
        Case {
            name: "a device the remover didn't know of is cut at 0",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "c", "member");
                b.removed("s-pc", "s", "c", cut(&[("b-pc", 9)]));
            },
            roles: "c=member/removed s=owner/active",
            cuts: &[("c-pc", 0)],
            conflicts: &[],
            board_name: "",
        },
        Case {
            name: "concurrent remove and promote: the removal stands",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "c", "owner");
                b.added("s-pc", "s", "d", "member");
                b.removed("s-pc", "s", "d", cut(&[("d-pc", 2)]));
                b.role("c-pc", "c", "d", "owner");
            },
            roles: "c=owner/active d=member/removed s=owner/active",
            cuts: &[("d-pc", 2)],
            conflicts: &[
                "board.member.role d: they were removed or left before it; the removal stands",
            ],
            board_name: "",
        },
        Case {
            name: "a promote before the removal is undone by it",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "c", "owner");
                b.added("s-pc", "s", "d", "member");
                b.role("c-pc", "c", "d", "owner");
                b.removed("s-pc", "s", "d", cut(&[("d-pc", 0)]));
            },
            roles: "c=owner/active d=owner/removed s=owner/active",
            cuts: &[("d-pc", 0)],
            conflicts: &[],
            board_name: "",
        },
        Case {
            name: "a removed person's rows are ignored until an owner adds them again",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "owner");
                b.removed("s-pc", "s", "b", cut(&[("b-pc", 1)]));
                b.role("s-pc", "s", "b", "member");
                b.added("s-pc", "s", "b", "member");
            },
            roles: "b=member/active s=owner/active",
            cuts: &[("b-pc", 1)],
            conflicts: &["board.member.role b: they were removed or left"],
            board_name: "",
        },
        Case {
            name: "leaving cuts the leaver, never below the leave row itself",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.add("b-pc", "noise", &serde_json::Value::Null);
                b.left("b-pc", "b", cut(&[("b-pc", 1)]));
            },
            roles: "b=member/left s=owner/active",
            cuts: &[("b-pc", 2)],
            conflicts: &[],
            board_name: "",
        },
        Case {
            name: "a leave for someone else, or a removal by a non-owner, counts nowhere",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.added("s-pc", "s", "c", "member");
                let l = BoardMemberLeft {
                    person_id: "c".into(),
                    person_epoch: 1,
                    person_sig: "ok:b".into(),
                    ..Default::default()
                };
                b.add("b-pc", kinds::BOARD_MEMBER_LEFT, &l);
                b.removed("b-pc", "b", "c", None);
            },
            roles: "b=member/active c=member/active s=owner/active",
            cuts: &[],
            conflicts: &[
                "board.member.left c: it isn't signed by the person leaving",
                "board.member.removed c: only an owner removes",
            ],
            board_name: "",
        },
        Case {
            name: "two owners removing each other while apart are both removed",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "owner");
                b.added("s-pc", "s", "c", "member");
                b.removed("s-pc", "s", "b", cut(&[("b-pc", 0)]));
                b.removed("b-pc", "b", "s", cut(&[("s-pc", 3), ("s-mac", 0)]));
            },
            roles: "b=owner/removed c=member/active s=owner/removed",
            cuts: &[("b-pc", 0), ("s-pc", 3), ("s-mac", 0)],
            conflicts: &["board.member.removed s: its remover was no longer an owner"],
            board_name: "",
        },
        Case {
            name: "rows by a forged writer or another person's key count nowhere",
            rows: |b| {
                b.created("s");
                let mut a = BoardMemberAdded {
                    person_id: "c".into(),
                    role: "owner".into(),
                    by: "s".into(),
                    by_epoch: 1,
                    person_sig: "ok:s".into(),
                    person_pub: pin_of("c").person_pub,
                    root_device_id: "c-pc".into(),
                    person_epoch: 1,
                    ..Default::default()
                };
                b.add("b-pc", kinds::BOARD_MEMBER_ADDED, &a);
                a.person_id = "d".into();
                a.role = "member".into();
                a.person_sig = "ok:b".into();
                a.root_device_id = "d-pc".into();
                b.add("s-pc", kinds::BOARD_MEMBER_ADDED, &a);
            },
            roles: "s=owner/active",
            cuts: &[],
            conflicts: &[
                "board.member.added c: its signature doesn't count",
                "board.member.added d: its signature doesn't count",
            ],
            board_name: "",
        },
        Case {
            name: "only the anchored creator's board.created counts",
            rows: |b| {
                b.created("b");
                b.created("s");
                b.created("s");
            },
            roles: "s=owner/active",
            cuts: &[],
            conflicts: &[
                "board.created b: it comes before",
                "board.created s: the board was already created",
            ],
            board_name: "",
        },
        Case {
            name: "a re-add can't change the key the board pins",
            rows: |b| {
                b.created("s");
                b.added("s-pc", "s", "b", "member");
                b.removed("s-pc", "s", "b", cut(&[]));
                let a = BoardMemberAdded {
                    person_id: "b".into(),
                    role: "member".into(),
                    by: "s".into(),
                    by_epoch: 1,
                    person_sig: "ok:s".into(),
                    person_pub: "b".repeat(64),
                    root_device_id: "b-pc".into(),
                    person_epoch: 1,
                    ..Default::default()
                };
                b.add("s-pc", kinds::BOARD_MEMBER_ADDED, &a);
            },
            roles: "b=member/removed s=owner/active",
            cuts: &[("b-pc", 0)],
            conflicts: &["board.member.added b: it pins another key"],
            board_name: "",
        },
    ];
    let keys = test_keys();
    for tc in &cases {
        let mut b = Mb::default();
        (tc.rows)(&mut b);
        let want = fold_membership("bd", &anchor_s(), &b.rows, &keys);
        assert!(want.created, "{}: board.created didn't count", tc.name);
        assert_eq!(roles(&want), tc.roles, "{}", tc.name);
        let want_cuts: BTreeMap<String, i64> =
            tc.cuts.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        assert_eq!(want.cuts, want_cuts, "{}", tc.name);
        let got = reasons(&want);
        assert_eq!(
            got.len(),
            tc.conflicts.len(),
            "{}: conflicts {got:?}",
            tc.name
        );
        for (g, sub) in got.iter().zip(tc.conflicts) {
            assert!(g.contains(sub), "{}: conflict {g:?} lacks {sub:?}", tc.name);
        }
        if !tc.board_name.is_empty() {
            assert_eq!(want.name, tc.board_name, "{}", tc.name);
        }
        if b.rows.len() > 7 {
            continue;
        }
        perm(b.rows.len(), &mut |idx| {
            let rows: Vec<MembershipRow> = idx.iter().map(|&i| b.rows[i].clone()).collect();
            let m = fold_membership("bd", &anchor_s(), &rows, &keys);
            assert_eq!(m, want, "{}: order {idx:?}", tc.name);
        });
    }
}

#[test]
fn fold_membership_needs_its_board() {
    let mut b = Mb::default();
    b.created("s");
    b.added("s-pc", "s", "b", "owner");
    let keys = test_keys();
    let mut another_root = anchor_s();
    another_root.get_mut("s").unwrap().root_device_id = "s-mac".into();
    for (name, m) in [
        (
            "another board",
            fold_membership("other", &anchor_s(), &b.rows, &keys),
        ),
        (
            "no anchor",
            fold_membership("bd", &BTreeMap::new(), &b.rows, &keys),
        ),
        (
            "another root",
            fold_membership("bd", &another_root, &b.rows, &keys),
        ),
    ] {
        assert!(!m.created && m.members.is_empty(), "{name}: {}", roles(&m));
    }
    // "" skips the board id check.
    assert!(fold_membership("", &anchor_s(), &b.rows, &keys).created);
}

#[test]
fn board_names_and_tags() {
    for (tag, ok) in [
        ("mlg", true),
        ("a", true),
        ("mlg-2", true),
        ("abcdefghijklmnop", true),
        ("", false),
        ("Mlg", false),
        ("12", false),
        ("1abc", false),
        ("ml#g", false),
        ("abcdefghijklmnopq", false),
        ("ml g", false),
        ("-x", false),
    ] {
        assert_eq!(valid_board_tag(tag), ok, "{tag:?}");
    }
    for (name, ok) in [
        ("mlg".to_string(), true),
        ("Blake & me".into(), true),
        ("".into(), false),
        (" mlg".into(), false),
        ("a\nb".into(), false),
        ("x".repeat(80), true),
        ("x".repeat(81), false),
    ] {
        assert_eq!(valid_board_name(&name), ok, "{name:?}");
    }
}

#[test]
fn membership_json_is_gos_shape() {
    // No board: Go's nil Members is null and the omitempty fields go.
    let m = fold_membership("bd", &BTreeMap::new(), &[], &test_keys());
    assert_eq!(
        String::from_utf8(to_json(&m)).unwrap(),
        r#"{"created":false,"boardId":"bd","name":"","tag":"","creator":"","members":null}"#
    );
}
