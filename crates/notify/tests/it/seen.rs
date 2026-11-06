//! notify-seen (#364): what earlier runs showed, one delivery id per line.

use std::collections::HashSet;

use inbox::watch::SeenStore;
use notify::{SEEN_FILE, SeenFile};

#[test]
fn seen_file_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let f = SeenFile::new(&dir.path().join("data"));
    assert!(f.load().is_empty(), "nothing yet reads as empty");
    let ids: HashSet<String> = ["dlv_b", "dlv_a"].iter().map(|s| s.to_string()).collect();
    f.save(&ids);
    assert_eq!(f.load(), ids);
    let raw = std::fs::read_to_string(dir.path().join("data").join(SEEN_FILE)).unwrap();
    assert_eq!(raw, "dlv_a\ndlv_b\n");
    f.save(&HashSet::new());
    assert!(f.load().is_empty());
}
