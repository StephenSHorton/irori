//! Merge desk slice 2 (§3): when an agent on this device runs `gh pr
//! create`, Irori links the new PR to the one card that agent holds.

use crate::common;

use common::*;
use coord::NewTask;
use serde_json::json;

const PR: &str = "https://github.com/you/game-repo/pull/13";

fn slug(p: &std::path::Path) -> String {
    p.to_string_lossy()
        .chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect()
}

#[test]
fn an_opened_pr_is_linked_to_the_authors_held_card() {
    let r = Rig::new();
    let co = r.core.coord();
    let t = co
        .create_task(
            &r.key_a,
            NewTask {
                title: "fix the thing".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_a, &t.id).unwrap();
    // Another agent's card stays as it is.
    let other = co
        .create_task(
            &r.key_b,
            NewTask {
                title: "other work".into(),
                ..NewTask::default()
            },
        )
        .unwrap();
    co.claim_task(&r.key_b, &other.id).unwrap();

    let cwd = r.data.clone();
    let at = gocompat::time::GoTime::from_system(std::time::SystemTime::now())
        .utc()
        .format_rfc3339_nano();
    let sid = SESS_A;
    let base = |typ: &str, uuid: &str| {
        json!({"parentUuid": null, "isSidechain": false, "type": typ, "uuid": uuid,
               "timestamp": at, "userType": "external", "entrypoint": "cli",
               "cwd": cwd.to_string_lossy(), "sessionId": sid, "version": "2.1.300",
               "gitBranch": "toffee/13-fix"})
    };
    let mut prompt = base("user", "u1");
    prompt["promptId"] = json!("p1");
    prompt["message"] = json!({"role": "user", "content": "open a pr"});
    let mut call = base("assistant", "u2");
    call["message"] = json!({"id": "msg_b1", "role": "assistant", "type": "message",
        "content": [{"type": "tool_use", "id": "b1", "name": "Bash",
                     "input": {"command": "gh pr create --fill", "description": "run"}}]});
    let mut res = base("user", "u3");
    res["message"] = json!({"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "b1", "is_error": false, "content": "ok"}]});
    res["toolUseResult"] = json!({"stdout": format!("{PR}\n"), "stderr": "",
        "interrupted": false, "isImage": false});
    let dir = r.claude.join("projects").join(slug(&cwd));
    std::fs::create_dir_all(&dir).unwrap();
    let mut body = String::new();
    for row in [prompt, call, res] {
        body.push_str(&serde_json::to_string(&row).unwrap());
        body.push('\n');
    }
    std::fs::write(dir.join(format!("{sid}.jsonl")), body).unwrap();

    wait_for(20, "the PR linked to the author's card", || {
        co.get_task(&t.id)
            .is_some_and(|t| t.links.iter().any(|l| l.reference == PR))
    });
    assert!(co.get_task(&other.id).unwrap().links.is_empty());
}
