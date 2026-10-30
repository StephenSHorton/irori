//! Pairing frames on the connection (identity.md §2.3, I:internal/pair/pair.go:260-281):
//! one JSON object per message, Go's field names and order. Go and Rust
//! devices can't pair with each other (D5), so these bytes only need to
//! round-trip between Rust devices; they still keep Go's shapes.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use irori_identity::{Person, RowRef};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use store::{Event, Timestamp};

pub(crate) const FRAME_PROOF: &str = "pair.proof";
pub(crate) const FRAME_GRANT: &str = "pair.grant";
pub(crate) const FRAME_DONE: &str = "pair.done";
pub(crate) const FRAME_FAIL: &str = "pair.fail";
pub(crate) const FRAME_CANCEL: &str = "pair.cancel";
/// B: the person typed A's key here.
pub(crate) const FRAME_KEY: &str = "pair.key";

/// A log row on the wire (Go `mesh.Row`): the payload and signature as
/// base64 of the stored bytes, `null` when empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireRow {
    pub d: String,
    pub s: i64,
    pub h: u64,
    pub a: String,
    pub k: String,
    #[serde(serialize_with = "ser_b64", deserialize_with = "de_b64", default)]
    pub p: Vec<u8>,
    #[serde(serialize_with = "ser_b64", deserialize_with = "de_b64", default)]
    pub sig: Vec<u8>,
}

impl WireRow {
    /// Go `mesh.RowOf`.
    pub fn of(ev: &Event) -> WireRow {
        WireRow {
            d: ev.device_id.clone(),
            s: ev.seq,
            h: ev.hlc.0,
            a: ev.actor.clone(),
            k: ev.kind.clone(),
            p: ev.payload.clone(),
            sig: ev.sig.clone(),
        }
    }

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
}

pub(crate) fn row_ref(ev: &Event) -> RowRef<'_> {
    ev.into()
}

fn ser_b64<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
    if v.is_empty() {
        s.serialize_none()
    } else {
        s.serialize_str(&STANDARD.encode(v))
    }
}

fn de_b64<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    let s: Option<String> = Option::deserialize(d)?;
    match s {
        None => Ok(Vec::new()),
        Some(s) => STANDARD.decode(s).map_err(serde::de::Error::custom),
    }
}

fn ser_b64_opt<S: Serializer>(v: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(b) => s.serialize_str(&STANDARD.encode(b)),
        None => s.serialize_none(),
    }
}

fn de_b64_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
    let s: Option<String> = Option::deserialize(d)?;
    s.map(|s| STANDARD.decode(s).map_err(serde::de::Error::custom))
        .transpose()
}

/// The person as the grant carries it (Go `identity.Person`'s JSON).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirePerson {
    #[serde(rename = "personId", default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub epoch: i64,
    #[serde(rename = "rootDeviceId", default)]
    pub root_device_id: String,
    #[serde(rename = "personPub", default)]
    pub pub_hex: String,
}

impl From<&Person> for WirePerson {
    fn from(p: &Person) -> Self {
        WirePerson {
            id: p.id.clone(),
            name: p.name.clone(),
            epoch: p.epoch,
            root_device_id: p.root_device_id.clone(),
            pub_hex: p.pub_hex.clone(),
        }
    }
}

impl From<&WirePerson> for Person {
    fn from(p: &WirePerson) -> Self {
        Person {
            id: p.id.clone(),
            name: p.name.clone(),
            epoch: p.epoch,
            root_device_id: p.root_device_id.clone(),
            pub_hex: p.pub_hex.clone(),
        }
    }
}

/// One message on the pairing connection (Go `frame`). Nothing in it is
/// logged: a grant carries the person secret.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Frame {
    #[serde(rename = "type", default)]
    pub typ: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "ser_b64_opt",
        deserialize_with = "de_b64_opt"
    )]
    pub proof: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub os: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<WirePerson>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "ser_b64_opt",
        deserialize_with = "de_b64_opt"
    )]
    pub seed: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<WireRow>,
    /// (grant) Every home server A knows and where to dial it, so B finds
    /// the server at once, before sync brings their `node.addr` rows.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub servers: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
}

impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the seed or the rows.
        f.debug_struct("Frame")
            .field("type", &self.typ)
            .finish_non_exhaustive()
    }
}

impl Frame {
    pub(crate) fn of(typ: &str) -> Frame {
        Frame {
            typ: typ.into(),
            ..Default::default()
        }
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("frame json")
    }

    /// A frame without a type is "bad frame" (Go pair.go:695).
    pub(crate) fn decode(b: &[u8]) -> Result<Frame, &'static str> {
        match serde_json::from_slice::<Frame>(b) {
            Ok(f) if !f.typ.is_empty() => Ok(f),
            _ => Err("pair: bad frame"),
        }
    }
}

/// The hello body (Go `helloBody`): who is joining.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    #[serde(rename = "deviceId", default)]
    pub device_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub os: String,
}
