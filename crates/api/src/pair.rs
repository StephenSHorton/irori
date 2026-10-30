//! The joining side of pairing (I:internal/api/routes_pair.go, pairing.go,
//! client_pair.go; v0.4 §3, §6.8), so `irori pair join` can drive the
//! running Core instead of starting its own. Paths, statuses and JSON
//! shapes are Go's:
//!
//! - `POST /v1/pair/join` `{"code","server"}` → the status; 400 a bad code
//!   or server address, 409 a pairing is already open or this device is
//!   already one of a person's devices.
//! - `GET /v1/pair/status` → the joining side's view: while an Add window
//!   is open only `{mode:"add", state}` shows, never its code or key.
//! - `POST /v1/pair/key` `{"key"}` → the status; 422 not the key the other
//!   device's handshake proved (the miss counts), 409 nothing waits for one.
//! - `POST /v1/pair/cancel` → the status; closes a join only, never an Add.
//!
//! 503 when the backend has no pairing. The code, the server address and
//! the key travel only in POST bodies and no answer repeats the code or the
//! key. There is NO route that opens a pairing window (Add), types the
//! words on the adding side or removes a device, and none may be added:
//! Add hands the new device the person secret and every row, so it stays in
//! the desktop app's Settings → Devices. The gate is the bearer token alone
//! (Go routes_pair.go has the full reasoning).

use irori_pair::{Error, MODE_ADD, Pairer, Status};

use crate::body;
use crate::client::{Client, Error as ClientError};
use crate::http::{self, Resp};
use crate::server::Handler;

pub const PATH_PAIR_JOIN: &str = "/v1/pair/join";
pub const PATH_PAIR_STATUS: &str = "/v1/pair/status";
pub const PATH_PAIR_KEY: &str = "/v1/pair/key";
pub const PATH_PAIR_CANCEL: &str = "/v1/pair/cancel";
/// Caps the code, server and key a pairing body may carry.
pub const MAX_PAIR_FIELD: usize = 200;

/// What a home server address must look like (Go's text, also the CLI's).
pub const BAD_SERVER: &str = "the home server address is a host and port, like 203.0.113.7:24817";

/// Whether `path` is a pairing route that accepts POST.
pub(crate) fn pair_post(path: &str) -> bool {
    matches!(path, PATH_PAIR_JOIN | PATH_PAIR_KEY | PATH_PAIR_CANCEL)
}

/// Go `JoinView`: everything on the joining side, and only the mode and
/// state of an Add, whose code and key are secrets the person types.
pub fn join_view(st: Status) -> Status {
    if st.mode == MODE_ADD {
        return Status {
            mode: st.mode,
            state: st.state,
            ..Status::default()
        };
    }
    st
}

pub(crate) fn route(h: &Handler) -> Option<Resp> {
    let p = h.r.path.as_str();
    if !matches!(
        p,
        PATH_PAIR_JOIN | PATH_PAIR_STATUS | PATH_PAIR_KEY | PATH_PAIR_CANCEL
    ) {
        return None;
    }
    let get = h.r.method == "GET";
    if get != (p == PATH_PAIR_STATUS) {
        let allow = if p == PATH_PAIR_STATUS { "GET" } else { "POST" };
        return Some(http::error(405, "method not allowed").with_header("Allow", allow));
    }
    let Some((pr, rt)) = h.s.b.pairing() else {
        return Some(http::error(503, "this Core has no pairing"));
    };
    // The server's handlers run on blocking threads (server::dispatch), so
    // the Pairer's calls block on its own runtime here, as the window's do.
    Some(match p {
        PATH_PAIR_STATUS => http::json(200, &join_view(pr.status())),
        PATH_PAIR_JOIN => pair_join(h, &pr, &rt),
        PATH_PAIR_KEY => pair_key(h, &pr, &rt),
        _ => {
            pr.cancel_join();
            http::json(200, &join_view(pr.status()))
        }
    })
}

/// Go `pairField`: trims a body field and refuses one too long or with
/// control characters. The message never repeats the value.
fn pair_field(v: &str, what: &str) -> Result<String, String> {
    let v = v.trim_matches(gocompat::text::is_space);
    if v.len() > MAX_PAIR_FIELD || v.chars().any(gocompat::text::is_control) {
        return Err(format!("{what} is too long or has control characters"));
    }
    Ok(v.to_string())
}

fn pair_join(h: &Handler, pr: &Pairer, rt: &tokio::runtime::Handle) -> Resp {
    let d = match body::read(&h.r.body, h.r.body_too_large, &body::PAIR_JOIN) {
        Ok(d) => d,
        Err(e) => return http::error(400, &e),
    };
    let code = match pair_field(&d.str(0), "code") {
        Ok(v) => v,
        Err(e) => return http::error(400, &e),
    };
    let mut server = match pair_field(&d.str(1), "server") {
        Ok(v) => v,
        Err(e) => return http::error(400, &e),
    };
    if !server.is_empty() {
        match home::hostport::host_port(&server, 0) {
            Ok(hp) => server = hp,
            Err(_) => return http::error(400, BAD_SERVER),
        }
    }
    match rt.block_on(pr.join_via(&code, &server)) {
        Ok(st) => http::json(200, &join_view(st)),
        Err(e @ Error::BadCode) => http::error(400, &e.to_string()),
        Err(e @ (Error::Busy | Error::InMesh)) => http::error(409, &e.to_string()),
        Err(e) => http::error(500, &e.to_string()),
    }
}

fn pair_key(h: &Handler, pr: &Pairer, rt: &tokio::runtime::Handle) -> Resp {
    let d = match body::read(&h.r.body, h.r.body_too_large, &body::PAIR_KEY) {
        Ok(d) => d,
        Err(e) => return http::error(400, &e),
    };
    let key = match pair_field(&d.str(0), "key") {
        Ok(v) => v,
        Err(e) => return http::error(400, &e),
    };
    // The grant may be adopted inside this call; it runs on this blocking
    // thread to its end even if the client hangs up (Go WithoutCancel).
    match rt.block_on(pr.confirm_key(&key)) {
        Ok(()) => http::json(200, &join_view(pr.status())),
        Err(e @ Error::WrongKey) => http::error(422, &e.to_string()),
        // NotOpen's own text is about words, the adding side's step.
        Err(Error::NotOpen) => http::error(409, "no pairing is waiting for a key"),
        Err(e) => http::error(500, &e.to_string()),
    }
}

// ---- the client (Go client_pair.go) -----------------------------------------

fn decode(path: &str, raw: &[u8]) -> Result<Status, ClientError> {
    serde_json::from_slice(raw).map_err(|e| ClientError::Other(format!("irori api {path}: {e}")))
}

fn pair_body(fields: &[(&str, &str)]) -> Vec<u8> {
    let m: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|(k, v)| (k.to_string(), serde_json::Value::from(*v)))
        .collect();
    serde_json::to_vec(&m).unwrap_or_default()
}

impl Client {
    /// Enters a code on this device (`POST /v1/pair/join`). `server` is the
    /// home server's address, `""` at home.
    pub async fn pair_join(&self, code: &str, server: &str) -> Result<Status, ClientError> {
        let body = pair_body(&[("code", code), ("server", server)]);
        let raw = self.call("POST", PATH_PAIR_JOIN, &[], Some(body)).await?;
        decode(PATH_PAIR_JOIN, &raw)
    }

    /// The joining side's pairing (`GET /v1/pair/status`).
    pub async fn pair_status(&self) -> Result<Status, ClientError> {
        let raw = self.call("GET", PATH_PAIR_STATUS, &[], None).await?;
        decode(PATH_PAIR_STATUS, &raw)
    }

    /// Types the other device's key (`POST /v1/pair/key`); a wrong key is an
    /// error [`is_wrong_key`] reports.
    pub async fn pair_key(&self, key: &str) -> Result<Status, ClientError> {
        let body = pair_body(&[("key", key)]);
        let raw = self.call("POST", PATH_PAIR_KEY, &[], Some(body)).await?;
        decode(PATH_PAIR_KEY, &raw)
    }

    /// Closes this device's join, if one is open (`POST /v1/pair/cancel`).
    /// It never closes an Add window.
    pub async fn pair_cancel(&self) -> Result<Status, ClientError> {
        let raw = self.call("POST", PATH_PAIR_CANCEL, &[], None).await?;
        decode(PATH_PAIR_CANCEL, &raw)
    }
}

/// Whether `e` is the server refusing a key (422).
pub fn is_wrong_key(e: &ClientError) -> bool {
    matches!(e, ClientError::Status { code: 422, .. })
}

/// Whether `e` is the server not knowing the route: a Core older than the
/// pairing routes.
pub fn is_no_route(e: &ClientError) -> bool {
    matches!(
        e,
        ClientError::Status {
            code: 404 | 405,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_like_go() {
        assert_eq!(pair_field("  abc \n", "code").unwrap(), "abc");
        assert_eq!(
            pair_field("a\u{7}b", "code").unwrap_err(),
            "code is too long or has control characters"
        );
        assert_eq!(
            pair_field(&"x".repeat(MAX_PAIR_FIELD + 1), "key").unwrap_err(),
            "key is too long or has control characters"
        );
        assert!(pair_field(&"x".repeat(MAX_PAIR_FIELD), "key").is_ok());
    }

    #[test]
    fn an_add_shows_only_its_mode_and_state() {
        let st = Status {
            mode: MODE_ADD.into(),
            state: irori_pair::STATE_WAITING.into(),
            code: "ABCD-EFGH-JKMN-PQRS".into(),
            fingerprint: "1111 2222 3333 4444".into(),
            server_addr: "203.0.113.7:24817".into(),
            role: "desktop".into(),
            ..Status::default()
        };
        let v = join_view(st);
        assert_eq!(
            v,
            Status {
                mode: MODE_ADD.into(),
                state: irori_pair::STATE_WAITING.into(),
                ..Status::default()
            }
        );
        let j = Status {
            mode: irori_pair::MODE_JOIN.into(),
            state: irori_pair::STATE_WORDS.into(),
            words: vec!["a".into()],
            ..Status::default()
        };
        assert_eq!(join_view(j.clone()), j);
    }
}
