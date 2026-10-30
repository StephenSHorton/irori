//! A bounded subscriber queue with Go buffered-channel semantics
//! (architecture.md §2: "a small per-subscriber bounded queue type in
//! `gocompat::sub`").
//!
//! Go's subscription code relies on three things a buffered `chan` gives it:
//! `len(ch)` is readable, a send can be skipped when the buffer is full so
//! writers never block, and `close(ch)` lets the reader drain what is
//! buffered before it sees the end. [`Queue`] is exactly that, readable from
//! a blocking thread ([`Queue::recv`]) or a task ([`Queue::recv_async`]).
//! What a full queue means (drop, or the store's reserved resync slot) is
//! the caller's rule, decided under the caller's own lock as in Go.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

struct State<T> {
    buf: VecDeque<T>,
    closed: bool,
    wakers: Vec<Waker>,
}

/// A bounded FIFO with a fixed capacity, like a Go `chan T` made with
/// `make(chan T, cap)`.
pub struct Queue<T> {
    cap: usize,
    state: Mutex<State<T>>,
    ready: Condvar,
}

/// What [`Queue::try_recv`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum TryRecv<T> {
    Item(T),
    /// Open and empty.
    Empty,
    /// Closed and drained.
    Closed,
}

impl<T> std::fmt::Debug for Queue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.lock();
        f.debug_struct("Queue")
            .field("cap", &self.cap)
            .field("len", &s.buf.len())
            .field("closed", &s.closed)
            .finish()
    }
}

impl<T> Queue<T> {
    /// A queue holding at most `cap` items (`cap` >= 1).
    pub fn new(cap: usize) -> Queue<T> {
        assert!(cap >= 1, "a subscriber queue needs room for one item");
        Queue {
            cap,
            state: Mutex::new(State {
                buf: VecDeque::with_capacity(cap),
                closed: false,
                wakers: Vec::new(),
            }),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<T>> {
        // No code path panics while holding it with the state half-changed.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Go `cap(ch)`.
    pub fn cap(&self) -> usize {
        self.cap
    }

    /// Go `len(ch)`: items buffered and not yet received.
    pub fn len(&self) -> usize {
        self.lock().buf.len()
    }

    /// No item is buffered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether [`close`](Self::close) was called.
    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// A non-blocking send: false (and `v` dropped) when the queue is full
    /// or closed. Never blocks the writer.
    pub fn push(&self, v: T) -> bool {
        let mut s = self.lock();
        if s.closed || s.buf.len() >= self.cap {
            return false;
        }
        s.buf.push_back(v);
        let wakers = std::mem::take(&mut s.wakers);
        drop(s);
        self.ready.notify_one();
        for w in wakers {
            w.wake();
        }
        true
    }

    /// Go `close(ch)`: no more sends; readers drain what is buffered, then
    /// see the end. Closing twice is a no-op (Go would panic; callers here
    /// close from one place under their own lock).
    pub fn close(&self) {
        let mut s = self.lock();
        s.closed = true;
        let wakers = std::mem::take(&mut s.wakers);
        drop(s);
        self.ready.notify_all();
        for w in wakers {
            w.wake();
        }
    }

    /// Receive without waiting.
    pub fn try_recv(&self) -> TryRecv<T> {
        let mut s = self.lock();
        match s.buf.pop_front() {
            Some(v) => TryRecv::Item(v),
            None if s.closed => TryRecv::Closed,
            None => TryRecv::Empty,
        }
    }

    /// Go `v, ok := <-ch` on a blocking thread: `None` once closed and
    /// drained.
    pub fn recv(&self) -> Option<T> {
        let mut s = self.lock();
        loop {
            if let Some(v) = s.buf.pop_front() {
                return Some(v);
            }
            if s.closed {
                return None;
            }
            s = self.ready.wait(s).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// [`recv`](Self::recv) giving up after `timeout`: `Err(())` on timeout.
    #[allow(clippy::result_unit_err)]
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<T>, ()> {
        let deadline = Instant::now() + timeout;
        let mut s = self.lock();
        loop {
            if let Some(v) = s.buf.pop_front() {
                return Ok(Some(v));
            }
            if s.closed {
                return Ok(None);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(());
            }
            s = self
                .ready
                .wait_timeout(s, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// [`recv`](Self::recv) for async code.
    pub fn recv_async(&self) -> Recv<'_, T> {
        Recv { q: self }
    }
}

/// The future [`Queue::recv_async`] returns.
pub struct Recv<'a, T> {
    q: &'a Queue<T>,
}

impl<T> Future for Recv<'_, T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
        let mut s = self.q.lock();
        if let Some(v) = s.buf.pop_front() {
            return Poll::Ready(Some(v));
        }
        if s.closed {
            return Poll::Ready(None);
        }
        if !s.wakers.iter().any(|w| w.will_wake(cx.waker())) {
            s.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn full_queue_refuses_without_blocking() {
        let q = Queue::new(2);
        assert!(q.push(1));
        assert!(q.push(2));
        assert!(!q.push(3));
        assert_eq!(q.len(), 2);
        assert_eq!(q.recv(), Some(1));
        assert!(q.push(4));
        assert_eq!(q.try_recv(), TryRecv::Item(2));
        assert_eq!(q.try_recv(), TryRecv::Item(4));
        assert_eq!(q.try_recv(), TryRecv::Empty);
    }

    #[test]
    fn close_drains_then_ends() {
        let q = Queue::new(4);
        q.push("a");
        q.close();
        assert!(!q.push("b"));
        assert_eq!(q.recv(), Some("a"));
        assert_eq!(q.recv(), None);
        assert_eq!(q.try_recv(), TryRecv::Closed);
    }

    #[test]
    fn recv_wakes_on_push_and_close() {
        let q = Arc::new(Queue::new(1));
        let q2 = q.clone();
        let h = std::thread::spawn(move || (q2.recv(), q2.recv()));
        std::thread::sleep(Duration::from_millis(20));
        q.push(7);
        std::thread::sleep(Duration::from_millis(20));
        q.close();
        assert_eq!(h.join().unwrap(), (Some(7), None));
        assert_eq!(q.recv_timeout(Duration::from_millis(1)), Ok(None));
        let open: Queue<u8> = Queue::new(1);
        assert_eq!(open.recv_timeout(Duration::from_millis(5)), Err(()));
    }
}
