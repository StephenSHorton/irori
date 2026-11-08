//! The Storage page (card #728): what Irori keeps here and its size by
//! kind, each kind's items, and the clean-outs the person asks for.

use std::fs;

use irori_window::storage::{self, StorageItem};

use crate::common::Rig;

fn items(r: &Rig, kind: &str) -> Vec<StorageItem> {
    r.w.storage_items(kind).unwrap()
}

fn ids(v: &[StorageItem]) -> Vec<String> {
    let mut out: Vec<String> = v.iter().map(|i| i.id.clone()).collect();
    out.sort();
    out
}

/// The overview lists every kind with its size; the log's breakdown names
/// what its rows are.
#[test]
fn the_report_lists_each_kind_with_its_size() {
    let r = Rig::new();
    fs::create_dir_all(r.data.join("updates")).unwrap();
    fs::write(
        r.data.join("updates/Irori_0.11.0_x64-setup.exe"),
        vec![0u8; 3000],
    )
    .unwrap();
    let rep = r.w.storage_report().unwrap();
    let kinds: Vec<&str> = rep.kinds.iter().map(|k| k.id.as_str()).collect();
    for k in [
        storage::LOG,
        storage::SPACES,
        storage::FILES,
        storage::BUILDS,
        storage::UPDATES,
        storage::LOGS,
        storage::BRIEFS,
        storage::BACKUPS,
        storage::RECEIVED,
        storage::VOICE,
        storage::CAPTURE,
        storage::CLAUDE,
    ] {
        assert!(kinds.contains(&k), "{k} missing from {kinds:?}");
    }
    let of = |id: &str| rep.kinds.iter().find(|k| k.id == id).unwrap();
    assert!(of(storage::LOG).bytes > 0, "the log has a size");
    assert_eq!(of(storage::UPDATES).bytes, 3000);
    assert_eq!(of(storage::UPDATES).items, 1);
    assert!(!of(storage::BUILDS).measured, "build output comes later");
    let parts = r.w.storage_log().unwrap();
    assert!(!parts.is_empty());
    assert!(
        parts.windows(2).all(|w| w[0].bytes >= w[1].bytes),
        "largest first: {parts:?}"
    );
    assert_eq!(storage::log_part("capture.turn"), "conversations");
    assert_eq!(storage::log_part("talk.post"), "talk");
    assert_eq!(storage::log_part("task.comment"), "cards");
    assert_eq!(storage::log_part("message.sent"), "messages");
    assert_eq!(storage::log_part("assistant.chat.said"), "assistant");
}

/// Installers, old logs, briefs of settled cards, backups and saved-out
/// copies go; what Irori still writes or needs stays and says why.
#[test]
fn cleaning_files_around_the_log() {
    let r = Rig::new();
    let d = &r.data;
    fs::create_dir_all(d.join("updates")).unwrap();
    fs::write(d.join("updates/a.exe"), b"aaaa").unwrap();
    fs::write(d.join("updates/b.exe"), b"bb").unwrap();
    fs::write(d.join("irori.log.1"), b"old").unwrap();
    fs::write(d.join("crash-1657.log"), b"x").unwrap();
    fs::write(d.join("notes.txt"), b"not a log").unwrap();
    fs::write(d.join("irori.db.v0.3"), b"0123456789").unwrap();
    fs::create_dir_all(d.join("reset-20260101-ab/sub")).unwrap();
    fs::write(d.join("reset-20260101-ab/sub/irori.db"), b"12345").unwrap();
    fs::create_dir_all(d.join("received/dev_1")).unwrap();
    fs::write(d.join("received/dev_1/trace.txt"), b"t").unwrap();

    // Logs: the current one stays, the rotated and stray ones go.
    let logs = items(&r, storage::LOGS);
    assert!(logs.iter().all(|i| i.id != "notes.txt"));
    let rotated = logs.iter().find(|i| i.id == "irori.log.1").unwrap();
    assert!(rotated.cleanable);
    if let Some(cur) = logs.iter().find(|i| i.id == "irori.log") {
        assert!(!cur.cleanable);
        assert_eq!(cur.keep, storage::IN_USE);
    }
    let done = r.w.storage_clean(storage::LOGS, &[]).unwrap();
    assert_eq!(done.removed, 2);
    assert!(!d.join("irori.log.1").exists());
    assert!(!d.join("crash-1657.log").exists());
    assert!(d.join("notes.txt").exists());

    // One installer by name, then the rest.
    let one =
        r.w.storage_clean(storage::UPDATES, &["b.exe".into()])
            .unwrap();
    assert_eq!((one.removed, one.freed), (1, 2));
    assert!(d.join("updates/a.exe").exists());
    r.w.storage_clean(storage::UPDATES, &[]).unwrap();
    assert!(items(&r, storage::UPDATES).is_empty());

    // A name that reaches out of its folder is never followed.
    let out =
        r.w.storage_clean(storage::UPDATES, &["../notes.txt".into()])
            .unwrap();
    assert_eq!(out.removed, 0);
    assert!(d.join("notes.txt").exists());

    assert_eq!(
        ids(&items(&r, storage::BACKUPS)),
        vec!["irori.db.v0.3".to_string(), "reset-20260101-ab".to_string()]
    );
    let b = r.w.storage_clean(storage::BACKUPS, &[]).unwrap();
    assert_eq!((b.removed, b.freed), (2, 15));
    assert!(!d.join("reset-20260101-ab").exists());

    assert_eq!(
        ids(&items(&r, storage::RECEIVED)),
        vec!["dev_1".to_string()]
    );
    r.w.storage_clean(storage::RECEIVED, &[]).unwrap();
    assert!(!d.join("received/dev_1").exists());
}

/// A brief goes once its card is settled; an open card's stays.
#[test]
fn briefs_of_settled_cards_go() {
    let r = Rig::new();
    let co = r.w.core.coord();
    let me = r.w.core.user_sender().key;
    let mk = |title: &str| {
        co.create_task(
            &r.key_a,
            coord::NewTask {
                title: title.into(),
                ..Default::default()
            },
        )
        .unwrap()
    };
    let open = mk("still going");
    let done = mk("finished");
    co.set_task_state(&me, &done.id, coord::CANCELED, "")
        .unwrap();
    let briefs = r.data.join("briefs");
    fs::create_dir_all(&briefs).unwrap();
    for id in [&open.id, &done.id] {
        fs::write(briefs.join(format!("{id}-1.md")), b"brief").unwrap();
        fs::write(briefs.join(format!("{id}-1.settings.json")), b"{}").unwrap();
    }
    fs::write(briefs.join("01GONE-2.md"), b"gone").unwrap();
    let got = items(&r, storage::BRIEFS);
    let of = |id: &str| got.iter().find(|i| i.id == id).unwrap().clone();
    assert!(!of(&open.id).cleanable);
    assert_eq!(of(&open.id).keep, storage::CARD_OPEN);
    assert!(of(&done.id).cleanable);
    assert_eq!(of(&done.id).bytes, 7);
    assert!(
        of("01GONE").cleanable,
        "a card not here can't be waiting on it"
    );
    let out = r.w.storage_clean(storage::BRIEFS, &[]).unwrap();
    assert_eq!(out.removed, 2);
    assert!(briefs.join(format!("{}-1.md", open.id)).exists());
    assert!(!briefs.join(format!("{}-1.md", done.id)).exists());
    assert!(!briefs.join(format!("{}-1.settings.json", done.id)).exists());
    // Asked for by id, an open card's brief stays and says why.
    let kept =
        r.w.storage_clean(storage::BRIEFS, std::slice::from_ref(&open.id))
            .unwrap();
    assert_eq!(kept.removed, 0);
    assert_eq!(kept.kept[0].why, storage::CARD_OPEN);
    assert_eq!(storage::brief_card("01ABC-12.settings.json"), Some("01ABC"));
    assert_eq!(storage::brief_card("01ABC.md"), None);
}

/// An attachment is listed by the name it was attached with, and deleting
/// it here marks it so it isn't picked up again.
#[test]
fn attachments_are_listed_and_deleted() {
    let r = Rig::new();
    let outside = tempfile::tempdir().unwrap();
    let src = outside.path().join("trace.txt");
    fs::write(&src, b"frame 12 took 40ms").unwrap();
    let refs =
        r.w.core
            .attach_files(&r.key_a, &[src.to_string_lossy().into_owned()])
            .unwrap();
    let sha = refs[0].sha256.clone();
    let got = items(&r, storage::FILES);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, sha);
    assert_eq!(got[0].name, "trace.txt");
    assert_eq!(got[0].bytes, 18);
    assert!(got[0].only_here, "attached here: maybe the only copy");
    let out =
        r.w.storage_clean(storage::FILES, std::slice::from_ref(&sha))
            .unwrap();
    assert_eq!((out.removed, out.freed), (1, 18));
    assert!(!r.w.core.files().has(&sha));
    assert!(r.w.core.files().dropped(&sha));
    assert!(items(&r, storage::FILES).is_empty());
}

/// Resolved threads clear like done cards: they leave the threads list and
/// Storage's cleared list shows them, with Put back.
#[test]
fn clearing_threads_and_putting_cards_back() {
    let r = Rig::new();
    let w = &r.w;
    let open = w
        .start_thread("Is the build green?", &[], &[], "")
        .unwrap()
        .task
        .id;
    let over = w.start_thread("Which port?", &[], &[], "").unwrap().task.id;
    w.resolve_thread(&over).unwrap();
    let listed = |id: &str| w.threads("").iter().any(|t| t.id == id);
    assert!(listed(&open) && listed(&over));
    assert_eq!(w.clear_threads_count(3600), 0, "none older than an hour");
    assert_eq!(w.clear_threads_count(0), 1, "only the resolved one");
    assert_eq!(w.clear_threads(0).unwrap(), 1);
    assert!(!listed(&over), "a cleared thread leaves the list");
    assert!(listed(&open));
    assert!(
        w.core.coord().get_task(&over).is_some(),
        "show still finds it"
    );
    assert_eq!(w.clear_threads_count(0), 0);

    // A done card cleared the old way shows there too.
    let co = w.core.coord();
    let me = w.core.user_sender().key;
    let card = co
        .create_task(
            &r.key_a,
            coord::NewTask {
                title: "old card".into(),
                ..Default::default()
            },
        )
        .unwrap();
    co.set_task_state(&me, &card.id, coord::CANCELED, "")
        .unwrap();
    w.clear_done(0, std::slice::from_ref(&card.id), "").unwrap();
    let cleared = w.storage_cleared();
    let got: Vec<(&str, bool)> = cleared.iter().map(|c| (c.id.as_str(), c.thread)).collect();
    assert!(got.contains(&(over.as_str(), true)), "{got:?}");
    assert!(got.contains(&(card.id.as_str(), false)), "{got:?}");

    // Reopened, a cleared card is back in the lists, so not listed here.
    w.reopen(&card.id).unwrap();
    assert!(!w.storage_cleared().iter().any(|c| c.id == card.id));
    co.set_task_state(&me, &card.id, coord::CANCELED, "")
        .unwrap();
    assert!(w.storage_cleared().iter().any(|c| c.id == card.id));

    assert_eq!(
        w.storage_unclear(&[over.clone(), card.id.clone()]).unwrap(),
        2
    );
    assert!(listed(&over), "put back in the list");
    assert!(w.storage_cleared().is_empty());
    assert!(
        w.done_cards(0, "")
            .unwrap()
            .cards
            .iter()
            .any(|c| c.id == card.id)
    );
}

/// Build output: only a worktree Irori made and an agent isn't working in.
#[test]
fn build_output_needs_a_tree_irori_made() {
    let r = Rig::new();
    assert!(items(&r, storage::BUILDS).is_empty());
    let err = r.w.core.clear_build(&r.data.to_string_lossy()).unwrap_err();
    assert!(err.contains("isn't a worktree Irori made here"), "{err}");
}

/// A kind the page doesn't know is refused.
#[test]
fn an_unknown_kind_is_refused() {
    let r = Rig::new();
    assert!(r.w.storage_items("everything").is_err());
    assert!(r.w.storage_clean("everything", &[]).is_err());
    // The log and capture's files list nothing to delete.
    assert!(items(&r, storage::LOG).is_empty());
    assert!(items(&r, storage::CAPTURE).is_empty());
}
