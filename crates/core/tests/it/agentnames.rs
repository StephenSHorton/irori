//! The agent name book (card #98): names outlive agents and restarts.

use coord::roster::AgentCard;
use irori_core::agentnames::{AgentNames, KEEP};

fn card(key: &str, name: &str) -> AgentCard {
    AgentCard {
        key: key.into(),
        name: name.into(),
        ..AgentCard::default()
    }
}

#[test]
fn names_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let b = AgentNames::new(dir.path());
    b.note(&[card("dev1/grok-abc", "oak"), card("dev1/s2", "")], 1_000);
    assert_eq!(b.get("dev1/grok-abc"), "oak");
    assert_eq!(b.get("dev1/s2"), "", "an unnamed card isn't kept");
    // A new book on the same folder: a Core restart.
    let b2 = AgentNames::new(dir.path());
    assert_eq!(b2.get("dev1/grok-abc"), "oak");
    assert_eq!(b2.all().len(), 1);
}

#[test]
fn a_rename_wins_and_old_names_drop() {
    let dir = tempfile::tempdir().unwrap();
    let b = AgentNames::new(dir.path());
    b.note(&[card("dev1/a", "oak")], 1_000);
    b.note(&[card("dev1/a", "birch")], 2_000);
    assert_eq!(b.get("dev1/a"), "birch");
    // Not seen for longer than KEEP: forgotten.
    b.note(&[], 2_000 + KEEP.as_secs() as i64 + 1);
    assert_eq!(b.get("dev1/a"), "");
    assert_eq!(AgentNames::new(dir.path()).get("dev1/a"), "");
}

#[test]
fn an_unreadable_file_starts_over() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(irori_core::agentnames::FILE), b"{not json").unwrap();
    let b = AgentNames::new(dir.path());
    assert_eq!(b.all().len(), 0);
    b.note(&[card("dev1/a", "oak")], 1);
    assert_eq!(AgentNames::new(dir.path()).get("dev1/a"), "oak");
}
