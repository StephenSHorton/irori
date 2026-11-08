//! Settings → Devices (I:devices_service.go): the person's devices, the
//! home server, the advanced relay and pairing, behind [`crate::seams`]:
//! [`CoreMesh`] and [`CorePairing`] are those seams on the Core. Remove
//! device is the Core's mesh Node revoke (`Core::remove_device`); the
//! device rows, the home server's view of other devices' servers and files
//! held elsewhere are the running mesh's (`irori_core::meshview`).

use std::sync::Arc;

use blob::FileRef;
use gocompat::time::GoTime;
use irori_core::Core;
use irori_core::meshview::{FileFetchError, RemoteServer};
use serde::Serialize;
use specta::Type;

use crate::Window;
use crate::seams::{FetchError, Mesh, Pairing};
use crate::ts;

/// One device in the dialog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub device_id: String,
    pub name: String,
    pub role: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub this: bool,
    /// connected, recent or offline (this device: "this").
    pub state: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub revoked: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub warning: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub action: String,
    /// Zero (omitted): never heard from, or this device.
    #[serde(skip_serializing_if = "GoTime::is_zero")]
    #[specta(optional, type = ts::Time)]
    pub last_seen: GoTime,
    /// The device its rows come through while there is no session with it
    /// (card #544): an id from the mesh, its name by the time the window
    /// reads it.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub via: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub color: String,
    /// The work/personal sides it runs, the first its new cards' (card
    /// #494). Empty (omitted): every side.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub sides: Vec<String>,
}

/// The Home server section of Devices.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct HomeServerView {
    pub can: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub suggest: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub elsewhere: bool,
    pub on: bool,
    pub outside: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub public: String,
    pub running: bool,
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub port: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub reach: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub address: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub line: String,
    pub awake: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub awake_line: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub lan: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub remote: String,
    /// "Meet through public relays" (card #328), on every device.
    pub public_relays: bool,
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// The device that is joining (Go `pair.Joiner`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PairJoiner {
    pub device_id: String,
    pub name: String,
    pub os: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub fingerprint: String,
}

/// The open (or last) pairing (Go `pair.Status`); the devices:pairing
/// payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PairStatus {
    pub mode: String,
    pub state: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub code: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub server_addr: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub via: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional, type = Option<ts::Time>)]
    pub expires: Option<GoTime>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[specta(optional)]
    pub joiner: Option<PairJoiner>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[specta(optional)]
    pub words: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub peer: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[specta(optional)]
    pub key_ok: bool,
    #[serde(skip_serializing_if = "is_zero")]
    #[specta(optional)]
    pub misses: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    #[specta(optional)]
    pub error: String,
}

impl Window {
    /// DevicesService.Devices: the person's devices, this one first.
    pub fn devices(&self) -> Vec<DeviceView> {
        let colors = self.core.device_colors();
        let names = self.core.device_labels();
        let sides = self.core.device_sides();
        let mut out = self.seams.mesh.devices();
        for d in &mut out {
            d.color = colors.get(&d.device_id).cloned().unwrap_or_default();
            d.sides = sides.get(&d.device_id).cloned().unwrap_or_default();
            // The person's name for it, when they gave one (card #184).
            if let Some(n) = names.get(&d.device_id).filter(|n| !n.is_empty()) {
                d.name = n.clone();
            }
            if let Some(n) = names.get(&d.via).filter(|n| !n.is_empty()) {
                d.via = n.clone();
            }
        }
        out
    }

    /// DevicesService.RenameDevice: the person's name for one of their
    /// devices (`""` goes back to the computer's name); the name as kept.
    pub fn rename_device(&self, device_id: &str, name: &str) -> Result<String, String> {
        self.core.set_device_name(device_id, name)
    }

    /// DevicesService.SideNames: every work/personal side named on a
    /// device or a card (card #494), for Settings to offer.
    pub fn side_names(&self) -> Vec<String> {
        self.core.side_names()
    }

    /// DevicesService.SetDeviceSides: the sides this device runs, the
    /// first its new cards'; empty runs every side (card #494). The sides
    /// as kept.
    pub fn set_device_sides(&self, sides: &[String]) -> Result<Vec<String>, String> {
        self.core.set_device_sides(sides)
    }

    /// DevicesService.SetDeviceColor ("" goes back to the default).
    pub fn set_device_color(&self, device_id: &str, color: &str) -> Result<(), String> {
        self.core.set_device_color(device_id, color)
    }

    /// DevicesService.PersonColor: the person's colour on shared boards,
    /// "" for the default each friend's screen picks (card #385).
    pub fn person_color(&self) -> String {
        self.core.person_color()
    }

    /// DevicesService.SetPersonColor ("" goes back to the default).
    pub fn set_person_color(&self, color: &str) -> Result<(), String> {
        self.core.set_person_color(color)
    }

    /// DevicesService.PersonAvatar: the person's avatar on shared spaces,
    /// a small image data URL, "" for none (card #393).
    pub fn person_avatar(&self) -> String {
        self.core.person_avatar()
    }

    /// DevicesService.SetPersonAvatar ("" takes it off).
    pub fn set_person_avatar(&self, avatar: &str) -> Result<(), String> {
        self.core.set_person_avatar(avatar)
    }

    /// DevicesService.HomeServer.
    pub fn home_server(&self) -> HomeServerView {
        self.seams.mesh.home_server()
    }

    /// DevicesService.SetHomeServer.
    pub fn set_home_server(
        &self,
        on: bool,
        outside: bool,
        public: &str,
    ) -> Result<HomeServerView, String> {
        self.seams
            .mesh
            .set_home_server(on, outside, crate::trim(public))?;
        Ok(self.home_server())
    }

    /// DevicesService.SetHomeServerAwake.
    pub fn set_home_server_awake(&self, on: bool) -> Result<HomeServerView, String> {
        self.seams.mesh.set_home_server_awake(on)?;
        Ok(self.home_server())
    }

    /// "Meet through public relays" (card #328).
    pub fn set_public_relays(&self, on: bool) -> Result<HomeServerView, String> {
        self.seams.mesh.set_public_relays(on)?;
        Ok(self.home_server())
    }

    /// DevicesService.RemoveDevice.
    pub fn remove_device(&self, device_id: &str) -> Result<(), String> {
        self.seams.mesh.remove_device(device_id)
    }

    /// DevicesService.ResetDevice: marks the Reset for the next start; the
    /// app quits after the call returns (the app's `on_reset`).
    pub fn reset_device(&self) -> Result<(), String> {
        self.core.request_reset().map_err(|e| e.to_string())?;
        if let Some(f) = self.on_reset.get() {
            f();
        }
        Ok(())
    }
}

/// The mesh seam on the Core: the device rows with their live state, the
/// home server and files held on other devices come from the Core's
/// running mesh Node (Go `DevicesService` over `core.Mesh()`).
pub struct CoreMesh {
    core: Arc<Core>,
}

impl CoreMesh {
    pub fn new(core: Arc<Core>) -> CoreMesh {
        CoreMesh { core }
    }
}

/// Go `DevicesService.Devices`' fields of a mesh row (colours aside).
pub fn device_view(r: mesh::DeviceRow) -> DeviceView {
    DeviceView {
        device_id: r.device_id,
        name: r.name,
        role: r.role,
        this: r.this,
        state: r.state,
        revoked: r.revoked,
        warning: r.warning,
        action: r.action,
        last_seen: r.last_seen.unwrap_or_default(),
        via: r.via,
        color: String::new(),
        sides: Vec::new(),
    }
}

/// Go `remoteLine`: the one line about a home server this device can't
/// reach ("" when it reaches every one).
pub fn remote_line(servers: &[RemoteServer], devices: &[DeviceView]) -> String {
    for r in servers {
        if r.unreachable {
            let mut name = r.device_id.as_str();
            for d in devices {
                if d.device_id == r.device_id && !d.name.is_empty() {
                    name = &d.name;
                }
            }
            return format!(
                "Can't reach your home server ({name}). Away from home your devices connect through it."
            );
        }
    }
    String::new()
}

impl Mesh for CoreMesh {
    fn devices(&self) -> Vec<DeviceView> {
        self.core
            .mesh_devices(false)
            .into_iter()
            .map(device_view)
            .collect()
    }

    /// Go `HomeServer`.
    fn home_server(&self) -> HomeServerView {
        let st = self.core.home_server();
        let mut v = HomeServerView {
            can: self.core.can_serve(),
            suggest: self.core.role() == irori_identity::ROLE_NODE,
            elsewhere: self.core.other_server(),
            on: st.on,
            outside: st.outside,
            public: st.public,
            running: st.running,
            port: st.port,
            reach: st.reach,
            address: st.address,
            line: st.line,
            awake: st.awake,
            awake_line: st.awake_line,
            lan: self.core.lan_error(),
            remote: String::new(),
            public_relays: self.core.public_relays(),
        };
        // With public relays on, devices away from home meet through them,
        // so an unreachable home server isn't in anyone's way (card #580).
        if self.core.net_status().state == mesh::STATE_ON && !v.public_relays {
            v.remote = remote_line(&self.core.remote_servers(), &self.devices());
        }
        v
    }

    fn set_home_server(&self, on: bool, outside: bool, public: &str) -> Result<(), String> {
        self.core.set_home_server(on, outside, public)
    }

    fn set_home_server_awake(&self, on: bool) -> Result<(), String> {
        self.core.set_home_server_awake(on)
    }

    fn set_public_relays(&self, on: bool) -> Result<(), String> {
        self.core.set_public_relays(on)
    }

    /// Blocks on the Core's revoke: call it off the runtime's workers
    /// (the command runs it on a blocking thread).
    fn remove_device(&self, device_id: &str) -> Result<(), String> {
        self.core
            .pairing_runtime()
            .block_on(self.core.remove_device(device_id))
    }

    /// Go `fetchRef` within `FetchTimeout`. Blocks on the mesh's runtime:
    /// the commands and the attachment route call it on their own threads.
    fn fetch_file(&self, r: &FileRef) -> Result<(), FetchError> {
        if self.core.files().has(&r.sha256) {
            return Ok(());
        }
        self.core
            .fetch_file_blocking(r)
            .map(|_| ())
            .map_err(|e| match e {
                FileFetchError::Offline(on) => FetchError::Offline(on),
                FileFetchError::TimedOut => FetchError::TimedOut,
                FileFetchError::Other(s) => FetchError::Other(s),
            })
    }

    /// Go `mesh.Offer`.
    fn offer(&self, r: &FileRef) -> Result<(), String> {
        self.core.mesh().offer(r)
    }
}

/// The pairing seam on the Core's Pairer (Go `DevicesService`'s pairing
/// calls). The flow's calls block on the pairing runtime: call them off
/// the runtime's workers (the commands run them on a blocking thread).
pub struct CorePairing {
    core: Arc<Core>,
}

impl CorePairing {
    pub fn new(core: Arc<Core>) -> CorePairing {
        CorePairing { core }
    }

    fn run<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.core.pairing_runtime().block_on(f)
    }
}

/// The window's shape of the Pairer's status (the same JSON).
pub fn pair_status(s: irori_pair::Status) -> PairStatus {
    PairStatus {
        mode: s.mode,
        state: s.state,
        code: s.code,
        server_addr: s.server_addr,
        via: s.via,
        expires: s.expires,
        role: s.role,
        fingerprint: s.fingerprint,
        joiner: s.joiner.map(|j| PairJoiner {
            device_id: j.device_id,
            name: j.name,
            os: j.os,
            fingerprint: j.fingerprint,
        }),
        words: s.words,
        peer: s.peer,
        key_ok: s.key_ok,
        misses: s.misses,
        error: s.error,
    }
}

impl Pairing for CorePairing {
    fn status(&self) -> PairStatus {
        pair_status(self.core.pairing().status())
    }
    fn add(&self, role: &str) -> Result<PairStatus, String> {
        self.run(irori_core::devices::add_device(&self.core, role))
            .map(pair_status)
    }
    fn join(&self, code: &str, server: &str) -> Result<PairStatus, String> {
        self.run(irori_core::devices::join_with_code(
            &self.core, code, server,
        ))
        .map(pair_status)
    }
    fn confirm_words(&self, words: &str) -> Result<(), String> {
        self.run(irori_core::devices::confirm_words(&self.core, words))
    }
    fn confirm_key(&self, key: &str) -> Result<(), String> {
        self.run(irori_core::devices::confirm_key(&self.core, key))
    }
    fn cancel(&self) {
        self.core.pairing().cancel();
    }
    fn watch(&self, f: Option<Arc<dyn Fn(PairStatus) + Send + Sync>>) {
        self.core.pairing().set_on_change(f.map(|f| {
            Arc::new(move |s: irori_pair::Status| f(pair_status(s))) as irori_pair::StatusFn
        }));
    }
}
