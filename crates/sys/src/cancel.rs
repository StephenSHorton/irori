//! A cancellation token for blocking code, the part of Go's
//! `context.Context` the capture loop, git runs and process waits use.
//!
//! [`Cancel::cancel`] is sticky and wakes every [`Cancel::sleep`] and every
//! callback registered with [`Cancel::on_cancel`]. A child token
//! ([`Cancel::child`]) is canceled with its parent, or alone.

use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

type Callback = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct State {
    canceled: bool,
    next_id: u64,
    callbacks: Vec<(u64, Callback)>,
    children: Vec<Weak<Inner>>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    cond: Condvar,
}

/// A cloneable cancellation token. Clones share one state.
#[derive(Clone, Default)]
pub struct Cancel {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Cancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cancel")
            .field("canceled", &self.is_canceled())
            .finish()
    }
}

/// Unregisters an [`Cancel::on_cancel`] callback when dropped.
pub struct Registration {
    inner: Weak<Inner>,
    id: u64,
}

impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            let mut st = lock(&inner.state);
            st.callbacks.retain(|(id, _)| *id != self.id);
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Cancel {
    /// A token that is not canceled.
    pub fn new() -> Cancel {
        Cancel::default()
    }

    /// A token canceled with this one (or by itself).
    pub fn child(&self) -> Cancel {
        let c = Cancel::new();
        let mut st = lock(&self.inner.state);
        if st.canceled {
            drop(st);
            c.cancel();
            return c;
        }
        st.children.retain(|w| w.strong_count() > 0);
        st.children.push(Arc::downgrade(&c.inner));
        c
    }

    /// Cancels the token, runs its callbacks and those of its children.
    pub fn cancel(&self) {
        let (callbacks, children) = {
            let mut st = lock(&self.inner.state);
            if st.canceled {
                return;
            }
            st.canceled = true;
            (
                std::mem::take(&mut st.callbacks),
                std::mem::take(&mut st.children),
            )
        };
        self.inner.cond.notify_all();
        for (_, f) in callbacks {
            f();
        }
        for w in children {
            if let Some(inner) = w.upgrade() {
                Cancel { inner }.cancel();
            }
        }
    }

    /// Go `ctx.Err() != nil`.
    pub fn is_canceled(&self) -> bool {
        lock(&self.inner.state).canceled
    }

    /// Sleeps for `d` or until canceled; true when canceled.
    pub fn sleep(&self, d: Duration) -> bool {
        let deadline = Instant::now() + d;
        let mut st = lock(&self.inner.state);
        loop {
            if st.canceled {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            st = self
                .inner
                .cond
                .wait_timeout(st, deadline - now)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// Runs `f` once when the token is canceled (at once if it already
    /// is). Dropping the registration before then unregisters `f`.
    pub fn on_cancel(&self, f: impl FnOnce() + Send + 'static) -> Registration {
        let mut st = lock(&self.inner.state);
        if st.canceled {
            drop(st);
            f();
            return Registration {
                inner: Weak::new(),
                id: 0,
            };
        }
        st.next_id += 1;
        let id = st.next_id;
        st.callbacks.push((id, Box::new(f)));
        Registration {
            inner: Arc::downgrade(&self.inner),
            id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn cancel_wakes_sleep_callbacks_and_children() {
        let c = Cancel::new();
        let child = c.child();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let _r = c.on_cancel(move || {
            h.fetch_add(1, Ordering::SeqCst);
        });
        let h2 = hits.clone();
        let dropped = c.on_cancel(move || {
            h2.fetch_add(10, Ordering::SeqCst);
        });
        drop(dropped);
        let c2 = c.clone();
        let t = std::thread::spawn(move || c2.sleep(Duration::from_secs(30)));
        std::thread::sleep(Duration::from_millis(20));
        c.cancel();
        assert!(t.join().unwrap());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(child.is_canceled());
        assert!(!Cancel::new().sleep(Duration::from_millis(1)));
    }
}
