//! The invite link (architecture §3.3 "Invites as tickets"):
//! `irori-invite:<base32 ticket>`. It replaces the design's
//! `irori-invite:<code>@<server host:port>#<person fingerprint>`
//! (v0.5-shared-boards §4.1 step 1): on iroh the invitee dials the
//! inviter's EndpointAddr, not a hub topic.
//!
//! The ticket is compact JSON, base32 (RFC 4648, lower case, no padding)
//! so it survives chat apps, line wrapping and case changes. It holds the
//! one-use code, the inviter's EndpointAddr (id, home relay URL and the
//! relay's key, direct addresses), the inviter's person fingerprint, and
//! the board's id and name for the join screen.
//!
//! Nothing in it is secret beyond the code, and the code alone joins
//! nothing: the words and the fingerprints read out on a call decide
//! (decision B4). A chat reader who changes the link changes the
//! fingerprint or the words, and the call catches it.

use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};

/// The link's scheme.
pub const SCHEME: &str = "irori-invite:";
const VERSION: u32 = 1;
/// The longest link [`Ticket::parse`] reads (a ticket is a few hundred
/// bytes; a pasted wall of text isn't one).
const MAX_LINK: usize = 4096;

/// What an invite link carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ticket {
    /// The one-use code, canonical (`XXXX-XXXX-XXXX-XXXX`).
    pub code: String,
    /// The inviter device's EndpointId, hex.
    pub endpoint: String,
    /// The inviter's home relay URL, or empty.
    pub relay: String,
    /// The relay's device key, hex (its TLS is on that key, D3), or empty.
    pub relay_key: String,
    /// The relay's QAD UDP port (`irori_net::invite::InviteAddr`), 0 for
    /// none.
    pub relay_quic_port: u16,
    /// The inviter's direct addresses (`ip:port`).
    pub addrs: Vec<String>,
    /// The inviter's person fingerprint ([`irori_pair::fingerprint`] of the
    /// person public key).
    pub fingerprint: String,
    pub board_id: String,
    pub board_name: String,
}

fn is_zero(n: &u16) -> bool {
    *n == 0
}

#[derive(Serialize, Deserialize)]
struct Wire {
    v: u32,
    c: String,
    e: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    r: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    rk: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    rq: u16,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    a: Vec<String>,
    f: String,
    b: String,
    #[serde(default)]
    n: String,
}

/// Why a link didn't parse. The text is what the join screen shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BadLink(pub &'static str);

impl std::fmt::Display for BadLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for BadLink {}

const NOT_A_LINK: BadLink =
    BadLink("That isn't an Irori invite link (it starts with irori-invite:).");
const DAMAGED: BadLink =
    BadLink("That invite link is damaged. Ask for the link again and paste all of it.");
const NEWER: BadLink =
    BadLink("That invite link is from a newer Irori. Update Irori and paste it again.");

impl Ticket {
    /// The link: `irori-invite:` and the base32 ticket.
    pub fn to_link(&self) -> String {
        let w = Wire {
            v: VERSION,
            c: self.code.clone(),
            e: self.endpoint.clone(),
            r: self.relay.clone(),
            rk: self.relay_key.clone(),
            rq: self.relay_quic_port,
            a: self.addrs.clone(),
            f: self.fingerprint.clone(),
            b: self.board_id.clone(),
            n: self.board_name.clone(),
        };
        let body = serde_json::to_vec(&w).expect("a ticket always marshals");
        format!(
            "{SCHEME}{}",
            BASE32_NOPAD.encode(&body).to_ascii_lowercase()
        )
    }

    /// Reads a link as pasted: surrounding space, any case, line breaks or
    /// spaces inside the ticket (chat apps wrap long words), with or
    /// without `=` padding.
    pub fn parse(link: &str) -> Result<Ticket, BadLink> {
        let link = link.trim();
        if link.len() > MAX_LINK {
            return Err(NOT_A_LINK);
        }
        let Some(rest) = link
            .get(..SCHEME.len())
            .filter(|s| s.eq_ignore_ascii_case(SCHEME))
            .map(|_| &link[SCHEME.len()..])
        else {
            return Err(NOT_A_LINK);
        };
        let b32: String = rest
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '=')
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let body = BASE32_NOPAD.decode(b32.as_bytes()).map_err(|_| DAMAGED)?;
        let w: Wire = serde_json::from_slice(&body).map_err(|_| DAMAGED)?;
        if w.v > VERSION {
            return Err(NEWER);
        }
        let code = irori_pair::normalize_code(&w.c).ok_or(DAMAGED)?;
        let hex32 = |s: &str| s.len() == 64 && hex::decode(s).is_ok();
        if w.v != VERSION
            || !hex32(&w.e)
            || !(w.rk.is_empty() || hex32(&w.rk))
            || !valid_fingerprint(&w.f)
            || w.b.is_empty()
            || w.a.len() > 16
            || w.a
                .iter()
                .any(|a| a.parse::<std::net::SocketAddr>().is_err())
        {
            return Err(DAMAGED);
        }
        Ok(Ticket {
            code,
            endpoint: w.e,
            relay: w.r,
            relay_key: w.rk,
            relay_quic_port: w.rq,
            addrs: w.a,
            fingerprint: w.f,
            board_id: w.b,
            board_name: w.n,
        })
    }
}

/// `xxxx xxxx xxxx xxxx`, lower-case hex, as [`irori_pair::fingerprint`]
/// writes it.
fn valid_fingerprint(f: &str) -> bool {
    let parts: Vec<&str> = f.split(' ').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            p.len() == 4
                && p.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Ticket {
        Ticket {
            code: "ABCD-EFGH-JKMN-PQRS".into(),
            endpoint: "11".repeat(32),
            relay: "https://127.0.0.1:4433/".into(),
            relay_key: "22".repeat(32),
            relay_quic_port: 4433,
            addrs: vec!["127.0.0.1:5000".into(), "[::1]:5001".into()],
            fingerprint: irori_pair::fingerprint(&[7u8; 32]),
            board_id: "b-0123".into(),
            board_name: "MLG & co".into(),
        }
    }

    #[test]
    fn round_trips() {
        let t = sample();
        let link = t.to_link();
        assert!(link.starts_with(SCHEME));
        assert!(
            link[SCHEME.len()..]
                .bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)),
            "{link}"
        );
        assert_eq!(Ticket::parse(&link).unwrap(), t);
    }

    #[test]
    fn reads_it_as_pasted() {
        let t = sample();
        let link = t.to_link();
        let (a, b) = link.split_at(40);
        let pasted = format!("  {}\n {}  ", a.to_uppercase(), b);
        assert_eq!(Ticket::parse(&pasted).unwrap(), t);
        let lower_code = Ticket {
            code: "abcd efgh jkmn pqrs".into(),
            ..t.clone()
        };
        assert_eq!(Ticket::parse(&lower_code.to_link()).unwrap(), t);
    }

    #[test]
    fn refuses_what_isnt_one() {
        assert_eq!(Ticket::parse("https://example.com"), Err(NOT_A_LINK));
        assert_eq!(Ticket::parse("irori-invite:"), Err(DAMAGED));
        assert_eq!(Ticket::parse("irori-invite:!!!"), Err(DAMAGED));
        let link = sample().to_link();
        assert_eq!(Ticket::parse(&link[..link.len() - 9]), Err(DAMAGED));
        for bad in [
            Ticket {
                code: "nope".into(),
                ..sample()
            },
            Ticket {
                endpoint: "zz".into(),
                ..sample()
            },
            Ticket {
                fingerprint: "1234".into(),
                ..sample()
            },
            Ticket {
                board_id: String::new(),
                ..sample()
            },
            Ticket {
                addrs: vec!["somewhere".into()],
                ..sample()
            },
        ] {
            assert_eq!(Ticket::parse(&bad.to_link()), Err(DAMAGED), "{bad:?}");
        }
        let long = format!("{SCHEME}{}", "a".repeat(MAX_LINK));
        assert_eq!(Ticket::parse(&long), Err(NOT_A_LINK));
    }

    #[test]
    fn a_newer_version_says_so() {
        let w = Wire {
            v: 2,
            c: "ABCD-EFGH-JKMN-PQRS".into(),
            e: "11".repeat(32),
            r: String::new(),
            rk: String::new(),
            rq: 0,
            a: vec![],
            f: irori_pair::fingerprint(&[7u8; 32]),
            b: "b".into(),
            n: String::new(),
        };
        let link = format!(
            "{SCHEME}{}",
            BASE32_NOPAD.encode(&serde_json::to_vec(&w).unwrap())
        );
        assert_eq!(Ticket::parse(&link), Err(NEWER));
    }
}
