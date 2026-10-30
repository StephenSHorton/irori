//! Board-log pulls on `board/<boardId>` streams (mesh.md §2.3 "Board";
//! I:internal/mesh/boards.go). Stage 1 ran them on the private mesh
//! connection between one person's devices; stage 2 runs them only on
//! that board's `irori/board/1` connections ([`crate::boardmesh`]), so a
//! board's rows move only between its members' devices and the private
//! connection never carries one.
//!
//! A pull: the puller writes `u32 BE len || {"vector":{...}}`; the server
//! answers with `u32 BE len || {"events":[Row]}` chunks of what the puller
//! lacks, at most 2000 rows in all, then finishes the stream.

use std::sync::Arc;

use gocompat::json::{self, Out, Schema};
use serde::Serialize;
use store::{ErrorKind, Event, Store, Vector};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::frames::{MAX_BATCH_ROWS, MAX_MESSAGE, Row, batches, decode_chunk};
use crate::transport::{Conn, Stream};

pub const BOARD_LABEL_PREFIX: &str = "board/";
const MAX_BOARD_REQUEST: usize = 1 << 20;
const BOARD_PULL_ROWS: usize = 4 * MAX_BATCH_ROWS;

#[derive(Serialize)]
struct BoardRequest<'a> {
    vector: &'a Vector,
}

#[derive(Serialize)]
struct BoardChunk<'a> {
    events: &'a [Row],
}

/// A chunk read error: the stream ended (Go `io.EOF` /
/// `io.ErrUnexpectedEOF`) or something else went wrong.
#[derive(Debug)]
pub(crate) enum ChunkErr {
    Eof,
    Other(String),
}

/// Go `writeChunk`: `u32 BE len || body`.
pub(crate) async fn write_chunk<W: AsyncWrite + Unpin + ?Sized>(
    w: &mut W,
    body: &[u8],
) -> std::io::Result<()> {
    let mut msg = (body.len() as u32).to_be_bytes().to_vec();
    msg.extend_from_slice(body);
    w.write_all(&msg).await?;
    w.flush().await
}

/// Go `readChunk`: a length of 0 or over `limit` is an error.
pub(crate) async fn read_chunk<R: AsyncRead + Unpin + ?Sized>(
    r: &mut R,
    limit: usize,
) -> Result<Vec<u8>, ChunkErr> {
    let eof = |e: std::io::Error| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ChunkErr::Eof
        } else {
            ChunkErr::Other(e.to_string())
        }
    };
    let mut hdr = [0u8; 4];
    r.read_exact(&mut hdr).await.map_err(eof)?;
    let size = u32::from_be_bytes(hdr) as usize;
    if size == 0 || size > limit {
        return Err(ChunkErr::Other("mesh: board chunk too large".into()));
    }
    let mut body = vec![0u8; size];
    r.read_exact(&mut body).await.map_err(eof)?;
    Ok(body)
}

fn decode_events(body: &[u8]) -> Result<Vec<Event>, String> {
    Ok(decode_chunk(body)?.iter().map(Row::event).collect())
}

fn decode_vector(body: &[u8]) -> Option<Vector> {
    let schema = Schema::object(vec![("vector", Schema::Map(Box::new(Schema::Int(64))))]);
    let o = json::unmarshal(body, &schema).ok()?;
    let mut v = Vector::new();
    if let Some(Out::Map(Some(m))) = o.get("vector") {
        for (k, x) in m {
            if let Out::Int(n) = x {
                v.insert(k.clone(), *n);
            }
        }
    }
    Some(v)
}

/// Go `serveBoard`'s reply: what `log` holds past the puller's vector, at
/// most 2000 rows. The caller has already decided the peer may pull this
/// board (its board connection's admission).
pub(crate) async fn serve_pull(log: Arc<Store>, st: &mut Stream) {
    let Ok(body) = read_chunk(st, MAX_BOARD_REQUEST).await else {
        return;
    };
    let Some(mut vec) = decode_vector(&body) else {
        return;
    };
    let mut sent = 0;
    while sent < BOARD_PULL_ROWS {
        let (l, v) = (log.clone(), vec.clone());
        let evs =
            match tokio::task::spawn_blocking(move || l.missing(&v, MAX_BATCH_ROWS as i64)).await {
                Ok(Ok(evs)) if !evs.is_empty() => evs,
                _ => return,
            };
        for f in batches(&evs, false) {
            let body = json::marshal(&BoardChunk { events: &f.events }).unwrap_or_default();
            if write_chunk(st, &body).await.is_err() {
                return;
            }
        }
        for ev in &evs {
            let e = vec.entry(ev.device_id.clone()).or_insert(0);
            *e = (*e).max(ev.seq);
        }
        sent += evs.len();
    }
}

/// Go `pullBoard`: asks the peer on `conn` for the rows of board `id` that
/// `log` lacks and inserts them. Returns how many rows came.
pub(crate) async fn pull_board(
    conn: &dyn Conn,
    id: &str,
    log: Arc<Store>,
) -> Result<usize, String> {
    let vec = log.vector().map_err(|e| e.to_string())?;
    let st = conn
        .open_stream(&format!("{BOARD_LABEL_PREFIX}{id}"))
        .await
        .map_err(|e| e.to_string())?;
    let mut stream: Stream = Box::new(tokio::io::join(st.recv, st.send));
    let req = json::marshal(&BoardRequest { vector: &vec }).map_err(|e| e.to_string())?;
    write_chunk(&mut stream, &req)
        .await
        .map_err(|e| e.to_string())?;
    let mut evs: Vec<Event> = Vec::new();
    loop {
        match read_chunk(&mut stream, MAX_MESSAGE).await {
            Ok(body) => evs.extend(decode_events(&body)?),
            Err(ChunkErr::Eof) => break,
            Err(ChunkErr::Other(e)) => return Err(e),
        }
        if evs.len() > BOARD_PULL_ROWS {
            break; // a peer that sends more than asked: what came is enough
        }
    }
    let _ = stream.shutdown().await;
    let n = evs.len();
    tokio::task::spawn_blocking(move || insert_board_rows(&log, evs))
        .await
        .map_err(|e| e.to_string())??;
    Ok(n)
}

/// Go `insertBoardRows`: device by device, trying again the runs refused
/// for "no cert yet" while another run got in.
pub fn insert_board_rows(st: &Store, evs: Vec<Event>) -> Result<(), String> {
    let mut order: Vec<String> = Vec::new();
    let mut runs: std::collections::BTreeMap<String, Vec<Event>> = Default::default();
    for ev in evs {
        if !runs.contains_key(&ev.device_id) {
            order.push(ev.device_id.clone());
        }
        runs.entry(ev.device_id.clone()).or_default().push(ev);
    }
    let mut last: Option<String> = None;
    while !order.is_empty() {
        let mut wait = Vec::new();
        let mut moved = false;
        for dev in &order {
            let run = runs.get_mut(dev).expect("every ordered device has a run");
            run.sort_by_key(|e| e.seq);
            let (applied, r) = st.insert_batch(run);
            if applied > 0 {
                moved = true;
            }
            match r {
                Ok(()) => {}
                Err(e) if e.is(ErrorKind::NoCert) => {
                    run.drain(..applied);
                    wait.push(dev.clone());
                }
                Err(e) if e.is(ErrorKind::Closed) => return Ok(()),
                // A fork, an HLC refusal, a row ahead of the clock: the store
                // recorded what it must; the next pull starts from its vector.
                Err(e) => last = Some(e.to_string()),
            }
        }
        if !moved {
            break;
        }
        order = wait;
    }
    match last {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
