//! Go `http.Server`'s per-connection deadlines (I:internal/api/server.go
//! Serve; surface.md §2.4): ReadHeaderTimeout 5 s, ReadTimeout 10 s,
//! WriteTimeout 30 s, IdleTimeout 60 s, which the attach and events
//! streams lift for themselves (Go `ResponseController.SetReadDeadline`
//! and `SetWriteDeadline` with a zero time).
//!
//! hyper has only a header-read timer, so the connection's socket is
//! wrapped in [`TimedIo`], whose reads and writes fail with `TimedOut` past
//! the deadlines in its [`Deadlines`]; hyper then drops the connection, as
//! Go's server does when a deadline passes. The deadlines move as Go's do:
//!
//! - a new connection, and the first byte after an idle wait, start a
//!   request: read by start + 5 s (the header);
//! - once the head is read ([`Deadlines::request_started`]): read by start
//!   + 10 s (the body), write by now + 30 s;
//! - once the body is read ([`Deadlines::body_read`]): no read deadline
//!   while the handler runs (hyper may poll the socket for a close);
//! - a stream ([`Deadlines::lift`]): no read or write deadline;
//! - once a response is written ([`Deadlines::response_done`]): idle, read
//!   by now + 60 s (the next request's first byte), no write deadline.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use bytes::Bytes;
use hyper::body::{Body, Frame, SizeHint};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::time::{Instant, Sleep};

pub const READ_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The timeouts one server uses (tests shorten them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    pub read_header: Duration,
    pub read: Duration,
    pub write: Duration,
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Timeouts {
        Timeouts {
            read_header: READ_HEADER_TIMEOUT,
            read: READ_TIMEOUT,
            write: WRITE_TIMEOUT,
            idle: IDLE_TIMEOUT,
        }
    }
}

struct State {
    read: Option<Instant>,
    write: Option<Instant>,
    /// Waiting for the next request's first byte.
    idle: bool,
    /// When the current request's first byte came (or the connection).
    started: Instant,
    /// The read parked without a deadline timer (none was set when it
    /// parked); woken when one is set, so its timer starts.
    reader: Option<Waker>,
}

/// One connection's read and write deadlines.
pub(crate) struct Deadlines {
    t: Timeouts,
    s: Mutex<State>,
}

impl Deadlines {
    /// A new connection: its first request's header deadline runs now.
    pub fn new(t: Timeouts) -> Arc<Deadlines> {
        let now = Instant::now();
        Arc::new(Deadlines {
            t,
            s: Mutex::new(State {
                read: Some(now + t.read_header),
                write: None,
                idle: false,
                started: now,
                reader: None,
            }),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.s.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bytes arrived: after an idle wait they start a request.
    fn on_data(&self) {
        let mut s = self.lock();
        if s.idle {
            let now = Instant::now();
            s.idle = false;
            s.started = now;
            s.read = Some(now + self.t.read_header);
        }
    }

    /// The request's head is read: the body must be in by start + 10 s, and
    /// the response out within 30 s.
    pub fn request_started(&self) {
        let mut s = self.lock();
        s.idle = false;
        s.read = Some(s.started + self.t.read);
        s.write = Some(Instant::now() + self.t.write);
    }

    /// The request is read: the handler's run is bounded by the write
    /// deadline only.
    pub fn body_read(&self) {
        self.lock().read = None;
    }

    /// A stream: no deadlines for the rest of the connection's request.
    pub fn lift(&self) {
        let mut s = self.lock();
        s.read = None;
        s.write = None;
    }

    /// The response is written: wait at most 60 s for the next request.
    ///
    /// hyper already polled the socket for the next request (or a close)
    /// while the handler ran, with no read deadline, so that read is parked
    /// without a timer: wake it to arm the idle one. Without the wake an
    /// idle keep-alive connection was never closed.
    pub fn response_done(&self) {
        let reader = {
            let mut s = self.lock();
            s.idle = true;
            s.read = Some(Instant::now() + self.t.idle);
            s.write = None;
            s.reader.take()
        };
        if let Some(w) = reader {
            w.wake();
        }
    }

    /// The read deadline for a read that found no data; with none, the
    /// read is parked (in the same lock) for [`response_done`] to wake.
    ///
    /// [`response_done`]: Deadlines::response_done
    fn read_deadline_or_park(&self, w: &Waker) -> Option<Instant> {
        let mut s = self.lock();
        if s.read.is_none() && !s.reader.as_ref().is_some_and(|old| old.will_wake(w)) {
            s.reader = Some(w.clone());
        }
        s.read
    }

    fn write_deadline(&self) -> Option<Instant> {
        self.lock().write
    }
}

/// A timer for the deadline it was armed with.
type Armed = Option<(Instant, Pin<Box<Sleep>>)>;

/// Whether `deadline` has passed; when it has not, a timer wakes `cx` at
/// it. A deadline moved meanwhile re-arms the timer.
fn passed(deadline: Option<Instant>, armed: &mut Armed, cx: &mut Context<'_>) -> bool {
    let Some(dl) = deadline else {
        *armed = None;
        return false;
    };
    if Instant::now() >= dl {
        return true;
    }
    if !matches!(armed, Some((at, _)) if *at == dl) {
        *armed = Some((dl, Box::pin(tokio::time::sleep_until(dl))));
    }
    let (_, sleep) = armed.as_mut().expect("armed above");
    sleep.as_mut().poll(cx).is_ready()
}

fn timed_out(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("api: {what} deadline passed"),
    )
}

/// A socket whose reads and writes honour [`Deadlines`].
pub(crate) struct TimedIo {
    io: TcpStream,
    d: Arc<Deadlines>,
    read_timer: Armed,
    write_timer: Armed,
}

impl TimedIo {
    pub fn new(io: TcpStream, d: Arc<Deadlines>) -> TimedIo {
        TimedIo {
            io,
            d,
            read_timer: None,
            write_timer: None,
        }
    }

    fn write_pending<T>(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<T>> {
        if passed(self.d.write_deadline(), &mut self.write_timer, cx) {
            return Poll::Ready(Err(timed_out("write")));
        }
        Poll::Pending
    }
}

impl AsyncRead for TimedIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        match Pin::new(&mut this.io).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if buf.filled().len() > before {
                    this.d.on_data();
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => {
                let dl = this.d.read_deadline_or_park(cx.waker());
                if passed(dl, &mut this.read_timer, cx) {
                    return Poll::Ready(Err(timed_out("read")));
                }
                Poll::Pending
            }
        }
    }
}

impl AsyncWrite for TimedIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.io).poll_write(cx, buf) {
            Poll::Pending => this.write_pending(cx),
            r => r,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match Pin::new(&mut this.io).poll_flush(cx) {
            Poll::Pending => this.write_pending(cx),
            r => r,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        match Pin::new(&mut this.io).poll_write_vectored(cx, bufs) {
            Poll::Pending => this.write_pending(cx),
            r => r,
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }
}

/// A response body that marks the connection idle once it is written (or
/// dropped unfinished).
pub(crate) struct DoneBody<B> {
    inner: B,
    d: Option<Arc<Deadlines>>,
}

impl<B> DoneBody<B> {
    pub fn new(inner: B, d: Arc<Deadlines>) -> DoneBody<B> {
        DoneBody { inner, d: Some(d) }
    }

    fn done(&mut self) {
        if let Some(d) = self.d.take() {
            d.response_done();
        }
    }
}

impl<B> Drop for DoneBody<B> {
    fn drop(&mut self) {
        self.done();
    }
}

impl<B: Body<Data = Bytes> + Unpin> Body for DoneBody<B> {
    type Data = Bytes;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, B::Error>>> {
        let this = self.get_mut();
        let r = Pin::new(&mut this.inner).poll_frame(cx);
        if let Poll::Ready(None) = r {
            this.done();
        }
        r
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
