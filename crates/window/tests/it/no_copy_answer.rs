//! Card #531: answering Irori's "Choose its folder" blocker in the window.
//! The window resolves an Irori ask with the answer, so the reply is on no
//! comment: a reply that isn't a folder still comes back quoted, with why
//! it is asked again, the answer doesn't say nobody will act on it, and a
//! folder's path, alone or in a sentence (card #681), starts the agent
//! there. A card that names no repo is never asked about (card #600), so
//! these use a repo no device has.

use crate::common;

use base64::Engine;
use std::path::Path;
use std::sync::{Arc, Mutex};

use common::*;
use coord::{Blocker, NewTask, Task};
use irori_core::spawn::{SpawnReq, SpawnSetting, SpawnStarted, Spawner};
use irori_core::spawn_stop::is_no_copy;
use irori_window::Window;

/// A repo no device has a copy of.
const NOWHERE: &str = "remote:github.com/example/nowhere";

struct T {
    r: Rig,
    started: Arc<Mutex<Vec<SpawnReq>>>,
}

fn rig() -> T {
    let r = Rig::new();
    let started: Arc<Mutex<Vec<SpawnReq>>> = Arc::default();
    let s = started.clone();
    r.w.core.set_spawner(Some(Spawner {
        spawn: Arc::new(move |req: &SpawnReq| {
            s.lock().unwrap().push(req.clone());
            Ok(SpawnStarted {
                name: req.name.clone(),
                prefix: "dddddddd".into(),
            })
        }),
        end: Arc::new(|_| Ok(String::new())),
        trusted: Arc::new(|_: &Path| true),
        list: None,
        ready: None,
        stop: None,
    }));
    r.w.core
        .set_spawn_setting(SpawnSetting {
            on: true,
            ..SpawnSetting::default()
        })
        .unwrap();
    T { r, started }
}

impl T {
    fn w(&self) -> &Window {
        &self.r.w
    }
    fn card(&self, repo: &str) -> Task {
        let person = format!("user:{}", self.w().core.device_id());
        self.w()
            .core
            .coord()
            .create_task(
                &person,
                NewTask {
                    title: "Sort the board by age".into(),
                    repo_key: repo.into(),
                    ..NewTask::default()
                },
            )
            .unwrap()
    }
    fn get(&self, id: &str) -> Task {
        self.w().core.coord().get_task(id).unwrap()
    }
    /// The card's open "no copy" blocker once it is blocked on one that
    /// isn't `not`.
    fn asked(&self, id: &str, not: &str) -> Blocker {
        let open = || {
            let t = self.get(id);
            let b = t.blockers.iter().find(|b| b.open() && is_no_copy(b));
            b.filter(|b| b.id != not && t.state == coord::BLOCKED)
                .cloned()
        };
        wait("the no-copy blocker", || {
            self.w().core.set_spawn_cap(self.w().core.spawn_cap()).ok();
            open().is_some()
        });
        open().unwrap()
    }
}

#[test]
fn a_reply_that_is_no_folder_comes_back_quoted_and_a_path_starts_the_agent() {
    let t = rig();
    let c = t.card(NOWHERE);
    let b = t.asked(&c.id, "");
    assert!(b.question.starts_with("No device has a copy of"), "{b:?}");

    // The words from #392: the blocker isn't understood.
    let said = "i don't understand this blocker";
    let res = t.w().answer(&c.id, &b.id, -1, said, true, &[]).unwrap();
    assert!(res.irori_acts, "the window says Irori has it, not nobody");

    let b2 = t.asked(&c.id, &b.id);
    assert_ne!(b2.question, b.question, "not the same question back");
    assert!(b2.question.contains(&format!("“{said}”")), "{b2:?}");
    assert!(b2.question.contains("Choose its folder"), "{b2:?}");
    assert!(
        b2.context.starts_with(&format!(
            "“{said}” doesn't name a folder, so Irori can't start the agent yet and is asking again."
        )),
        "{}",
        b2.context
    );
    assert!(
        b2.context.contains("none of your devices has one"),
        "{}",
        b2.context
    );
    assert!(!b2.context.contains("isn't a"), "{}", b2.context);
    assert!(t.started.lock().unwrap().is_empty());

    // A folder's path, as the window sends it: the agent starts there.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().display().to_string();
    let res = t.w().answer(&c.id, &b2.id, -1, &path, true, &[]).unwrap();
    assert!(res.irori_acts);
    wait("its agent started", || {
        t.w().core.set_spawn_cap(t.w().core.spawn_cap()).ok();
        !t.started.lock().unwrap().is_empty()
    });
    assert_eq!(Path::new(&t.started.lock().unwrap()[0].folder), dir.path());
    let open: Vec<_> = t
        .get(&c.id)
        .blockers
        .into_iter()
        .filter(|b| b.open() && is_no_copy(b))
        .collect();
    assert!(open.is_empty(), "{open:?}");
}

/// An answer with a file puts the card back on the Board before it can be
/// resolved: that closes the blocker, and it is no error.
#[test]
fn a_reply_with_a_file_is_taken() {
    let t = rig();
    let c = t.card(NOWHERE);
    let b = t.asked(&c.id, "");
    let file = irori_window::board::UploadFile {
        name: "where.txt".into(),
        data: base64::engine::general_purpose::STANDARD.encode(b"here"),
    };
    let res = t
        .w()
        .answer_files(&c.id, &b.id, -1, "see this file", true, &[], &[file])
        .unwrap();
    assert!(res.irori_acts);
    let b2 = t.asked(&c.id, &b.id);
    assert!(b2.question.contains("“see this file”"), "{b2:?}");
}

/// A card an agent took while it waited is theirs: Irori won't start one,
/// and its question closes by itself, saying who took it (card #717), so
/// it leaves Needs you rather than wait on an answer nothing reads.
#[test]
fn a_card_with_a_holder_does_not_promise_an_agent() {
    let t = rig();
    let c = t.card(NOWHERE);
    let b = t.asked(&c.id, "");
    t.w().core.coord().claim_task(&t.r.key_a, &c.id).unwrap();
    wait("the question closes", || {
        t.w().core.set_spawn_cap(t.w().core.spawn_cap()).ok();
        t.get(&c.id)
            .blockers
            .iter()
            .any(|x| x.id == b.id && !x.open())
    });
    let done = t
        .get(&c.id)
        .blockers
        .into_iter()
        .find(|x| x.id == b.id)
        .and_then(|x| x.resolved)
        .unwrap();
    assert!(
        done.answer
            .ends_with("took the card, so no agent needs to start for it."),
        "{done:?}"
    );
    assert!(
        t.w()
            .answer(&c.id, &b.id, -1, "any folder", true, &[])
            .is_err(),
        "nothing left to answer"
    );
    assert!(t.started.lock().unwrap().is_empty());
}

/// A path that isn't there is still told as one, and each reply gets its
/// own question: nothing stops at the second round.
#[test]
fn a_wrong_path_is_said_so_each_time() {
    let t = rig();
    let c = t.card(NOWHERE);
    let b = t.asked(&c.id, "");
    assert!(b.question.starts_with("No device has a copy of"), "{b:?}");

    let gone = tempfile::tempdir().unwrap().path().join("gone");
    let gone = gone.display().to_string();
    t.w().answer(&c.id, &b.id, -1, &gone, true, &[]).unwrap();
    let b2 = t.asked(&c.id, &b.id);
    assert!(
        b2.context
            .starts_with(&format!("“{gone}” isn't a folder on")),
        "{}",
        b2.context
    );

    t.w()
        .answer(&c.id, &b2.id, -1, "where do I find it?", true, &[])
        .unwrap();
    let b3 = t.asked(&c.id, &b2.id);
    assert!(
        b3.context
            .starts_with("“where do I find it?” doesn't name a folder"),
        "{}",
        b3.context
    );
    assert!(b3.context.contains("clone it first"), "{}", b3.context);
}

/// Card #681: a folder in a sentence is read as the folder, as Stephen
/// wrote it on #677 ("put the clone in ~/projects/rune").
#[test]
fn a_folder_in_a_sentence_starts_the_agent_there() {
    let t = rig();
    let c = t.card(NOWHERE);
    let b = t.asked(&c.id, "");
    let dir = tempfile::tempdir().unwrap();
    let said = format!("put the clone in {}, thanks", dir.path().display());
    let res = t.w().answer(&c.id, &b.id, -1, &said, true, &[]).unwrap();
    assert!(res.irori_acts);
    wait("its agent started", || {
        t.w().core.set_spawn_cap(t.w().core.spawn_cap()).ok();
        !t.started.lock().unwrap().is_empty()
    });
    assert_eq!(Path::new(&t.started.lock().unwrap()[0].folder), dir.path());
}
