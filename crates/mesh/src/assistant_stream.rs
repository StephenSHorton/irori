//! The assistant's reply on the private mesh (one-assistant §5, card 7).
//! A device that asked opens a stream labelled `assistant/<turn>` to the
//! assistant's computer, writes one byte, and reads frames until the
//! server finishes: `u8 type ‖ u32 BE len ‖ body` (inline-terminals §6.4's
//! framing). The stream is a view: a stream that drops, or never starts,
//! changes nothing, since the `said` row is the reply.
//!
//! Served only to the person's certified devices, as `blob/` is. What the
//! turns are comes from [`crate::Options::assistant_turns`].

use std::sync::mpsc::Receiver;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::node::Node;
use crate::transport::Stream;

pub const ASSISTANT_LABEL_PREFIX: &str = "assistant/";

/// Frame types. A `delta` and a `tool` carry text; `changed` too; `done`
/// and `error` end the turn.
pub const T_DELTA: u8 = 1;
pub const T_TOOL: u8 = 2;
pub const T_CHANGED: u8 = 3;
pub const T_DONE: u8 = 4;
pub const T_ERROR: u8 = 5;

/// The most one frame's body holds.
pub const MAX_FRAME: usize = 1 << 20;
/// The longest turn id served.
const MAX_TURN: usize = 128;

/// A running turn's frames, as [`crate::Options::assistant_turns`] hands
/// them out: each item is one whole frame ([`frame`]). The channel
/// closing ends the stream.
pub type TurnFrames = Receiver<Vec<u8>>;

/// One framed event.
pub fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let body = &body[..body.len().min(MAX_FRAME)];
    let mut out = Vec::with_capacity(5 + body.len());
    out.push(kind);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// Whether `turn` is a plausible turn id to put in a label.
fn valid_turn(turn: &str) -> bool {
    !turn.is_empty()
        && turn.len() <= MAX_TURN
        && turn
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

/// Reads one frame; `None` at a clean end or any failure.
pub async fn read_frame(st: &mut Stream) -> Option<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    st.read_exact(&mut head).await.ok()?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX_FRAME {
        return None;
    }
    let mut body = vec![0u8; len];
    st.read_exact(&mut body).await.ok()?;
    Some((head[0], body))
}

impl Node {
    /// Opens the stream of `turn` on `dev`, the assistant's computer. An
    /// `Err` is a device not connected or a stream that would not open:
    /// the caller waits for the `said` row instead.
    pub async fn open_assistant_stream(&self, dev: &str, turn: &str) -> Result<Stream, String> {
        if !valid_turn(turn) {
            return Err("assistant: bad turn".into());
        }
        let conn = self
            .conn_to(dev)
            .ok_or_else(|| "assistant: that device is not connected".to_string())?;
        let st = conn
            .raw()
            .open_stream(&format!("{ASSISTANT_LABEL_PREFIX}{turn}"))
            .await
            .map_err(|e| e.to_string())?;
        let mut st: Stream = Box::new(tokio::io::join(st.recv, st.send));
        st.write_all(&[0]).await.map_err(|e| e.to_string())?;
        st.flush().await.map_err(|e| e.to_string())?;
        Ok(st)
    }

    /// Answers one `assistant/<turn>` stream from a certified device: its
    /// frames until the turn ends. A turn that is not running (or this
    /// device has no turns) is a finish with nothing written.
    pub(crate) async fn serve_assistant(&self, label: &str, st: &mut Stream) {
        let Some(turn) = label.strip_prefix(ASSISTANT_LABEL_PREFIX) else {
            return;
        };
        let Some(open) = self.o.assistant_turns.clone() else {
            return;
        };
        if !valid_turn(turn) {
            return;
        }
        let mut one = [0u8; 1];
        if st.read_exact(&mut one).await.is_err() {
            return;
        }
        let turn = turn.to_string();
        let Some(rx) = open(&turn) else {
            return;
        };
        // The receiver is a std channel: a thread feeds a tokio one.
        let (tx, mut out) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        std::thread::spawn(move || {
            while let Ok(f) = rx.recv() {
                if tx.send(f).is_err() {
                    break; // the stream is gone: dropping rx lets the hub forget it
                }
            }
        });
        let _busy = self.activity().begin();
        while let Some(f) = out.recv().await {
            if st.write_all(&f).await.is_err() || st.flush().await.is_err() {
                break;
            }
        }
    }
}
