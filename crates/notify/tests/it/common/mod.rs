//! The fake card system of I:internal/notify/notify_test.go: records what
//! reached the engine and lets a test press a card.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{Config, Notifier, Toaster};

#[derive(Default)]
pub struct FakeState {
    pub cfg: Option<toast::Config>,
    pub opts: Vec<toast::Options>,
    pub posts: Vec<toast::Note>,
    pub closes: Vec<String>,
    pub live: HashMap<String, toast::Note>,
}

pub struct Fake {
    pub st: Mutex<FakeState>,
    tx: Mutex<Sender<String>>,
    pub posted: Mutex<Receiver<String>>,
}

impl Fake {
    pub fn new() -> Arc<Fake> {
        let (tx, rx) = channel();
        Arc::new(Fake {
            st: Mutex::new(FakeState::default()),
            tx: Mutex::new(tx),
            posted: Mutex::new(rx),
        })
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.st.lock().unwrap()
    }

    /// The live card id, or panics.
    pub fn card(&self, id: &str) -> toast::Note {
        let st = self.lock();
        match st.live.get(id) {
            Some(n) => n.clone(),
            None => panic!(
                "no live card {id:?}; live: {:?}",
                st.live.keys().collect::<Vec<_>>()
            ),
        }
    }

    pub fn is_live(&self, id: &str) -> bool {
        self.lock().live.contains_key(id)
    }

    /// Runs a button the way the engine does: the card goes, then run.
    pub fn press(&self, id: &str, label: &str) {
        let n = self.card(id);
        self.lock().live.remove(id);
        let a = n
            .actions
            .iter()
            .find(|a| a.label == label)
            .unwrap_or_else(|| panic!("card {id:?} has no button {label:?}"));
        if let Some(f) = &a.run {
            f();
        }
    }

    pub fn dismiss(&self, id: &str) {
        let n = self.card(id);
        self.lock().live.remove(id);
        if let Some(f) = &n.on_dismiss {
            f();
        }
    }

    pub fn click(&self, id: &str) {
        let n = self.card(id);
        self.lock().live.remove(id);
        if let Some(f) = &n.on_click {
            f();
        }
    }

    pub fn post_count(&self) -> usize {
        self.lock().posts.len()
    }

    /// Waits for a post of id.
    pub fn wait_post(&self, id: &str) {
        let rx = self.posted.lock().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(got) if got == id => return,
                Ok(_) => {}
                Err(_) => panic!("no card {id:?} posted"),
            }
        }
    }

    /// No post within d.
    pub fn no_post(&self, d: Duration) {
        let rx = self.posted.lock().unwrap();
        if let Ok(id) = rx.recv_timeout(d) {
            panic!("unexpected card {id:?}");
        }
    }

    /// Drains the posted ids.
    pub fn drain(&self) -> usize {
        let rx = self.posted.lock().unwrap();
        let mut n = 0;
        while rx.try_recv().is_ok() {
            n += 1;
        }
        n
    }

    /// Waits for id to leave the screen.
    pub fn wait_gone(&self, id: &str) {
        for _ in 0..300 {
            if !self.is_live(id) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("card {id:?} is still up");
    }

    /// Waits for the card id to have title.
    pub fn wait_title(&self, id: &str, title: &str) -> toast::Note {
        for _ in 0..300 {
            if let Some(n) = self.lock().live.get(id)
                && n.title == title
            {
                return n.clone();
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("card {id:?} never read {title:?}");
    }
}

impl Toaster for Fake {
    fn configure(&self, c: toast::Config) {
        self.lock().cfg = Some(c);
    }
    fn set_options(&self, o: toast::Options) {
        let mut st = self.lock();
        if !o.enabled {
            st.live.clear();
        }
        st.opts.push(o);
    }
    fn post(&self, n: toast::Note) {
        let id = n.id.clone();
        {
            let mut st = self.lock();
            st.posts.push(n.clone());
            st.live.insert(id.clone(), n);
        }
        let _ = self.tx.lock().unwrap().send(id);
    }
    fn close(&self, id: &str) {
        let mut st = self.lock();
        st.closes.push(id.into());
        st.live.remove(id);
    }
}

pub fn notifier(f: &Arc<Fake>) -> Notifier {
    Notifier::new(Config {
        toaster: Some(f.clone()),
        ..Config::default()
    })
}

pub fn wait_for(what: &str, ok: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for: {what}");
}
