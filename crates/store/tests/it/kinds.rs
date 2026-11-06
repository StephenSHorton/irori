//! Go's `TestEveryKindIsClassified` (kinds_test.go), recast for Rust
//! (store.md §6 "kinds"): every kind is a constant made by the one table
//! that is also the registry, so the source scan becomes these checks; the
//! table itself is compared with Go's in golden.rs.

use std::collections::HashSet;

use store::kinds::*;
use store::{Board, Side};

#[test]
fn every_kind_is_classified() {
    assert!(REGISTRY.len() >= 40, "only {} kinds", REGISTRY.len());
    let mut seen = HashSet::new();
    let dotted = regex::Regex::new(r"^[a-z]+(\.[a-z]+)+$").unwrap();
    for e in REGISTRY {
        assert!(seen.insert(e.kind), "{} listed twice", e.kind);
        assert!(dotted.is_match(e.kind), "{} is not a dotted kind", e.kind);
        assert_ne!(e.side, Side::Unclassified, "{} has no Side", e.kind);
        // Every replicated kind says whether a board log accepts it, and
        // only replicated kinds have a Board verdict.
        match e.side {
            Side::Replicated => {
                assert_ne!(
                    e.board,
                    Board::Unclassified,
                    "{} has no Board verdict",
                    e.kind
                )
            }
            _ => assert_eq!(e.board, Board::Unclassified, "{} is local", e.kind),
        }
        assert_eq!(store::kind_side(e.kind), e.side);
        assert_eq!(store::kind_board(e.kind), e.board);
    }
    for local in [POLICY_SET, MEMBER_JOINED] {
        assert!(store::is_local(local), "{local} must be device-local");
    }
    for rep in [
        AGENT_SEEN,
        LOCAL_MOVED,
        PERSON_DEVICE_ADDED,
        MESSAGE_SENT,
        TASK_CREATED,
        CAPTURE_TURN,
    ] {
        assert!(!store::is_local(rep), "{rep} must replicate");
    }
    assert_eq!(store::kind_side("future.kind"), Side::Unclassified);
    assert!(!store::board_accepted("future.kind"));
}
