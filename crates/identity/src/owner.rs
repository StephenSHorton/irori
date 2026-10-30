//! The owner table (I:internal/owner/owner.go; identity.md §4.8): any
//! device may ask, only the owner decides. Every entity that grants
//! something has one owning device; other devices write request rows.

pub const DELIVERY: &str = "delivery";
pub const LEASE: &str = "lease";
pub const ASK: &str = "ask";
pub const REVIVE: &str = "revive";
pub const SERVER: &str = "server";
pub const TAKE: &str = "take";

/// One row of the table.
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub entity: &'static str,
    /// Kinds only the owner writes.
    pub owner_kinds: &'static [&'static str],
    /// Kinds anyone may write; the owner applies the first it folds.
    pub request_kinds: &'static [&'static str],
}

/// Go `owner.Table`, in Go's order.
pub const TABLE: &[Rule] = &[
    Rule {
        entity: DELIVERY,
        owner_kinds: &[
            "delivery.state",
            "wake.issued",
            "turn.engaged",
            "turn.ended",
        ],
        request_kinds: &["delivery.decided"],
    },
    Rule {
        entity: LEASE,
        owner_kinds: &["lease.acquired", "lease.released", "lease.refused"],
        request_kinds: &["lease.stolen"],
    },
    Rule {
        entity: ASK,
        owner_kinds: &["ask.resolved"],
        request_kinds: &["ask.answered"],
    },
    Rule {
        entity: REVIVE,
        owner_kinds: &[],
        request_kinds: &["revive.requested"],
    },
    Rule {
        entity: SERVER,
        owner_kinds: &["node.addr"],
        request_kinds: &[],
    },
    Rule {
        entity: TAKE,
        owner_kinds: &["take.state"],
        request_kinds: &["take.requested"],
    },
];

/// The entity an owner-only kind belongs to.
pub fn entity_of(kind: &str) -> Option<&'static str> {
    TABLE
        .iter()
        .find(|r| r.owner_kinds.contains(&kind))
        .map(|r| r.entity)
}

/// The device a sender key lives on: `<device>/<session>` and
/// `user:<device>` name it; anything else (`irori`, a repo address) names
/// none and gives `""`.
pub fn device_of(key: &str) -> &str {
    if let Some(dev) = key.strip_prefix("user:") {
        return dev;
    }
    if let Some(i) = key.find('/')
        && i > 0
        && !key[..i].contains(':')
    {
        return &key[..i];
    }
    ""
}

/// Whether `writer_device` may write an owner-only row for the entity
/// owned through `owner_key`. A key naming no device has no owner to
/// enforce.
pub fn may_write(owner_key: &str, writer_device: &str) -> bool {
    let dev = device_of(owner_key);
    dev.is_empty() || dev == writer_device
}
