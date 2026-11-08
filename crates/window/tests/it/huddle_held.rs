//! Card #1046: a huddle's held assistant session, end to end on this
//! computer: the Core's policy over the window's held `claude -p`
//! (a fake here, never the real one). It primes with the packet, answers
//! looks on one process, and starts again from the seed, packet included,
//! after the process is killed or its context passes the limit.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use irori_core::huddle_session::RESTART_AT;
use irori_core::{HuddleRunner, HuddleSeed};
use irori_window::assistant::{HUDDLES_DIR, HeldRunner, Input, Job};

use crate::common::Rig;

const CALL: &str = "0123456789abcdef0123456789abcdef";

/// The fake held `claude`: logs each start's arguments and every message,
/// writes its pid, answers each message with `reply.txt` until stdin ends.
fn fake_held(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let d = dir.display();
    let p = dir.join("claude");
    std::fs::write(
        &p,
        format!(
            "#!/bin/sh\n\
             echo \"$@\" >> '{d}/argv.txt'\n\
             echo $$ > '{d}/pid'\n\
             while IFS= read -r line; do\n\
               printf '%s\\n' \"$line\" >> '{d}/in.txt'\n\
               cat '{d}/reply.txt'\n\
             done\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    reply(dir, 6000);
    p
}

/// What the fake answers: a result whose context is `cache_read` and a
/// little more.
fn reply(dir: &Path, cache_read: u64) {
    std::fs::write(
        dir.join("reply.txt"),
        format!(
            "{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"<<item kind=\\\"note\\\">>ok<</item>>\",\"usage\":{{\"input_tokens\":3,\"cache_creation_input_tokens\":10,\"cache_read_input_tokens\":{cache_read},\"output_tokens\":7}},\"model_usage_since_last_result\":{{\"m\":{{\"costUSD\":0.0002}}}}}}\n"
        ),
    )
    .unwrap();
}

fn starts(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join("argv.txt"))
        .unwrap_or_default()
        .lines()
        .count()
}

fn messages(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("in.txt"))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            v["message"]["content"].as_str().unwrap().to_string()
        })
        .collect()
}

/// A Core with a Personal channel holding one card, and the window's held
/// runner on the fake `claude`.
fn rig(fake: &Path) -> (Rig, Arc<HeldRunner>, HuddleSeed) {
    let r = Rig::new();
    let me = r.core.user_sender().key;
    let ch = r.core.talk_channel_create(&me, "us", "").unwrap().id;
    r.core
        .coord()
        .create_task(
            &me,
            coord::NewTask {
                title: "Ship the loader".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let bin = fake_held(fake);
    let core = Arc::downgrade(&r.core);
    let runner = Arc::new(HeldRunner::with(
        Box::new(move || Ok(bin.clone())),
        Box::new(move |call: &str, system: &str| {
            let c = core.upgrade().ok_or("closing")?;
            let mut job = Job::huddle(&c, call, system)?;
            job.timeout = Duration::from_secs(20);
            Ok(job)
        }),
    ));
    r.core
        .set_huddle_runner(Some(runner.clone() as Arc<dyn HuddleRunner>));
    let seed = HuddleSeed {
        call_id: CALL.into(),
        conv_id: ch,
        name: "Juno \u{b7} Addam's assistant".into(),
        ..Default::default()
    };
    (r, runner, seed)
}

#[test]
fn a_held_session_primes_looks_and_starts_again_from_the_seed() {
    let f = tempfile::tempdir().unwrap();
    let (r, runner, mut seed) = rig(f.path());

    // The priming turn, then looks on the same process.
    r.core.huddle_open(&seed).unwrap();
    for i in 1..=3 {
        let l = r.core.huddle_look(&seed, &format!("lines {i}")).unwrap();
        assert!(l.text.contains("ok"), "{l:?}");
        assert!((l.cost_usd - 0.0002).abs() < 1e-12);
    }
    assert_eq!(starts(f.path()), 1);
    let m = messages(f.path());
    assert_eq!(m.len(), 4, "{m:?}");
    assert!(m[0].contains("Ship the loader") && m[0].contains("<irori-data kind=\"space-cards\">"));
    assert_eq!(&m[1..], ["lines 1", "lines 2", "lines 3"]);
    // Its arguments: held, no tools, no MCP config, no secret.
    let argv = std::fs::read_to_string(f.path().join("argv.txt")).unwrap();
    assert!(argv.contains("--input-format stream-json"), "{argv}");
    assert!(
        !argv.contains("--mcp-config") && !argv.contains("--allowedTools"),
        "{argv}"
    );
    let dir = r.core.assistant_dir().join(HUDDLES_DIR).join(CALL);
    assert!(dir.join("s0").join("system.md").exists());
    assert!(!dir.join("s0").join("mcp.json").exists());

    // Killed between looks: the next look starts again from the seed.
    let pid = std::fs::read_to_string(f.path().join("pid")).unwrap();
    assert!(
        std::process::Command::new("kill")
            .args(["-9", pid.trim()])
            .status()
            .unwrap()
            .success()
    );
    std::thread::sleep(Duration::from_millis(200));
    seed.recent = vec!["[12:01:02] Addam: do we have anyone on signing?".into()];
    r.core.huddle_look(&seed, "lines 4").unwrap();
    assert_eq!(starts(f.path()), 2);
    let m = messages(f.path());
    let reseed = &m[4];
    assert!(
        reseed.contains("Ship the loader"),
        "the seed has the packet: {reseed}"
    );
    assert!(
        reseed.contains("Addam: do we have anyone on signing?"),
        "{reseed}"
    );
    assert_eq!(m[5], "lines 4");

    // Past the context limit: the look answers, then the next one starts
    // again from the seed.
    reply(f.path(), RESTART_AT);
    r.core.huddle_look(&seed, "lines 5").unwrap();
    assert!(!r.core.huddle_is_open(CALL));
    reply(f.path(), 6000);
    r.core.huddle_look(&seed, "lines 6").unwrap();
    assert_eq!(starts(f.path()), 3);
    let m = messages(f.path());
    assert!(m[7].contains("Ship the loader"), "{}", m[7]);
    assert_eq!(m[8], "lines 6");

    // Closing ends it and takes its folder.
    r.core.huddle_close(CALL);
    for _ in 0..100 {
        if runner.count() == 0 && !dir.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(runner.count(), 0);
    assert!(!dir.exists());
    r.core.close().unwrap();
}

/// §7's numbers on this implementation with the real Claude Code: a held
/// session primed with a 40-card packet, then looks, on Haiku and on
/// Sonnet low. Spends a few cents of the person's usage; run by hand:
/// `cargo test -p irori-window --test it huddle_held::live_held_session_numbers -- --ignored --nocapture`
#[test]
#[ignore = "real Claude Code: spends usage; run by hand"]
fn live_held_session_numbers() {
    use irori_core::huddle_packet::{Packet, PacketCard, render};
    use irori_core::huddle_session::{first_message, system};
    use irori_window::assistant::{HeldPool, LOOK_TIMEOUT};

    let topics = [
        "Windows signing",
        "the installer",
        "the phone build",
        "huddle captions",
        "the release notes",
        "the sync bug",
        "card weights",
        "the canvas layout",
        "Android Whisper",
        "the updater",
    ];
    let cards: Vec<PacketCard> = (0..40)
        .map(|i| {
            let agent = format!(
                "{} ({}'s agent)",
                ["Kit", "Juno", "Pip"][i % 3],
                ["Addam", "Stephen", "Mira"][i % 3]
            );
            PacketCard {
                card_ref: format!("mlg#{}", 400 + i),
                title: format!(
                    "Finish {} for the 0.{} release",
                    topics[i % topics.len()],
                    14 + i % 3
                ),
                state: ["working", "submitted", "input_required", "in_review"][i % 4].into(),
                holders: if i % 3 == 2 {
                    vec![]
                } else {
                    vec![agent.clone()]
                },
                agents: if i % 3 == 2 { vec![] } else { vec![agent] },
                updated: i as i64,
            }
        })
        .collect();
    let packet = render(
        &Packet {
            from: "MLG farming".into(),
            cards,
        },
        "",
        &[],
    );
    let looks = [
        "[12:00:05] Addam: Okay, let's start with the release. Do we have anyone working on Windows signing?",
        "[12:00:31] Stephen: I think Kit has it. Mira, are you still on the phone build?\n[12:00:40] Mira: Yes, the phone build is blocked on the review.",
        "[12:01:02] Addam: Let's keep the old updater until 0.16, no point rushing it.\n[12:01:10] Stephen: Agreed.",
        "[12:01:35] Mira: I'll take the release notes this afternoon.",
        "[12:02:01] Stephen: Juno, who's on huddle captions?",
    ];
    for (model, effort) in [("haiku", None), ("sonnet", Some("low"))] {
        let d = tempfile::tempdir().unwrap();
        let job = Job {
            dir: d.path().join(HUDDLES_DIR).join(CALL),
            data_dir: d.path().to_path_buf(),
            secret: String::new(),
            exe: PathBuf::new(),
            system: system("Juno \u{b7} Addam's assistant"),
            prompt: String::new(),
            input: Input::Text,
            model: model.into(),
            effort: effort.map(String::from),
            claude_config_dir: None,
            timeout: LOOK_TIMEOUT,
            tools: false,
            look: String::new(),
        };
        let bin = which_claude();
        let pool = HeldPool::new();
        let p = pool
            .open(bin, Ok(job), CALL, &first_message(&packet, &[]))
            .unwrap();
        println!(
            "{model}{}: priming {} ms ${:.5} {} tokens",
            effort.map(|e| format!(" {e}")).unwrap_or_default(),
            p.ms,
            p.cost_usd,
            p.context
        );
        for (i, m) in looks.iter().enumerate() {
            let msg = format!(
                "New lines since your last look:\n{}",
                irori_core::huddle_session::fence_lines(&[m.to_string()])
            );
            match pool.look(CALL, &msg) {
                Ok(l) => println!(
                    "  look {}: {} ms ${:.5} {} tokens: {}",
                    i + 1,
                    l.ms,
                    l.cost_usd,
                    l.context,
                    l.text.replace('\n', " | ")
                ),
                Err(e) => println!("  look {}: {e}", i + 1),
            }
        }
        pool.shutdown();
    }
}

/// The `claude` on PATH, for the live run.
fn which_claude() -> Result<PathBuf, String> {
    let out = std::process::Command::new("sh")
        .args(["-c", "command -v claude"])
        .output()
        .map_err(|e| e.to_string())?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if p.is_empty() {
        Err("no claude on PATH".into())
    } else {
        Ok(PathBuf::from(p))
    }
}
