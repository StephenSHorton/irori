//! What the server serves (Go `api.Backend`), and the tool set it hands
//! tool calls to.

use std::sync::Arc;

use coord::Coord;
use gocompat::json::RawJson;
use inbox::{Inbox, Sender};
use irori_core::{Core, Member};
use irori_identity::DeviceJson;
use store::{Event, Filter};

/// What the server serves; [`Core`] implements it. A backend without a
/// service returns `None` and its routes answer 503, as Go's do for a nil
/// service.
pub trait Backend: Send + Sync + 'static {
    fn version(&self) -> String;
    /// This device (`device.json`, Go `device.Identity`).
    fn device(&self) -> DeviceJson;
    /// This device's roster (Go `LocalAgents`): presence plus Grok members.
    fn agents(&self) -> Vec<presence::AgentCard>;
    fn events(&self, f: &Filter) -> Result<Vec<Event>, String>;
    /// Rows appended or inserted from now on; a zero event (seq 0) means
    /// rows were dropped and the reader should re-read.
    fn subscribe_events(&self) -> store::Subscription;
    fn inbox(&self) -> Option<Arc<Inbox>>;
    fn coord(&self) -> Option<Arc<Coord>>;
    fn capture(&self) -> Option<capture::Capture>;
    /// Binds a Grok member (`POST /v1/members`).
    fn join_member(&self, name: &str, session: &str) -> Result<Member, String>;
    /// Maps `X-Irori-Member` (or `?member=`) to its member.
    fn member_by_secret(&self, secret: &str) -> Option<Member>;
    /// That key's member stream opened (true) or closed (false).
    fn member_stream(&self, key: &str, open: bool);
    /// The pairing flow and the runtime it runs on (Go `Pairing()`), for
    /// the joining side's routes (`/v1/pair/*`); `None` answers 503.
    fn pairing(&self) -> Option<(irori_pair::Pairer, tokio::runtime::Handle)> {
        None
    }
    /// The person's devices, this one first (Go `Mesh().Devices(detail)`),
    /// for `GET /v1/devices`; `None` (no mesh) answers 503.
    fn mesh_devices(&self, detail: bool) -> Option<Vec<mesh::DeviceRow>> {
        let _ = detail;
        None
    }
    /// The shared boards this device holds, for `GET /v1/boards`;
    /// `detail` adds members, fingerprints and the set-aside counts.
    /// The Core always answers (an empty list when it holds none); `None`,
    /// a backend without a boards layer, answers 503.
    fn boards(&self, detail: bool) -> Option<Vec<crate::boards::BoardRow>> {
        let _ = detail;
        None
    }
    /// The net layer's state and how many other devices are connected now
    /// (Go `Mesh().Status().State` and `Mesh().Connected()`), for health's
    /// `net`; `None` (no mesh) leaves `net` out.
    fn net_health(&self) -> Option<NetHealth> {
        None
    }
}

/// Health's `net` (Go `api.NetHealth`): the net layer's state (off, on,
/// removed, error) and how many other devices are connected now.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct NetHealth {
    pub state: String,
    pub peers: usize,
}

impl Backend for Core {
    fn version(&self) -> String {
        Core::version(self).to_string()
    }
    fn device(&self) -> DeviceJson {
        Core::device(self).clone()
    }
    fn agents(&self) -> Vec<presence::AgentCard> {
        self.local_agents()
    }
    fn events(&self, f: &Filter) -> Result<Vec<Event>, String> {
        Core::events(self, f).map_err(|e| e.to_string())
    }
    fn subscribe_events(&self) -> store::Subscription {
        self.subscribe_rows()
    }
    fn inbox(&self) -> Option<Arc<Inbox>> {
        Some(Core::inbox(self).clone())
    }
    fn coord(&self) -> Option<Arc<Coord>> {
        Some(Core::coord(self).clone())
    }
    fn capture(&self) -> Option<capture::Capture> {
        Some(Core::capture(self).clone())
    }
    fn join_member(&self, name: &str, session: &str) -> Result<Member, String> {
        Core::join_member(self, name, session)
    }
    fn member_by_secret(&self, secret: &str) -> Option<Member> {
        Core::member_by_secret(self, secret)
    }
    fn member_stream(&self, key: &str, open: bool) {
        Core::member_stream(self, key, open)
    }
    fn pairing(&self) -> Option<(irori_pair::Pairer, tokio::runtime::Handle)> {
        Some((Core::pairing(self).clone(), self.pairing_runtime().clone()))
    }
    fn mesh_devices(&self, detail: bool) -> Option<Vec<mesh::DeviceRow>> {
        Some(Core::mesh_devices(self, detail))
    }
    fn boards(&self, detail: bool) -> Option<Vec<crate::boards::BoardRow>> {
        Some(
            self.board_list()
                .into_iter()
                .map(|s| {
                    let detail = detail.then(|| board_detail(self, &s.board_id));
                    crate::boards::BoardRow {
                        board_id: s.board_id,
                        name: s.name,
                        tag: s.tag,
                        people: s.people,
                        owner: s.owner,
                        devices: s.devices,
                        cards: s.cards,
                        ready: s.ready,
                        detail,
                    }
                })
                .collect(),
        )
    }
    fn net_health(&self) -> Option<NetHealth> {
        let n = self.mesh();
        Some(NetHealth {
            state: n.status().state,
            peers: n.connected_count(),
        })
    }
}

/// What `?detail=1` adds for one board (v0.5 §2.3, §8): every member the
/// board ever added with their fingerprint and counted devices, the fold's
/// set-aside counters, and the membership rows that didn't count. A board
/// forgotten since the list was read shows an empty detail.
fn board_detail(c: &Core, board_id: &str) -> crate::boards::BoardDetail {
    let mut d = crate::boards::BoardDetail::default();
    if let Ok(co) = c.board_coord(board_id) {
        d.set_aside = co.set_aside();
    }
    let Ok(info) = c.board_info(board_id) else {
        return d;
    };
    d.members = info
        .members
        .iter()
        .map(|p| {
            let who = c.board_member(board_id, &p.person_id).unwrap_or_default();
            crate::boards::BoardMember {
                person_id: p.person_id.clone(),
                name: if who.name.is_empty() {
                    p.name.clone()
                } else {
                    who.name
                },
                fingerprint: hex::decode(&p.pin.person_pub)
                    .ok()
                    .filter(|k| !k.is_empty())
                    .map(|k| irori_pair::fingerprint(&k))
                    .unwrap_or_default(),
                role: p.role.clone(),
                state: p.state.clone(),
                devices: who.devices.iter().filter(|d| !d.revoked).count() as i64,
            }
        })
        .collect();
    d.conflicts = info
        .conflicts
        .iter()
        .map(|x| {
            let mut s = x.kind.clone();
            if !x.person_id.is_empty() {
                s += &format!(" for {}", x.person_id);
            }
            if !x.by.is_empty() {
                s += &format!(" by {}", x.by);
            }
            format!("{s}: {}", x.reason)
        })
        .collect();
    d
}

/// Who a tool call acts as (Go `tools.Caller`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Caller {
    pub sender: Sender,
    /// `inbox::path::MOD` or `inbox::path::CHANNEL`.
    pub path: String,
}

/// Why a tool call did not run (Go's `Set.Call` errors). A tool's own
/// failure is not one: it is `Ok` text starting `Error: `.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// Go `tools.ErrUnknownTool` → 404 `no tool called "x"`.
    UnknownTool,
    /// Anything else (`tools: args must be a JSON object`) → 400 with the text.
    Other(String),
}

/// The agent tools (Go `*tools.Set`), served by `GET /v1/tools` and
/// `POST /v1/tools/{name}`.
///
/// [`CoreTools`] implements it over the Core's tools; the app passes it to
/// [`crate::Server::with_tools`].
pub trait ToolSet: Send + Sync + 'static {
    /// The specs for `client` (`claude` or `grok`), in front-end order, each
    /// a Go `tools.Spec` object as `json.Marshal` writes it.
    fn specs(&self, client: &str) -> Vec<RawJson>;
    /// Runs tool `name` with `args` (a JSON object; the server turns an
    /// absent or `null` args into `{}`). The text is clipped by the tool set.
    fn call(&self, caller: &Caller, name: &str, args: &[u8]) -> Result<String, ToolError>;
}

/// The Core's agent tools ([`Core::tools`], the tools crate's `Set`) as the
/// server's [`ToolSet`]: `Server::new(core.clone(), token)
/// .with_tools(Arc::new(CoreTools(core)))`.
pub struct CoreTools(pub Arc<Core>);

impl ToolSet for CoreTools {
    fn specs(&self, client: &str) -> Vec<RawJson> {
        self.0
            .tools()
            .specs(client)
            .iter()
            .map(|sp| RawJson(gocompat::json::marshal(sp).expect("api: marshal a tool spec")))
            .collect()
    }

    fn call(&self, caller: &Caller, name: &str, args: &[u8]) -> Result<String, ToolError> {
        let c = tools::Caller {
            sender: caller.sender.clone(),
            path: caller.path.clone(),
        };
        self.0.tools().call(&c, name, args).map_err(|e| match e {
            tools::CallError::UnknownTool(_) => ToolError::UnknownTool,
            e => ToolError::Other(e.to_string()),
        })
    }
}
