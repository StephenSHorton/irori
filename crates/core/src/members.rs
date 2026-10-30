//! Bound Grok members (I:internal/core/members.go; surface.md §2.12).
//!
//! A member is an `irori mcp` process bound with `POST /v1/members`. Its
//! secret lives only in memory: an Irori restart makes every `irori mcp`
//! join again. While a member stream is open, and for
//! [`inbox::REATTACH_GRACE`] after it closes, the member has a synthetic
//! presence card (agent `grok`) in [`crate::Core::local_agents`].

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use gocompat::time::GoTime;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// A bound Grok member (Go `api.Member`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Member {
    /// `<device>/grok-<id>`.
    #[serde(rename = "memberKey")]
    pub key: String,
    #[serde(rename = "memberSecret")]
    pub secret: String,
    pub name: String,
}

struct Rec {
    key: String,
    name: String,
    secret: String,
    joined_at: GoTime,
    /// Open member streams, counted per stream (2 while a supersede settles).
    open: u32,
    /// Join, or the last stream open or close.
    last_seen: GoTime,
    /// A member stream has opened at least once.
    streamed: bool,
}

/// The member registry (Go `members`).
pub struct Members {
    device_id: String,
    grace: Duration,
    now: Box<dyn Fn() -> GoTime + Send + Sync>,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    by_key: BTreeMap<String, Rec>,
    /// secret → key.
    by_secret: BTreeMap<String, String>,
}

fn rand_hex(n: usize) -> Result<String, String> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(hex::encode(b))
}

impl Members {
    pub fn new(device_id: &str) -> Members {
        Members {
            device_id: device_id.to_string(),
            grace: inbox::REATTACH_GRACE,
            now: Box::new(presence::now_local),
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Tests: the clock and the grace.
    pub fn with_clock(
        device_id: &str,
        grace: Duration,
        now: impl Fn() -> GoTime + Send + Sync + 'static,
    ) -> Members {
        Members {
            device_id: device_id.to_string(),
            grace,
            now: Box::new(now),
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stable for a client session id, so a rejoin (an `irori mcp` restart
    /// for the same Grok session) keeps its key, across restarts too.
    pub fn member_id(&self, session: &str) -> Result<String, String> {
        if session.is_empty() {
            return rand_hex(6);
        }
        let mut h = Sha256::new();
        h.update(self.device_id.as_bytes());
        h.update([0u8]);
        h.update(session.as_bytes());
        Ok(hex::encode(&h.finalize()[..6]))
    }

    /// Binds a member: the same non-empty session gets the same key; every
    /// join mints a new secret and the old one stops working.
    pub fn join(&self, name: &str, session: &str) -> Result<Member, String> {
        let id = self.member_id(session)?;
        let secret = rand_hex(24)?;
        let key = format!("{}/grok-{id}", self.device_id);
        let mut g = self.lock();
        let now = (self.now)();
        if let Some(old) = g.by_key.get(&key).map(|r| r.secret.clone()) {
            // A rejoin retires the old secret.
            g.by_secret.remove(&old);
        } else {
            g.by_key.insert(
                key.clone(),
                Rec {
                    key: key.clone(),
                    name: String::new(),
                    secret: String::new(),
                    joined_at: now,
                    open: 0,
                    last_seen: now,
                    streamed: false,
                },
            );
        }
        // A join alone lists no card (and a rejoin doesn't extend the
        // grace): the card shows while a stream is open and for the grace
        // after it closes.
        let rec = g.by_key.get_mut(&key).expect("just inserted");
        rec.name = name.to_string();
        rec.secret = secret.clone();
        g.by_secret.insert(secret.clone(), key.clone());
        Ok(Member {
            key,
            secret,
            name: name.to_string(),
        })
    }

    /// Go `bySecretLookup`.
    pub fn by_secret(&self, secret: &str) -> Option<Member> {
        if secret.is_empty() {
            return None;
        }
        let g = self.lock();
        let key = g.by_secret.get(secret)?;
        let r = g.by_key.get(key)?;
        Some(Member {
            key: r.key.clone(),
            secret: r.secret.clone(),
            name: r.name.clone(),
        })
    }

    /// A member stream for `key` opened (true) or closed (false).
    pub fn stream(&self, key: &str, open: bool) {
        let now = (self.now)();
        let mut g = self.lock();
        let Some(r) = g.by_key.get_mut(key) else {
            return;
        };
        if open {
            r.open += 1;
            r.streamed = true;
        } else if r.open > 0 {
            r.open -= 1;
        }
        r.last_seen = now;
    }

    /// How many streams `key` has open (0 for an unknown key).
    pub fn open_count(&self, key: &str) -> u32 {
        self.lock().by_key.get(key).map_or(0, |r| r.open)
    }

    /// The synthetic cards: members with an open stream, or whose last
    /// stream closed less than the grace ago, sorted by key.
    pub fn cards(&self) -> Vec<presence::AgentCard> {
        let now = (self.now)();
        let g = self.lock();
        let mut out = Vec::new();
        for r in g.by_key.values() {
            if r.open == 0 && (!r.streamed || since(&now, &r.last_seen) >= self.grace) {
                continue;
            }
            out.push(presence::AgentCard {
                key: r.key.clone(),
                device_id: self.device_id.clone(),
                agent: "grok".into(),
                session_id: r.key[self.device_id.len() + 1..].to_string(),
                name: r.name.clone(),
                status: "idle".into(),
                session_kind: "member".into(),
                peer_features: vec![],
                started_at: r.joined_at,
                updated_at: r.last_seen,
                last_activity: r.last_seen,
                ..Default::default()
            });
        }
        // by_key is a BTreeMap: already sorted by key.
        out
    }
}

/// `now.Sub(t)`, clamped at zero.
fn since(now: &GoTime, t: &GoTime) -> Duration {
    let (a, b) = (now.to_system(), t.to_system());
    a.duration_since(b).unwrap_or(Duration::ZERO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicI64, Ordering};

    const DEV: &str = "0123456789abcdef0123456789abcdef";

    fn clock() -> (Arc<AtomicI64>, impl Fn() -> GoTime + Send + Sync + 'static) {
        let ms = Arc::new(AtomicI64::new(1_700_000_000_000));
        let m = ms.clone();
        (ms, move || GoTime::unix_milli(m.load(Ordering::SeqCst)))
    }

    #[test]
    fn same_session_same_key_new_secret() {
        let (_, now) = clock();
        let m = Members::with_clock(DEV, Duration::from_secs(120), now);
        let a = m.join("grok", "s1").unwrap();
        let b = m.join("grok2", "s1").unwrap();
        assert_eq!(a.key, b.key);
        assert!(a.key.starts_with(&format!("{DEV}/grok-")));
        assert_eq!(a.key.len(), DEV.len() + 1 + 5 + 12);
        assert_ne!(a.secret, b.secret);
        assert_eq!(a.secret.len(), 48);
        assert!(
            m.by_secret(&a.secret).is_none(),
            "a rejoin retires the old secret"
        );
        assert_eq!(m.by_secret(&b.secret).unwrap().name, "grok2");
        assert!(m.by_secret("").is_none());
        let c = m.join("x", "").unwrap();
        let d = m.join("x", "").unwrap();
        assert_ne!(c.key, d.key, "no session: a new member each time");
    }

    #[test]
    fn card_while_open_and_for_the_grace() {
        let (ms, now) = clock();
        let m = Members::with_clock(DEV, Duration::from_secs(120), now);
        let a = m.join("grok", "s1").unwrap();
        assert!(m.cards().is_empty(), "a join alone lists no card");
        m.stream(&a.key, true);
        m.stream(&a.key, true);
        m.stream(&a.key, false);
        let cs = m.cards();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].agent, "grok");
        assert_eq!(cs[0].session_kind, "member");
        assert_eq!(cs[0].session_id, a.key[DEV.len() + 1..]);
        m.stream(&a.key, false);
        ms.fetch_add(119_000, Ordering::SeqCst);
        assert_eq!(m.cards().len(), 1, "inside the grace");
        ms.fetch_add(1_000, Ordering::SeqCst);
        assert!(m.cards().is_empty(), "the grace is over");
        m.stream("nobody", true);
    }
}
