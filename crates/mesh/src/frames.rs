//! Sync frames (mesh.md §2.1; I:internal/mesh/frames.go). One frame is one
//! JSON object per transport message, encoded with Go's `encoding/json`
//! rules (field order, `omitempty`) and decoded the way Go's `Unmarshal`
//! reads a struct. The bytes are frozen (D12): testdata/golden/mesh holds
//! frames Go encoded.

use std::sync::LazyLock;

use base64::Engine as _;
use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
use gocompat::json::{self, Out, Schema};
use serde::{Serialize, Serializer};
use store::{Event, Timestamp, Vector};

/// The sync protocol version a hello carries.
pub const PROTOCOL: i64 = 1;

/// Frame types on the sync channel.
pub const FRAME_HELLO: &str = "hello";
pub const FRAME_ROWS: &str = "rows";
pub const FRAME_VECTOR: &str = "vector";
pub const FRAME_PING: &str = "ping";
pub const FRAME_PONG: &str = "pong";
pub const FRAME_PERSON_SECRET: &str = "person.secret";
/// A shared board's secret to another of the person's devices
/// (boards2-transport.md §4): `boardId`, `epoch` (the board epoch), `maker`,
/// and `sealed` under the mesh key of epoch `from`.
pub const FRAME_BOARD_SECRET: &str = "board.secret";
/// Asks the peer for its copy of the asker's own row at `seq` (`device_id`
/// is the asker): a restored device checks the last row it holds against
/// the peer's before it takes the rows after it.
pub const FRAME_ROW_AT: &str = "row.at";

/// At most 500 rows and about 1 MiB per rows frame; `MAX_MESSAGE` is the
/// channel's cap on anything it receives.
pub const MAX_BATCH_ROWS: usize = 500;
pub const MAX_BATCH_BYTES: usize = 1 << 20;
pub const MAX_MESSAGE: usize = 8 << 20;

/// One sync message. `typ` picks which fields are set. Field order and
/// `omitempty` follow Go's struct, so [`Frame::encode`] writes Go's bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Frame {
    #[serde(rename = "type")]
    pub typ: String,

    // hello
    #[serde(skip_serializing_if = "is_zero")]
    pub v: i64,
    #[serde(rename = "deviceId", skip_serializing_if = "String::is_empty")]
    pub device_id: String,
    #[serde(rename = "personId", skip_serializing_if = "String::is_empty")]
    pub person_id: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub epoch: i64,
    #[serde(rename = "coreVersion", skip_serializing_if = "String::is_empty")]
    pub core_version: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub schema: i64,
    #[serde(rename = "wallMs", skip_serializing_if = "is_zero")]
    pub wall_ms: i64,
    /// hello and vector.
    #[serde(skip_serializing_if = "Vector::is_empty")]
    pub vector: Vector,
    /// The vector sent after applying a rows frame (not the periodic one):
    /// the sender sends its next batch on it.
    #[serde(skip_serializing_if = "is_false")]
    pub ack: bool,
    /// (hello) The RowHash of the receiver's own row at
    /// `vector[receiver]`, as the sender holds it.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub top: String,
    /// (hello) That row itself, which proves the receiver wrote that far.
    #[serde(rename = "topRow", skip_serializing_if = "Option::is_none")]
    pub top_row: Option<Box<Row>>,
    /// (row.at) The seq asked for.
    #[serde(skip_serializing_if = "is_zero")]
    pub seq: i64,

    // rows
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<Row>,
    #[serde(skip_serializing_if = "is_false")]
    pub loose: bool,
    /// Numbers a rows frame on its connection; its ack vector echoes it.
    #[serde(skip_serializing_if = "is_zero")]
    pub batch: i64,

    // ping / pong
    #[serde(skip_serializing_if = "is_zero")]
    pub t: i64,

    /// (hello) The sender's person key for its epoch.
    #[serde(rename = "personPub", skip_serializing_if = "String::is_empty")]
    pub person_pub: String,

    // person.secret (epoch too): the new epoch's secret, sealed with the
    // key of epoch `from` (0: the recipient's current epoch).
    #[serde(skip_serializing_if = "Vec::is_empty", serialize_with = "ser_bytes")]
    pub sealed: Vec<u8>,
    #[serde(skip_serializing_if = "is_zero")]
    pub from: i64,

    // board.secret (epoch, sealed and from too)
    #[serde(rename = "boardId", skip_serializing_if = "String::is_empty")]
    pub board_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub maker: String,
}

/// A log row on the wire. The payload travels as base64 of the stored
/// bytes, so the signed bytes survive the trip. `p` and `sig` have no
/// `omitempty`: empty encodes as `null`, as Go's nil `[]byte` does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Row {
    pub d: String,
    pub s: i64,
    pub h: u64,
    pub a: String,
    pub k: String,
    #[serde(serialize_with = "ser_bytes_or_null")]
    pub p: Vec<u8>,
    #[serde(serialize_with = "ser_bytes_or_null")]
    pub sig: Vec<u8>,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn ser_bytes<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_bytes(v)
}

fn ser_bytes_or_null<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
    if v.is_empty() {
        s.serialize_none()
    } else {
        s.serialize_bytes(v)
    }
}

impl Row {
    /// Go `RowOf`: `ev` on the wire.
    pub fn of(ev: &Event) -> Row {
        Row {
            d: ev.device_id.clone(),
            s: ev.seq,
            h: ev.hlc.0,
            a: ev.actor.clone(),
            k: ev.kind.clone(),
            p: ev.payload.clone(),
            sig: ev.sig.clone(),
        }
    }

    /// The row as a store Event.
    pub fn event(&self) -> Event {
        Event {
            device_id: self.d.clone(),
            seq: self.s,
            hlc: Timestamp(self.h),
            actor: self.a.clone(),
            kind: self.k.clone(),
            payload: self.p.clone(),
            sig: self.sig.clone(),
        }
    }

    /// Go `size`: the encoded bytes, estimated.
    pub fn size(&self) -> usize {
        96 + self.d.len() + self.a.len() + self.k.len() + (self.p.len() + self.sig.len()) * 4 / 3
    }
}

impl Frame {
    /// A frame of one type with every other field zero.
    pub fn of_type(typ: &str) -> Frame {
        Frame {
            typ: typ.to_string(),
            ..Frame::default()
        }
    }

    /// Go `Encode`: `json.Marshal(f)`.
    pub fn encode(&self) -> Vec<u8> {
        json::marshal(self).expect("a frame always marshals")
    }
}

/// Go's `base64.StdEncoding` as `[]byte` JSON decoding uses it: padding
/// required, non-zero trailing bits accepted, `\r` and `\n` ignored.
const GO_STD: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

fn go_base64(s: &str) -> Option<Vec<u8>> {
    if s.contains(['\r', '\n']) {
        let cleaned: String = s.chars().filter(|&c| c != '\r' && c != '\n').collect();
        return GO_STD.decode(cleaned.as_bytes()).ok();
    }
    GO_STD.decode(s.as_bytes()).ok()
}

fn row_schema() -> Schema {
    // []byte fields are read as strings, then base64-decoded as Go does.
    Schema::object(vec![
        ("d", Schema::String),
        ("s", Schema::Int(64)),
        ("h", Schema::Uint(64)),
        ("a", Schema::String),
        ("k", Schema::String),
        ("p", Schema::String),
        ("sig", Schema::String),
    ])
}

static FRAME_SCHEMA: LazyLock<Schema> = LazyLock::new(|| {
    Schema::object(vec![
        ("type", Schema::String),
        ("v", Schema::Int(64)),
        ("deviceId", Schema::String),
        ("personId", Schema::String),
        ("epoch", Schema::Int(64)),
        ("coreVersion", Schema::String),
        ("schema", Schema::Int(64)),
        ("wallMs", Schema::Int(64)),
        ("vector", Schema::Map(Box::new(Schema::Int(64)))),
        ("ack", Schema::Bool),
        ("top", Schema::String),
        ("topRow", Schema::Ptr(Box::new(row_schema()))),
        ("seq", Schema::Int(64)),
        ("events", Schema::Slice(Box::new(row_schema()))),
        ("loose", Schema::Bool),
        ("batch", Schema::Int(64)),
        ("t", Schema::Int(64)),
        ("personPub", Schema::String),
        ("sealed", Schema::String),
        ("from", Schema::Int(64)),
        ("boardId", Schema::String),
        ("maker", Schema::String),
    ])
});

fn int(o: &Out, name: &str) -> i64 {
    match o.get(name) {
        Some(Out::Int(n)) => *n,
        _ => 0,
    }
}

fn uint(o: &Out, name: &str) -> u64 {
    match o.get(name) {
        Some(Out::Uint(n)) => *n,
        _ => 0,
    }
}

fn boolean(o: &Out, name: &str) -> bool {
    matches!(o.get(name), Some(Out::Bool(true)))
}

/// A `[]byte` field: "" (or null, which leaves it nil) is empty.
fn bytes(o: &Out, name: &str) -> Result<Vec<u8>, String> {
    let s = o.get_str(name);
    if s.is_empty() {
        return Ok(Vec::new());
    }
    go_base64(s).ok_or_else(|| "illegal base64 data".to_string())
}

fn row_of(o: &Out) -> Result<Row, String> {
    Ok(Row {
        d: o.get_str("d").to_string(),
        s: int(o, "s"),
        h: uint(o, "h"),
        a: o.get_str("a").to_string(),
        k: o.get_str("k").to_string(),
        p: bytes(o, "p")?,
        sig: bytes(o, "sig")?,
    })
}

/// Go `DecodeFrame`: unmarshal one sync message. Unknown fields are
/// ignored; a missing type is an error.
pub fn decode_frame(b: &[u8]) -> Result<Frame, String> {
    let o = json::unmarshal(b, &FRAME_SCHEMA).map_err(|e| format!("mesh: bad frame: {e}"))?;
    let bad = |e: String| format!("mesh: bad frame: {e}");
    let mut vector = Vector::new();
    if let Some(Out::Map(Some(m))) = o.get("vector") {
        for (k, v) in m {
            if let Out::Int(n) = v {
                vector.insert(k.clone(), *n);
            }
        }
    }
    let top_row = match o.get("topRow") {
        Some(Out::Ptr(Some(r))) => Some(Box::new(row_of(r).map_err(bad)?)),
        _ => None,
    };
    let mut events = Vec::new();
    for r in o.get("events").map(Out::items).unwrap_or(&[]) {
        events.push(row_of(r).map_err(bad)?);
    }
    let f = Frame {
        typ: o.get_str("type").to_string(),
        v: int(&o, "v"),
        device_id: o.get_str("deviceId").to_string(),
        person_id: o.get_str("personId").to_string(),
        epoch: int(&o, "epoch"),
        core_version: o.get_str("coreVersion").to_string(),
        schema: int(&o, "schema"),
        wall_ms: int(&o, "wallMs"),
        vector,
        ack: boolean(&o, "ack"),
        top: o.get_str("top").to_string(),
        top_row,
        seq: int(&o, "seq"),
        events,
        loose: boolean(&o, "loose"),
        batch: int(&o, "batch"),
        t: int(&o, "t"),
        person_pub: o.get_str("personPub").to_string(),
        sealed: bytes(&o, "sealed").map_err(bad)?,
        from: int(&o, "from"),
        board_id: o.get_str("boardId").to_string(),
        maker: o.get_str("maker").to_string(),
    };
    if f.typ.is_empty() {
        return Err("mesh: frame has no type".into());
    }
    Ok(f)
}

/// A board chunk `{"events":[Row]}` as Go's `json.Unmarshal` reads it.
pub(crate) fn decode_chunk(b: &[u8]) -> Result<Vec<Row>, String> {
    let schema = Schema::object(vec![("events", Schema::Slice(Box::new(row_schema())))]);
    let o = json::unmarshal(b, &schema).map_err(|e| e.to_string())?;
    o.get("events")
        .map(Out::items)
        .unwrap_or(&[])
        .iter()
        .map(row_of)
        .collect()
}

/// Go `batches`: cut `evs` into rows frames of at most [`MAX_BATCH_ROWS`]
/// rows and about [`MAX_BATCH_BYTES`] each, in order. A single row larger
/// than the byte cap goes alone.
pub fn batches(evs: &[Event], loose: bool) -> Vec<Frame> {
    let mut out = Vec::new();
    let mut cur: Vec<Row> = Vec::new();
    let mut size = 0usize;
    let flush = |cur: &mut Vec<Row>, out: &mut Vec<Frame>| {
        out.push(Frame {
            events: std::mem::take(cur),
            loose,
            ..Frame::of_type(FRAME_ROWS)
        });
    };
    for ev in evs {
        let r = Row::of(ev);
        if !cur.is_empty() && (cur.len() >= MAX_BATCH_ROWS || size + r.size() > MAX_BATCH_BYTES) {
            flush(&mut cur, &mut out);
            size = 0;
        }
        size += r.size();
        cur.push(r);
    }
    if !cur.is_empty() {
        flush(&mut cur, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_board_secret_frame_round_trips() {
        let f = Frame {
            board_id: "0123456789abcdef0123456789abcdef".into(),
            epoch: 2,
            maker: "dev-a".into(),
            from: 3,
            sealed: vec![7; 56],
            ..Frame::of_type(FRAME_BOARD_SECRET)
        };
        assert_eq!(decode_frame(&f.encode()).unwrap(), f);
        // Frames without a board leave the new fields out.
        let p = Frame::of_type(FRAME_PING).encode();
        let s = String::from_utf8(p).unwrap();
        assert!(!s.contains("boardId") && !s.contains("maker"), "{s}");
    }
}
