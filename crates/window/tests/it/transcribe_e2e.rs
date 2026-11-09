//! A huddle's transcript end to end with real Whisper (#656): two people
//! on loopback iroh in a space channel, Alice's fake mic playing recorded
//! speech, Bob's silent. Alice's device writes her lines down with the
//! app's own transcriber (`irori_window::voice::call_transcriber`), Bob's
//! window shows them live as captions, they're rows in the space's log
//! signed by Alice's device, the end post says there's a transcript, and
//! Bob's window reads it back under Alice's name.
//!
//! Ignored by default (it needs the speech model and a few seconds of
//! Whisper); skipped when `~/.irori/voice/ggml-base.en.bin` is missing or
//! this machine has no engine. Run it with
//! `cargo test --release -p irori-window --test it transcribe_e2e -- --ignored --nocapture`.
//! The model is linked into the test's own data dir; `~/.irori` is never
//! written.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use call::device::fake::{self, Recording};
use coord::talk::{CALL_END, CallMark};
use irori_core::{Core, Options};
use irori_window::call::CallCaptionView;
use irori_window::{Config, Window};
use store::{Filter, kinds};

const WAIT: Duration = Duration::from_secs(60);
const RATE: usize = 48_000;
const LINES: [&str; 2] = [
    "I'll take the PC build tonight.",
    "Then we can ship the Android version on Friday.",
];

fn wait(what: &str, limit: Duration, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn free_udp() -> SocketAddr {
    std::net::UdpSocket::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
}

/// One device: its Core, its window and its scratch dirs.
struct Dev {
    w: Arc<Window>,
    data: PathBuf,
    _dirs: Vec<tempfile::TempDir>,
}

impl Dev {
    fn core(&self) -> &Arc<Core> {
        &self.w.core
    }
}

/// A device whose mesh runs on loopback iroh with a LAN lookup on its own
/// port (as core's tests/it/common/mesh.rs), its options edited by `edit`
/// (given its data dir).
fn dev(edit: impl FnOnce(&Path, &mut Options)) -> Dev {
    let (d, c, dl) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    std::fs::create_dir_all(c.path().join("sessions")).unwrap();
    let lan = free_udp();
    let mut o = Options {
        data_dir: d.path().to_path_buf(),
        claude_dir: c.path().to_path_buf(),
        version: "0.6.0-test".into(),
        headless: true,
        keystore: Some(Box::new(irori_identity::keystore::FileKeystore)),
        fast_writes: true,
        no_home: true,
        ..Options::default()
    };
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = irori_core::devices::default_net_config(ident, chain, guests);
        nc.ip = irori_net::endpoint::IpMode::Loopback;
        nc.gossip = false;
        let lc = nc.lan_config.as_mut().expect("a LAN lookup");
        lc.bind = Some(lan);
        lc.targets = Some(Arc::new(Vec::new));
        lc.every = Duration::from_millis(200);
        irori_net::transport::IrohNet::new(nc)
    }));
    edit(d.path(), &mut o);
    let core = Arc::new(Core::start(o).unwrap());
    let downloads = dl.path().to_path_buf();
    let w = Arc::new(Window::new(
        core.clone(),
        Window::default_seams(&core),
        Config {
            claude_dir: c.path().to_path_buf(),
            downloads: Box::new(move || Ok(downloads.clone())),
        },
    ));
    Dev {
        w,
        data: d.path().to_path_buf(),
        _dirs: vec![d, c, dl],
    }
}

/// Alice's board, with Bob invited from a link and the words checked.
fn invited(a: &Dev, b: &Dev) -> String {
    let (a, b) = (a.core(), b.core());
    let id = a.create_board("Ship it", "ship").unwrap();
    let link = a
        .pairing_runtime()
        .clone()
        .block_on(a.invite(&id))
        .unwrap()
        .link;
    b.pairing_runtime()
        .clone()
        .block_on(b.join_invite(&link))
        .unwrap();
    wait("both screens show the words", WAIT, || {
        a.invite_status().status.state == "words" && b.invite_status().status.state == "words"
    });
    let words = b.invite_status().status.words.join(" ");
    a.confirm_invite_words(&words).unwrap();
    wait("the invite is done on both sides", WAIT, || {
        a.invite_status().status.state == "done" && b.invite_status().status.state == "done"
    });
    id
}

/// A 16-bit PCM WAV's samples as f32, with its rate and channel count.
fn read_wav(path: &Path) -> (Vec<f32>, u32, u16) {
    let b = std::fs::read(path).unwrap();
    assert!(&b[0..4] == b"RIFF" && &b[8..12] == b"WAVE", "not a WAV");
    let (mut at, mut rate, mut chans, mut bits, mut data) = (12, 0u32, 0u16, 0u16, None);
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let n = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = &b[at + 8..(at + 8 + n).min(b.len())];
        match id {
            b"fmt " => {
                chans = u16::from_le_bytes(body[2..4].try_into().unwrap());
                rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + n + (n & 1);
    }
    assert_eq!(bits, 16, "16-bit PCM only");
    let samples = data
        .expect("a data chunk")
        .chunks_exact(2 * chans as usize)
        .map(|f| i16::from_le_bytes([f[0], f[1]]) as f32 / 32768.0)
        .collect();
    (samples, rate, chans)
}

/// The speech: the job's feed.wav, or the two lines said by `say` into
/// `dir` when it's absent.
fn speech(dir: &Path) -> Vec<f32> {
    let given = PathBuf::from("/Users/stephenhorton/.claude/jobs/7adacf77/tmp/feed.wav");
    let path = if given.is_file() {
        given
    } else {
        let (aiff, wav) = (dir.join("feed.aiff"), dir.join("feed.wav"));
        let ok = std::process::Command::new("say")
            .arg("-o")
            .arg(&aiff)
            .arg(LINES.join(" [[slnc 800]] "))
            .status()
            .is_ok_and(|s| s.success())
            && std::process::Command::new("afconvert")
                .args(["-f", "WAVE", "-d", "LEI16@48000", "-c", "1"])
                .arg(&aiff)
                .arg(&wav)
                .status()
                .is_ok_and(|s| s.success());
        assert!(ok, "no feed.wav and couldn't make one with say + afconvert");
        wav
    };
    let (s, rate, chans) = read_wav(&path);
    assert_eq!((rate, chans), (RATE as u32, 1), "{}", path.display());
    println!(
        "speech: {} ({:.2} s)",
        path.display(),
        s.len() as f64 / RATE as f64
    );
    s
}

/// The ends (sample index after the last loud 10 ms block) of each run of
/// speech in `s`, runs split by 300 ms of quiet.
fn speech_ends(s: &[f32]) -> Vec<usize> {
    let loud: Vec<bool> = s
        .chunks(480)
        .map(|b| (b.iter().map(|v| v * v).sum::<f32>() / b.len() as f32).sqrt() > 0.01)
        .collect();
    let mut ends = Vec::new();
    let mut last: Option<usize> = None;
    for (i, &l) in loud.iter().enumerate() {
        if l {
            if let Some(p) = last
                && i - p > 30
            {
                ends.push((p + 1) * 480);
            }
            last = Some(i);
        }
    }
    if let Some(p) = last {
        ends.push(((p + 1) * 480).min(s.len()));
    }
    ends
}

/// When the speech began playing, set by the mic's clock thread.
type Began = Arc<Mutex<Option<Instant>>>;

/// A mic that's quiet until `go`, then plays `speech` once, then is quiet.
fn mic(speech: Arc<Vec<f32>>, go: Arc<AtomicBool>, began: Began) -> fake::Gen {
    let mut from: Option<u64> = None;
    Box::new(move |n| {
        if from.is_none() && go.load(Ordering::Relaxed) {
            from = Some(n);
            *began.lock().unwrap() = Some(Instant::now());
        }
        from.and_then(|f| speech.get((n - f) as usize).copied())
            .unwrap_or(0.0)
    })
}

fn live_peers(d: &Dev) -> usize {
    d.core()
        .call_snapshot()
        .mine
        .map_or(0, |m| m.peers.iter().filter(|p| p.phase == "live").count())
}

fn end_mark(d: &Dev, conv: &str, call_id: &str) -> Option<CallMark> {
    d.core()
        .talk_posts(&d.core().user_sender().key, conv, None, 50)
        .ok()?
        .into_iter()
        .filter_map(|p| p.call)
        .find(|c| c.call_id == call_id && c.op == CALL_END)
}

#[test]
#[ignore = "real Whisper: needs ~/.irori/voice/ggml-base.en.bin; run with --ignored --release"]
fn transcribe_e2e() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        println!("transcribe_e2e: skipped, no HOME");
        return;
    };
    let model = home.join(".irori/voice").join(voice::MODEL.file);
    if !voice::model_ready(&home.join(".irori"), &voice::MODEL) {
        println!(
            "transcribe_e2e: skipped, the speech model isn't at {}",
            model.display()
        );
        return;
    }
    if let voice::Status::Unsupported(why) = voice::status() {
        println!("transcribe_e2e: skipped, no speech engine here: {why}");
        return;
    }
    let scratch = tempfile::tempdir().unwrap();
    let speech = Arc::new(speech(scratch.path()));
    let ends = speech_ends(&speech);
    println!(
        "speech runs end at: {:?} s",
        ends.iter()
            .map(|e| format!("{:.2}", *e as f64 / RATE as f64))
            .collect::<Vec<_>>()
    );

    // Alice: the speech on her mic and the app's Whisper transcriber.
    let go = Arc::new(AtomicBool::new(false));
    let began: Began = Arc::default();
    let a = {
        let (speech, go, began) = (speech.clone(), go.clone(), began.clone());
        dev(move |data, o| {
            let voice_dir = data.join("voice");
            std::fs::create_dir_all(&voice_dir).unwrap();
            let to = voice_dir.join(voice::MODEL.file);
            // A symlink on Windows needs Developer Mode or admin rights.
            #[cfg(unix)]
            std::os::unix::fs::symlink(&model, &to).unwrap();
            #[cfg(not(unix))]
            if std::fs::hard_link(&model, &to).is_err() {
                std::fs::copy(&model, &to).unwrap();
            }
            let rec = Arc::new(Mutex::new(Recording::default()));
            o.call_audio = Some(Arc::new(move || {
                fake::devices(mic(speech.clone(), go.clone(), began.clone()), rec.clone())
            }));
            o.call_transcriber = Some(irori_window::voice::call_transcriber(data.to_path_buf()));
        })
    };
    // Bob: a silent mic, nothing to write down.
    let b = dev(|_, o| {
        let rec = Arc::new(Mutex::new(Recording::default()));
        o.call_audio = Some(Arc::new(move || {
            fake::devices(Box::new(|_| 0.0), rec.clone())
        }));
    });
    assert!(voice::model_ready(&a.data, &voice::MODEL));

    let space = invited(&a, &b);
    wait("alice hears bob read spaces", WAIT, || {
        a.core().space_not_ready(&space).unwrap().is_empty()
    });
    a.core().space_gate(&space).unwrap();
    let ch = a
        .core()
        .talk_channel_create_in(&a.core().user_sender().key, "general", "", &space)
        .unwrap()
        .id;
    let (a_dev, b_dev) = (
        a.core().device_id().to_string(),
        b.core().device_id().to_string(),
    );
    wait("the two devices reach each other", WAIT, || {
        a.core().call_reachable(&space).contains(&b_dev)
            && b.core().call_reachable(&space).contains(&a_dev)
    });

    let t = Instant::now();
    let call_id = a.core().call_start(&ch).expect("start");
    println!("call_start (model load included): {:?}", t.elapsed());
    wait("bob is rung", WAIT, || {
        b.core()
            .call_snapshot()
            .ringing
            .iter()
            .any(|r| r.call_id == call_id)
    });
    b.core().call_join(&call_id).expect("join");
    wait("connected", WAIT, || {
        live_peers(&a) == 1 && live_peers(&b) == 1
    });
    assert!(
        a.core().call_snapshot().mine.unwrap().transcribing,
        "alice's device isn't transcribing"
    );

    // Play the speech, and watch Bob's window for captions.
    go.store(true, Ordering::Relaxed);
    let mut seen: Vec<(CallCaptionView, Instant, i64)> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(90);
    let has = |seen: &[(CallCaptionView, Instant, i64)], w: &str| {
        seen.iter()
            .any(|(c, _, _)| c.text.to_lowercase().contains(w))
    };
    while !(has(&seen, "pc build") && has(&seen, "android")) {
        if Instant::now() > deadline {
            break;
        }
        if let Some(m) = b.w.call_state().mine {
            for c in m.captions {
                if !seen.iter().any(|(s, _, _)| s == &c) {
                    seen.push((c, Instant::now(), crate::common::unix_ms()));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let began = began.lock().unwrap().expect("the speech played");
    println!("captions on bob's window:");
    for (c, at, at_ms) in &seen {
        let since = at.duration_since(began).as_secs_f64();
        let after_end = ends
            .iter()
            .map(|e| *e as f64 / RATE as f64)
            .rfind(|e| *e <= since)
            .map_or(f64::NAN, |e| since - e);
        println!(
            "  [{} mine={}] {:?}  phrase {} ms long; arrived {:.2} s after the speech began, \
             {:.2} s after the end of the speech run before it, {} ms after its endMs",
            c.name,
            c.mine,
            c.text,
            c.end_ms - c.at_ms,
            since,
            after_end,
            at_ms - c.end_ms,
        );
    }
    if let Some(last) = ends.last() {
        let end = began + Duration::from_secs_f64(*last as f64 / RATE as f64);
        if let Some((_, at, _)) = seen
            .iter()
            .find(|(c, _, _)| c.text.to_lowercase().contains("android"))
        {
            println!(
                "latency, end of speech to the \"Android\" caption on bob's window: {} ms",
                at.saturating_duration_since(end).as_millis()
            );
        }
    }
    assert!(
        has(&seen, "pc build") && has(&seen, "android"),
        "bob's captions: {:?}",
        seen.iter().map(|(c, _, _)| &c.text).collect::<Vec<_>>()
    );
    for (c, _, _) in &seen {
        assert!(!c.mine && c.name != "you", "{c:?}");
        assert!(
            !c.name.contains(&a.core().person().id) && !c.name.contains(&a_dev),
            "{c:?}"
        );
    }
    let a_name = seen[0].0.name.clone();

    // Alice leaves, then Bob: his is the end post.
    a.core().call_leave().unwrap();
    wait("bob sees alice gone", WAIT, || live_peers(&b) == 0);
    b.core().call_leave().unwrap();
    let end = end_mark(&b, &ch, &call_id).expect("bob's end post");
    println!("end post: {end:?}");
    assert!(end.transcript, "{end:?}");
    wait("alice has the end post", WAIT, || {
        end_mark(&a, &ch, &call_id).is_some_and(|m| m.transcript)
    });

    // The lines are rows in the space's log, signed by Alice's device.
    let rows = a
        .core()
        .board_store(&space)
        .unwrap()
        .list(&Filter {
            kinds: vec![kinds::CALL_SEGMENT.into()],
            ..Default::default()
        })
        .unwrap();
    let actor = format!("{}{}", kinds::PERSON_ACTOR_PREFIX, a.core().person().id);
    println!("call.segment rows on alice's board log:");
    let mut row_texts = Vec::new();
    for r in &rows {
        let v: serde_json::Value = serde_json::from_slice(&r.payload).unwrap();
        println!(
            "  seg {} by {} on {}: {:?}",
            v["seg"],
            r.actor,
            &r.device_id[..8.min(r.device_id.len())],
            v["text"]
        );
        assert_eq!(r.actor, actor);
        assert_eq!(r.device_id, a_dev);
        assert_eq!(v["callId"], call_id.as_str());
        row_texts.push(v["text"].as_str().unwrap_or_default().to_lowercase());
    }
    assert!(
        row_texts.iter().any(|t| t.contains("pc build")),
        "{row_texts:?}"
    );
    assert!(
        row_texts.iter().any(|t| t.contains("android")),
        "{row_texts:?}"
    );

    // Bob's window reads the transcript back under Alice's name.
    wait("bob folds alice's lines", WAIT, || {
        b.w.talk_call_transcript(&call_id)
            .is_ok_and(|t| t.lines.len() >= rows.len())
    });
    let tr = b.w.talk_call_transcript(&call_id).unwrap();
    println!(
        "bob's transcript of #{} ({}), ended {}, {} ms:",
        tr.conv_name, tr.conv_kind, tr.ended, tr.duration_ms
    );
    for l in &tr.lines {
        println!("  [{} mine={}] {}", l.name, l.mine, l.text);
        assert_eq!((l.name.as_str(), l.mine), (a_name.as_str(), false), "{l:?}");
    }
    let all = tr
        .lines
        .iter()
        .map(|l| l.text.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(all.contains("pc build") && all.contains("android"), "{all}");
    assert!(tr.ended);
    let json = serde_json::to_string(&tr).unwrap();
    assert!(!json.contains(&a.core().person().id), "{json}");

    // The call (and its transcriber) stops before the model is freed.
    a.w.voice_shutdown();
    b.w.voice_shutdown();
    a.core().close().unwrap();
    b.core().close().unwrap();
}
