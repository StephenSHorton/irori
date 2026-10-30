//! Person-signed objects and the payloads that carry them
//! (I:internal/identity/rows.go, board.go:21-137, membership.go:51-127;
//! identity.md §2.2, §3).
//!
//! Every struct serializes in Go's field order with Go's `omitempty` rules,
//! so `gocompat::json::marshal` writes exactly the payload bytes Go stores,
//! and `gocompat::json::canonical` exactly the bytes a person signature
//! covers. Decoding is Go `json.Unmarshal` into the struct ([`decode`]):
//! case-insensitive keys, last duplicate wins, unknown fields dropped, a
//! type error fails the whole payload. Signatures are always checked over
//! Canonical(decoded struct), never over the stored bytes (trap 3).

use std::collections::BTreeMap;

use ed25519_dalek::{Signer as _, SigningKey};
use gocompat::json::{self, Out, Schema};
use gocompat::time::GoTime;
use serde::Serialize;

/// Roles a cert gives a device.
pub const ROLE_DESKTOP: &str = "desktop";
pub const ROLE_NODE: &str = "node";

/// A Go type decoded with `json.Unmarshal` semantics.
pub trait GoDecode: Sized {
    fn schema() -> Schema;
    fn from_out(o: &Out) -> Self;
}

/// `json.Unmarshal(payload, &v)`: `None` where Go returns an error.
pub fn decode<T: GoDecode>(payload: &[u8]) -> Option<T> {
    json::unmarshal(payload, &T::schema())
        .ok()
        .map(|o| T::from_out(&o))
}

pub(crate) fn get_i64(o: &Out, name: &str) -> i64 {
    match o.get(name) {
        Some(Out::Int(v)) => *v,
        _ => 0,
    }
}

pub(crate) fn get_time(o: &Out, name: &str) -> GoTime {
    match o.get(name) {
        Some(Out::Time(t)) => *t,
        _ => GoTime::ZERO,
    }
}

fn get_cut(o: &Out, name: &str) -> Option<BTreeMap<String, i64>> {
    match o.get(name) {
        Some(Out::Map(Some(m))) => Some(
            m.iter()
                .map(|(k, v)| {
                    let n = match v {
                        Out::Int(n) => *n,
                        _ => 0,
                    };
                    (k.clone(), n)
                })
                .collect(),
        ),
        _ => None,
    }
}

fn sub<T: GoDecode>(o: &Out, name: &str) -> T {
    match o.get(name) {
        Some(inner) => T::from_out(inner),
        None => T::from_out(&Out::zero(&T::schema())),
    }
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn s() -> Schema {
    Schema::String
}
fn int() -> Schema {
    Schema::Int(64)
}
fn time() -> Schema {
    Schema::Time
}

macro_rules! go_struct {
    ($ty:ident { $($json:literal => $field:ident : $kind:tt),* $(,)? }) => {
        impl GoDecode for $ty {
            fn schema() -> Schema {
                Schema::object(vec![$(($json, go_struct!(@schema $kind))),*])
            }
            fn from_out(o: &Out) -> Self {
                $ty { $($field: go_struct!(@get o, $json, $kind)),* }
            }
        }
    };
    (@schema str) => { s() };
    (@schema int) => { int() };
    (@schema time) => { time() };
    (@schema cut) => { Schema::Map(Box::new(int())) };
    (@schema $t:ident) => { <$t as GoDecode>::schema() };
    (@get $o:ident, $json:literal, str) => { $o.get_str($json).to_string() };
    (@get $o:ident, $json:literal, int) => { get_i64($o, $json) };
    (@get $o:ident, $json:literal, time) => { get_time($o, $json) };
    (@get $o:ident, $json:literal, cut) => { get_cut($o, $json) };
    (@get $o:ident, $json:literal, $t:ident) => { sub::<$t>($o, $json) };
}

/// What the person key signs to make a device part of the person.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cert {
    pub person_id: String,
    pub device_id: String,
    /// ed25519, hex.
    pub device_pub: String,
    pub label: String,
    pub role: String,
    pub at: GoTime,
}
go_struct!(Cert { "personId" => person_id: str, "deviceId" => device_id: str,
    "devicePub" => device_pub: str, "label" => label: str, "role" => role: str, "at" => at: time });

/// `person.device.added`. `personEpoch` sits beside the cert, outside the
/// signed bytes, so a v0.4 device that re-marshals its own Cert still
/// verifies.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAdded {
    pub cert: Cert,
    pub person_pub: String,
    pub person_sig: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub person_epoch: i64,
}
go_struct!(DeviceAdded { "cert" => cert: Cert, "personPub" => person_pub: str,
    "personSig" => person_sig: str, "personEpoch" => person_epoch: int });

/// What the person key signs to remove a device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Revoke {
    pub person_id: String,
    pub device_id: String,
    /// The highest seq of that device the revoker held.
    pub cut: i64,
    /// Hex sha256 of that row's signed bytes and sig (RowHash).
    pub cut_hash: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
}
go_struct!(Revoke { "personId" => person_id: str, "deviceId" => device_id: str, "cut" => cut: int,
    "cutHash" => cut_hash: str, "at" => at: time, "reason" => reason: str });

/// `person.device.revoked`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRevoked {
    pub revoke: Revoke,
    pub person_pub: String,
    pub person_sig: String,
    #[serde(skip_serializing_if = "is_zero")]
    pub person_epoch: i64,
}
go_struct!(DeviceRevoked { "revoke" => revoke: Revoke, "personPub" => person_pub: str,
    "personSig" => person_sig: str, "personEpoch" => person_epoch: int });

/// `person.key.rotated`: the new public key; since v0.5 also the device
/// that rotated and the previous key's signature over [`RotationBody`].
/// The private Chain checks neither.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRotated {
    pub person_id: String,
    pub epoch: i64,
    pub person_pub: String,
    pub at: GoTime,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prev_sig: String,
}
go_struct!(KeyRotated { "personId" => person_id: str, "epoch" => epoch: int, "personPub" => person_pub: str,
    "at" => at: time, "by" => by: str, "prevSig" => prev_sig: str });

impl KeyRotated {
    /// The rotation as its prevSig covers it.
    pub fn body(&self) -> RotationBody {
        RotationBody {
            person_id: self.person_id.clone(),
            person_epoch: self.epoch,
            person_pub: self.person_pub.clone(),
            by: self.by.clone(),
            at: self.at,
        }
    }
}

/// What a rotation's prevSig covers, private and board alike.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RotationBody {
    pub person_id: String,
    pub person_epoch: i64,
    pub person_pub: String,
    pub by: String,
    pub at: GoTime,
}

/// `person.profile`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub person_id: String,
    pub name: String,
    pub root_device_id: String,
    pub person_pub: String,
    pub person_sig: String,
}
go_struct!(Profile { "personId" => person_id: str, "name" => name: str, "rootDeviceId" => root_device_id: str,
    "personPub" => person_pub: str, "personSig" => person_sig: str });

impl Profile {
    pub fn body(&self) -> ProfileBody {
        ProfileBody {
            person_id: self.person_id.clone(),
            name: self.name.clone(),
            root_device_id: self.root_device_id.clone(),
            person_pub: self.person_pub.clone(),
        }
    }
}

/// What a Profile's personSig covers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileBody {
    pub person_id: String,
    pub name: String,
    pub root_device_id: String,
    pub person_pub: String,
}

/// What a board pins for one member: person, key and epoch as proven, and
/// the member's board root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberPin {
    pub person_id: String,
    pub person_pub: String,
    pub root_device_id: String,
    pub person_epoch: i64,
}
go_struct!(MemberPin { "personId" => person_id: str, "personPub" => person_pub: str,
    "rootDeviceId" => root_device_id: str, "personEpoch" => person_epoch: int });

/// A board's device cert: the private Cert without its label.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberCert {
    pub person_id: String,
    pub device_id: String,
    pub device_pub: String,
    pub role: String,
    pub person_epoch: i64,
    pub at: GoTime,
}
go_struct!(MemberCert { "personId" => person_id: str, "deviceId" => device_id: str, "devicePub" => device_pub: str,
    "role" => role: str, "personEpoch" => person_epoch: int, "at" => at: time });

/// `member.device.added`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDeviceAdded {
    pub cert: MemberCert,
    pub person_sig: String,
}
go_struct!(MemberDeviceAdded { "cert" => cert: MemberCert, "personSig" => person_sig: str });

/// Removes a member's device from one board; `board_cut` is a board-log seq.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberRevoke {
    pub person_id: String,
    pub device_id: String,
    pub board_cut: i64,
    pub board_cut_hash: String,
    pub person_epoch: i64,
    pub at: GoTime,
}
go_struct!(MemberRevoke { "personId" => person_id: str, "deviceId" => device_id: str, "boardCut" => board_cut: int,
    "boardCutHash" => board_cut_hash: str, "personEpoch" => person_epoch: int, "at" => at: time });

/// `member.device.revoked`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDeviceRevoked {
    pub revoke: MemberRevoke,
    pub person_sig: String,
}
go_struct!(MemberDeviceRevoked { "revoke" => revoke: MemberRevoke, "personSig" => person_sig: str });

/// `member.key.rotated`: written only by the device named in `by`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberKeyRotated {
    pub person_id: String,
    pub person_epoch: i64,
    pub person_pub: String,
    pub by: String,
    pub at: GoTime,
    pub prev_sig: String,
}
go_struct!(MemberKeyRotated { "personId" => person_id: str, "personEpoch" => person_epoch: int,
    "personPub" => person_pub: str, "by" => by: str, "at" => at: time, "prevSig" => prev_sig: str });

impl MemberKeyRotated {
    pub fn body(&self) -> RotationBody {
        RotationBody {
            person_id: self.person_id.clone(),
            person_epoch: self.person_epoch,
            person_pub: self.person_pub.clone(),
            by: self.by.clone(),
            at: self.at,
        }
    }
}

/// Go `MemberRotation`: the board copy of a private rotation; `false` for
/// one from before v0.5, which has no prevSig or by.
pub fn member_rotation(r: &KeyRotated) -> (MemberKeyRotated, bool) {
    let m = MemberKeyRotated {
        person_id: r.person_id.clone(),
        person_epoch: r.epoch,
        person_pub: r.person_pub.clone(),
        by: r.by.clone(),
        at: r.at,
        prev_sig: r.prev_sig.clone(),
    };
    (m, !r.prev_sig.is_empty() && !r.by.is_empty())
}

/// `member.profile`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberProfile {
    pub person_id: String,
    pub name: String,
    pub person_epoch: i64,
    pub person_sig: String,
}
go_struct!(MemberProfile { "personId" => person_id: str, "name" => name: str,
    "personEpoch" => person_epoch: int, "personSig" => person_sig: str });

impl MemberProfile {
    pub fn body(&self) -> MemberProfileBody {
        MemberProfileBody {
            person_id: self.person_id.clone(),
            name: self.name.clone(),
            person_epoch: self.person_epoch,
        }
    }
}

/// What a member.profile's personSig covers.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberProfileBody {
    pub person_id: String,
    pub name: String,
    pub person_epoch: i64,
}

/// `board.created`, self-signed by the creator for `creator.personEpoch`.
/// The signature covers the payload with `personSig` empty (so absent).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardCreated {
    pub board_id: String,
    pub name: String,
    pub tag: String,
    pub creator: MemberPin,
    pub at: GoTime,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub person_sig: String,
}
go_struct!(BoardCreated { "boardId" => board_id: str, "name" => name: str, "tag" => tag: str,
    "creator" => creator: MemberPin, "at" => at: time, "personSig" => person_sig: str });

/// `board.renamed`, by an owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardRenamed {
    pub name: String,
    pub by: String,
    pub at: GoTime,
    pub person_epoch: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub person_sig: String,
}
go_struct!(BoardRenamed { "name" => name: str, "by" => by: str, "at" => at: time,
    "personEpoch" => person_epoch: int, "personSig" => person_sig: str });

/// `board.member.added`: an owner pins a person. `person_epoch` is the
/// added person's; `by_epoch` is the signer's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardMemberAdded {
    pub person_id: String,
    pub person_pub: String,
    pub root_device_id: String,
    pub person_epoch: i64,
    pub name: String,
    pub role: String,
    pub by: String,
    pub by_epoch: i64,
    pub at: GoTime,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub person_sig: String,
}
go_struct!(BoardMemberAdded { "personId" => person_id: str, "personPub" => person_pub: str,
    "rootDeviceId" => root_device_id: str, "personEpoch" => person_epoch: int, "name" => name: str,
    "role" => role: str, "by" => by: str, "byEpoch" => by_epoch: int, "at" => at: time,
    "personSig" => person_sig: str });

impl BoardMemberAdded {
    /// The pin the row gives its person.
    pub fn pin(&self) -> MemberPin {
        MemberPin {
            person_id: self.person_id.clone(),
            person_pub: self.person_pub.clone(),
            root_device_id: self.root_device_id.clone(),
            person_epoch: self.person_epoch,
        }
    }
}

/// `board.member.role`, by an owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardMemberRole {
    pub person_id: String,
    pub role: String,
    pub by: String,
    pub at: GoTime,
    pub person_epoch: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub person_sig: String,
}
go_struct!(BoardMemberRole { "personId" => person_id: str, "role" => role: str, "by" => by: str,
    "at" => at: time, "personEpoch" => person_epoch: int, "personSig" => person_sig: str });

/// `board.member.removed`. `cut`: Go `nil` encodes `null`, an empty map
/// `{}`; both round-trip distinctly.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardMemberRemoved {
    pub person_id: String,
    pub by: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub reason: String,
    pub cut: Option<BTreeMap<String, i64>>,
    pub at: GoTime,
    pub person_epoch: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub person_sig: String,
}
go_struct!(BoardMemberRemoved { "personId" => person_id: str, "by" => by: str, "reason" => reason: str,
    "cut" => cut: cut, "at" => at: time, "personEpoch" => person_epoch: int, "personSig" => person_sig: str });

/// `board.member.left`, written by the leaving person's own device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardMemberLeft {
    pub person_id: String,
    pub cut: Option<BTreeMap<String, i64>>,
    pub at: GoTime,
    pub person_epoch: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub person_sig: String,
}
go_struct!(BoardMemberLeft { "personId" => person_id: str, "cut" => cut: cut, "at" => at: time,
    "personEpoch" => person_epoch: int, "personSig" => person_sig: str });

/// Go `Canonical(v)`: compact JSON, keys sorted at every level, `<>&` raw.
pub fn canonical<T: Serialize + ?Sized>(v: &T) -> Result<Vec<u8>, json::Error> {
    json::canonical(v)
}

/// Go `ed25519.Verify`: false for a key or signature of the wrong length,
/// a key that is not a point, or a non-canonical S. Non-strict, as Go.
pub fn ed25519_verify(public_key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    let (Ok(pk), Ok(sig)) = (<[u8; 32]>::try_from(public_key), <[u8; 64]>::try_from(sig)) else {
        return false;
    };
    let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(&pk) else {
        return false;
    };
    use ed25519_dalek::Verifier as _;
    vk.verify(msg, &ed25519_dalek::Signature::from_bytes(&sig))
        .is_ok()
}

/// Go `verifyPerson`: an ed25519 signature (hex) by `pub_hex` over
/// Canonical(v), with Go's error texts.
pub fn verify_person<T: Serialize + ?Sized>(
    pub_hex: &str,
    sig_hex: &str,
    v: &T,
) -> Result<(), String> {
    let msg = || canonical(v).map_err(|e| e.to_string());
    verify_person_with(pub_hex, sig_hex, msg)
}

/// [`verify_person`] with the canonical bytes made on demand.
pub(crate) fn verify_person_with(
    pub_hex: &str,
    sig_hex: &str,
    msg: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let pk = match hex::decode(pub_hex) {
        Ok(b) if b.len() == 32 => b,
        _ => return Err("identity: bad person key".into()),
    };
    let Ok(sig) = hex::decode(sig_hex) else {
        return Err("identity: bad person signature".into());
    };
    let msg = msg()?;
    if !ed25519_verify(&pk, &msg, &sig) {
        return Err("identity: person signature does not verify".into());
    }
    Ok(())
}

/// Hex ed25519 signature by `key` over Canonical(v).
pub fn sign_person<T: Serialize + ?Sized>(key: &SigningKey, v: &T) -> Result<String, json::Error> {
    let msg = canonical(v)?;
    Ok(hex::encode(key.sign(&msg).to_bytes()))
}

/// Go `validPub`: 32 bytes of hex.
pub fn valid_pub(h: &str) -> bool {
    matches!(hex::decode(h), Ok(b) if b.len() == 32)
}

/// Go `firstKeyOf`: `pub_hex` is the key `person_id` derives from.
pub fn first_key_of(person_id: &str, pub_hex: &str) -> bool {
    match hex::decode(pub_hex) {
        Ok(b) if b.len() == 32 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(&b);
            crate::person_id_of(&k) == person_id
        }
        _ => false,
    }
}

/// Hex of an ed25519 key's public half.
pub fn pub_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}
