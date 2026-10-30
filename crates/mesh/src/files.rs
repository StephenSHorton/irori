//! Attachment fetch over blob streams (mesh.md §2.3 "Blob";
//! I:internal/mesh/files.go). Each fetch is its own stream, labelled
//! `blob/<16 hex>`. The fetcher writes `u32 BE len || {"id","sha256",
//! "offset","len"}` (at most 4 KiB), then reads raw bytes until the server
//! finishes. The server sends only hashes an attach.offered row names and
//! that it holds; anything else is a finish with nothing written.

use std::io::{SeekFrom, Write};
use std::sync::Arc;

use blob::{ErrorKind as BlobKind, FileRef};
use gocompat::json::{self, Out, Schema};
use serde::Serialize;
use store::{Event, Filter, MAX_LIST_LIMIT, Store};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::node::Node;
use crate::transport::{Conn, PeerConn, Stream};

pub const BLOB_LABEL_PREFIX: &str = "blob/";
const MAX_BLOB_REQUEST: usize = 4 << 10;
const BLOB_CHUNK: usize = 16 << 10;
/// How many times one holder is asked before the next: a stream cut
/// mid-file resumes by offset.
const FETCH_TRIES: usize = 3;

const MISMATCH: &str = "blob: the bytes don't match the attachment's length and hash";

/// An attach.offered row's payload.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FileOffer {
    pub sha256: String,
    pub size: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mime: String,
}

#[derive(Serialize)]
struct BlobRequest<'a> {
    id: &'a str,
    sha256: &'a str,
    offset: i64,
    len: i64,
}

/// Why [`Node::fetch_file`] failed, with Go's texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Go `ErrNoFiles`: the Node was built without an attachment store.
    NoFiles,
    /// Go `*OfflineError`: no device that holds the file is connected;
    /// the holders, origin first.
    Offline(Vec<String>),
    /// A blob error (BadId, TooLarge, Mismatch, ...) and its text.
    Blob(BlobKind, String),
    /// Go `errNotServed` (a Mismatch): the device doesn't serve it.
    NotServed,
    /// Anything else: the connection or stream failed.
    Other(String),
}

impl FetchError {
    /// Go `errors.Is(err, blob.Err…)`.
    pub fn is_blob(&self, k: BlobKind) -> bool {
        match self {
            FetchError::Blob(kind, _) => *kind == k,
            FetchError::NotServed => k == BlobKind::Mismatch,
            _ => false,
        }
    }
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::NoFiles => f.write_str("mesh: this device keeps no attachments"),
            FetchError::Offline(h) => write!(
                f,
                "mesh: no device that holds the file is connected ({})",
                h.join(", ")
            ),
            FetchError::Blob(_, m) | FetchError::Other(m) => f.write_str(m),
            FetchError::NotServed => write!(f, "{MISMATCH}: the device doesn't serve it"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<blob::Error> for FetchError {
    fn from(e: blob::Error) -> Self {
        FetchError::Blob(e.kind(), e.to_string())
    }
}

fn mismatch() -> FetchError {
    FetchError::Blob(BlobKind::Mismatch, MISMATCH.into())
}

fn offer_schema() -> Schema {
    Schema::object(vec![
        ("sha256", Schema::String),
        ("size", Schema::Int(64)),
        ("name", Schema::String),
        ("mime", Schema::String),
    ])
}

impl Node {
    /// Go `Offer`: records that this device holds `r`'s file and serves it
    /// (an attach.offered row), once.
    pub fn offer(&self, r: &FileRef) -> Result<(), String> {
        let holders = self.holders(&r.sha256).map_err(|e| e.to_string())?;
        if holders.contains(&self.self_id) {
            return Ok(());
        }
        self.o
            .store
            .append(
                "irori",
                store::kinds::ATTACH_OFFERED,
                &FileOffer {
                    sha256: r.sha256.clone(),
                    size: r.size,
                    name: r.name.clone(),
                    mime: r.mime.clone(),
                },
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// Go `holders`: every device whose attach.offered row names `sha`, in
    /// log order.
    pub fn holders(&self, sha: &str) -> store::Result<Vec<String>> {
        holders_in(&self.o.store, sha)
    }

    /// Go `FetchFile`: makes sure `r`'s file is in this device's attachment
    /// store, fetching it over a blob stream from the origin or any other
    /// holder. It returns the device it came from ("" when it was already
    /// here). Dropping the future gives up (a caller's deadline).
    pub async fn fetch_file(&self, r: &FileRef) -> Result<String, FetchError> {
        let Some(fs) = self.o.files.clone() else {
            return Err(FetchError::NoFiles);
        };
        check_ref(&fs, r)?;
        let _turn = self.lock_fetch(&r.sha256).await;
        if fs.has(&r.sha256) {
            return Ok(String::new());
        }
        let held = self
            .holders(&r.sha256)
            .map_err(|e| FetchError::Other(e.to_string()))?;
        // The origin first, then every other holder in log order.
        let mut cands: Vec<String> = Vec::new();
        if !r.device.is_empty() && r.device != self.self_id {
            cands.push(r.device.clone());
        }
        for h in held {
            if h != self.self_id && h != r.device {
                cands.push(h);
            }
        }
        if cands.is_empty() {
            return Err(FetchError::Offline(Vec::new()));
        }
        let mut last: Option<FetchError> = None;
        for dev in &cands {
            let Some(conn) = self.conn_to(dev) else {
                continue;
            };
            match fetch_over(conn.raw(), &fs, r).await {
                Ok(()) => {
                    if let Err(e) = self.offer(r) {
                        self.log(&format!("mesh: offer {}: {e}", r.sha256));
                    }
                    return Ok(dev.clone());
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or(FetchError::Offline(cands)))
    }

    /// Go `serveStream`: answers one stream the peer opened. Only
    /// certified, unrevoked devices are served, and only blob streams: a
    /// `board/` stream is refused here (boards stage 2 serves them only on
    /// the board's own `irori/board/1` connections). The stream is always
    /// finished.
    pub(crate) async fn serve_stream(&self, peer: &str, label: &str, mut st: Stream) {
        if label.starts_with(BLOB_LABEL_PREFIX)
            && self.certified(peer)
            && let Some(fs) = self.o.files.clone()
        {
            let log = |l: &str| self.log(l);
            serve_blob(
                &mut st,
                &fs,
                |sha| self.holders(sha).is_ok_and(|h| !h.is_empty()),
                log,
            )
            .await;
        }
        let _ = st.shutdown().await;
    }

    /// Go `connTo`: a live, synced connection to `dev`.
    pub(crate) fn conn_to(&self, dev: &str) -> Option<Arc<PeerConn>> {
        self.session_list()
            .into_iter()
            .find(|s| s.conn.id() == dev && s.hello_done() && !s.conn.done().is_cancelled())
            .map(|s| s.conn.clone())
    }

    /// Go `lockFetch`: one fetch per attachment at a time. The returned
    /// permit is the turn; dropping the waiting future gives up.
    pub async fn lock_fetch(&self, sha: &str) -> tokio::sync::OwnedSemaphorePermit {
        let turn = self
            .fetching
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(sha.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(1)))
            .clone();
        turn.acquire_owned()
            .await
            .expect("the semaphore is never closed")
    }
}

/// Go `holders` over any log: every device whose attach.offered row in
/// `st` names `sha`, in log order.
pub fn holders_in(st: &Store, sha: &str) -> store::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut f = Filter {
        kinds: vec![store::kinds::ATTACH_OFFERED.into()],
        limit: MAX_LIST_LIMIT,
        ..Filter::default()
    };
    let schema = offer_schema();
    loop {
        let evs = st.list(&f)?;
        for ev in &evs {
            if let Ok(o) = json::unmarshal(&ev.payload, &schema)
                && o.get_str("sha256") == sha
                && !out.contains(&ev.device_id)
            {
                out.push(ev.device_id.clone());
            }
        }
        if (evs.len() as i64) < f.limit {
            return Ok(out);
        }
        f.after = evs.last().map(Event::cursor).unwrap_or_default();
    }
}

/// Checks a ref before any fetch: a valid id and a size under the cap.
pub(crate) fn check_ref(fs: &blob::Files, r: &FileRef) -> Result<(), FetchError> {
    if !blob::valid_id(&r.sha256) {
        return Err(FetchError::Blob(
            BlobKind::BadId,
            "blob: id must be 64 lowercase hex characters".into(),
        ));
    }
    if r.size < 0 || r.size as u64 > fs.max() {
        return Err(FetchError::Blob(
            BlobKind::TooLarge,
            format!(
                "blob: the file is larger than the attachment cap ({} bytes, at most {})",
                r.size,
                fs.max()
            ),
        ));
    }
    Ok(())
}

/// Asks one connection for `r`'s file, up to [`FETCH_TRIES`] times (a
/// stream cut mid-file resumes by offset), and moves it into place once
/// it is whole. The private mesh and a board connection both fetch
/// through it; the caller holds the fetch turn ([`Node::lock_fetch`]).
pub(crate) async fn fetch_over(
    conn: &dyn Conn,
    fs: &blob::Files,
    r: &FileRef,
) -> Result<(), FetchError> {
    let mut last = mismatch();
    for _ in 0..FETCH_TRIES {
        match fetch_from(conn, fs, r).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let stop = !e.is_blob(BlobKind::Mismatch) || e == FetchError::NotServed;
                last = e;
                if stop {
                    break;
                }
            }
        }
    }
    Err(last)
}

/// Go `fetchFrom`: asks `conn` for the rest of the file and moves it into
/// place once it is whole.
async fn fetch_from(conn: &dyn Conn, fs: &blob::Files, r: &FileRef) -> Result<(), FetchError> {
    let (mut part, off) = fs.partial(&r.sha256)?;
    let size = r.size as u64;
    if off > size {
        drop(part);
        fs.drop_partial(&r.sha256);
        return Err(mismatch());
    }
    let mut got = 0;
    if off < size {
        got = read_blob(conn, fs, r, off, &mut part).await?;
    }
    drop(part);
    if off < size && got == 0 {
        return Err(FetchError::NotServed);
    }
    fs.finish(&r.sha256, size)?;
    Ok(())
}

/// Go `readBlob`: one blob stream; appends what it carries to `w`,
/// never more than the ref still lacks.
async fn read_blob(
    conn: &dyn Conn,
    fs: &blob::Files,
    r: &FileRef,
    off: u64,
    w: &mut std::fs::File,
) -> Result<u64, FetchError> {
    let mut idb = [0u8; 8];
    let _ = getrandom::fill(&mut idb);
    let id = hex::encode(idb);
    let want = r.size as u64 - off;
    let st = conn
        .open_stream(&format!("{BLOB_LABEL_PREFIX}{id}"))
        .await
        .map_err(|e| FetchError::Other(e.to_string()))?;
    let mut st: Stream = Box::new(tokio::io::join(st.recv, st.send));
    let body = json::marshal(&BlobRequest {
        id: &id,
        sha256: &r.sha256,
        offset: off as i64,
        len: want as i64,
    })
    .unwrap_or_default();
    let mut msg = (body.len() as u32).to_be_bytes().to_vec();
    msg.extend_from_slice(&body);
    st.write_all(&msg)
        .await
        .map_err(|e| FetchError::Other(e.to_string()))?;
    let mut buf = vec![0u8; BLOB_CHUNK];
    let mut got: u64 = 0;
    let mut err: Option<String> = None;
    while got <= want {
        let room = ((want + 1 - got) as usize).min(BLOB_CHUNK);
        match st.read(&mut buf[..room]).await {
            Ok(0) => break,
            Ok(k) => {
                if let Err(e) = w.write_all(&buf[..k]) {
                    err = Some(e.to_string());
                    break;
                }
                got += k as u64;
            }
            Err(e) => {
                err = Some(e.to_string());
                break;
            }
        }
    }
    if got > want {
        fs.drop_partial(&r.sha256);
        return Err(mismatch());
    }
    // An error after some bytes is a cut stream: the caller resumes.
    match err {
        Some(e) if got == 0 => Err(FetchError::Other(e)),
        _ => Ok(got),
    }
}

/// Go `serveBlob`: answers one blob stream: a request for a hash
/// `may_serve` allows and this device holds gets its bytes; anything
/// else is nothing written.
pub(crate) async fn serve_blob(
    st: &mut Stream,
    fs: &blob::Files,
    may_serve: impl Fn(&str) -> bool,
    log: impl Fn(&str),
) {
    let mut hdr = [0u8; 4];
    if st.read_exact(&mut hdr).await.is_err() {
        return;
    }
    let size = u32::from_be_bytes(hdr) as usize;
    if size == 0 || size > MAX_BLOB_REQUEST {
        return;
    }
    let mut body = vec![0u8; size];
    if st.read_exact(&mut body).await.is_err() {
        return;
    }
    let schema = Schema::object(vec![
        ("id", Schema::String),
        ("sha256", Schema::String),
        ("offset", Schema::Int(64)),
        ("len", Schema::Int(64)),
    ]);
    let Ok(req) = json::unmarshal(&body, &schema) else {
        return;
    };
    let int = |k: &str| match req.get(k) {
        Some(Out::Int(n)) => *n,
        _ => 0,
    };
    let (sha, offset, len) = (req.get_str("sha256").to_string(), int("offset"), int("len"));
    if !blob::valid_id(&sha) || offset < 0 || len < 0 {
        return;
    }
    // Only hashes the caller allows, and that are here.
    if !fs.has(&sha) || !may_serve(&sha) {
        return;
    }
    // The file is read through tokio's blocking pool, so a large or
    // slow attachment never holds a runtime worker.
    let Ok(f) = fs.open_file(&sha) else {
        return;
    };
    let mut f = tokio::fs::File::from_std(f);
    let Ok(meta) = f.metadata().await else {
        return;
    };
    let fsize = meta.len();
    if fsize > fs.max() || offset as u64 > fsize {
        return;
    }
    if f.seek(SeekFrom::Start(offset as u64)).await.is_err() {
        return;
    }
    let mut left = fsize - offset as u64;
    if len > 0 {
        left = left.min(len as u64);
    }
    let mut buf = vec![0u8; BLOB_CHUNK];
    while left > 0 {
        let room = (left as usize).min(BLOB_CHUNK);
        let k = match f.read(&mut buf[..room]).await {
            Ok(0) => break,
            Ok(k) => k,
            Err(e) => {
                log(&format!("mesh: serve {sha}: {e}"));
                return;
            }
        };
        if let Err(e) = st.write_all(&buf[..k]).await {
            log(&format!("mesh: serve {sha}: {e}"));
            return;
        }
        left -= k as u64;
    }
}
