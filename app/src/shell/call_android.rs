//! Card #654: a call's Android side. [`Phone`] is the call layer's
//! [`call::Platform`] there, over the app's Kotlin `CallPlugin`
//! (app/gen/android): `begin` starts `CallService`, a microphone foreground
//! service that keeps the call running with the screen off, and puts the
//! phone in its voice-call audio mode; `end` stops both; `route` picks the
//! speaker or the earpiece.
//!
//! The service talks back through one [`Channel`], made once with the
//! [`Phone`] and reused by every call: Tauri never forgets a channel, so a
//! new one per call would leak. Its messages go to the sink of the call
//! running now.
//!
//! The parsing below has no Android in it, so its tests run on the desktop.

use std::sync::{Arc, Mutex};

use call::{AudioRoute, PlatformError, PlatformEvent};
use serde::Deserialize;

#[cfg(target_os = "android")]
pub use phone::{Phone, Slot, init};

/// What `start` answers.
#[derive(Deserialize, Default, Debug, PartialEq)]
struct Started {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    reason: String,
}

impl Started {
    fn result(self) -> Result<(), PlatformError> {
        if self.ok {
            return Ok(());
        }
        Err(match self.reason.as_str() {
            "mic-denied" => PlatformError::MicDenied,
            "background" => PlatformError::Background,
            "" => PlatformError::Other("the call service didn't start".into()),
            r => PlatformError::Other(r.to_string()),
        })
    }
}

/// One message from `CallService`.
#[derive(Deserialize)]
struct Event {
    event: String,
    #[serde(default)]
    reason: String,
}

/// A service message as a call event; None for one this build doesn't know.
fn event(json: &str) -> Option<PlatformEvent> {
    let e: Event = serde_json::from_str(json).ok()?;
    match e.event.as_str() {
        "hangup" => Some(PlatformEvent::HangUp),
        "failed" => Some(PlatformEvent::Failed(if e.reason.is_empty() {
            "the call service stopped".into()
        } else {
            e.reason
        })),
        _ => None,
    }
}

/// The name `route` takes.
fn route_name(to: AudioRoute) -> &'static str {
    match to {
        AudioRoute::Speaker => "speaker",
        AudioRoute::Earpiece => "earpiece",
    }
}

/// The sink of the call running now; None between calls.
type Current = Arc<Mutex<Option<call::PlatformSink>>>;

/// Hand a service message to the current call, if there is one.
fn deliver(current: &Current, json: &str) {
    let Some(ev) = event(json) else { return };
    let sink = current.lock().ok().and_then(|g| g.clone());
    if let Some(sink) = sink {
        sink(ev);
    }
}

#[cfg(target_os = "android")]
mod phone {
    use std::sync::{Arc, OnceLock};

    use call::{AudioRoute, PlatformError, PlatformSink};
    use serde::{Deserialize, Serialize};
    use tauri::Wry;
    use tauri::ipc::{Channel, InvokeResponseBody};
    use tauri::plugin::{Builder, PluginHandle, TauriPlugin};

    use super::{Current, Started, deliver, route_name};

    /// The Kotlin plugin, once the app has set it up.
    pub type Slot = Arc<OnceLock<PluginHandle<Wry>>>;

    #[derive(Serialize)]
    struct Start<'a> {
        title: &'a str,
        events: Channel<serde_json::Value>,
    }

    #[derive(Serialize)]
    struct Route<'a> {
        to: &'a str,
    }

    /// What `route` answers.
    #[derive(Deserialize, Default)]
    struct Routed {
        #[serde(default)]
        ok: bool,
    }

    /// The call layer's platform on the phone.
    ///
    /// Never call its methods from Android's main thread: the Kotlin side
    /// answers there, so a call from it waits on itself forever. The call
    /// layer calls them from its worker threads.
    pub struct Phone {
        slot: Slot,
        events: Channel<serde_json::Value>,
        current: Current,
    }

    impl Phone {
        /// A phone over `slot`, filled by [`init`]'s plugin.
        pub fn new(slot: Slot) -> Self {
            let current: Current = Arc::default();
            let events = Channel::new({
                let current = current.clone();
                move |body: InvokeResponseBody| {
                    // The service sends from Android's main thread, and a
                    // sink may well call end(): hand it on from another.
                    if let InvokeResponseBody::Json(json) = body {
                        let current = current.clone();
                        std::thread::spawn(move || deliver(&current, &json));
                    }
                    Ok(())
                }
            });
            Phone {
                slot,
                events,
                current,
            }
        }

        fn plugin(&self) -> Result<&PluginHandle<Wry>, String> {
            self.slot
                .get()
                .ok_or_else(|| "the call plugin isn't loaded".to_string())
        }

        fn set_current(&self, sink: Option<PlatformSink>) {
            if let Ok(mut g) = self.current.lock() {
                *g = sink;
            }
        }
    }

    impl call::Platform for Phone {
        fn begin(&self, title: &str, events: PlatformSink) -> Result<(), PlatformError> {
            let h = self.plugin().map_err(PlatformError::Other)?;
            // Set before starting: the service can fail at once.
            self.set_current(Some(events));
            let r: Result<Started, _> = h.run_mobile_plugin(
                "start",
                Start {
                    title,
                    events: self.events.clone(),
                },
            );
            let r = match r {
                Ok(s) => s.result(),
                Err(e) => Err(PlatformError::Other(e.to_string())),
            };
            if r.is_err() {
                self.set_current(None);
            }
            r
        }

        fn end(&self) {
            self.set_current(None);
            if let Ok(h) = self.plugin() {
                let r: Result<(), _> = h.run_mobile_plugin("stop", ());
                if let Err(e) = r {
                    eprintln!("irori: call: stop: {e}");
                }
            }
        }

        fn route(&self, to: AudioRoute) -> Result<(), String> {
            let h = self.plugin()?;
            let r: Routed = h
                .run_mobile_plugin("route", Route { to: route_name(to) })
                .map_err(|e| e.to_string())?;
            if r.ok {
                Ok(())
            } else {
                Err(format!("no {} on this phone", route_name(to)))
            }
        }
    }

    /// The plugin: on setup it registers the Kotlin class into `slot`.
    pub fn init(slot: Slot) -> TauriPlugin<Wry> {
        Builder::new("iroricall")
            .setup(move |_app, api| {
                let h = api.register_android_plugin("com.stephenshorton.irori", "CallPlugin")?;
                let _ = slot.set(h);
                Ok(())
            })
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_reply_maps_to_errors() {
        let s = |j: &str| serde_json::from_str::<Started>(j).unwrap().result();
        assert_eq!(s(r#"{"ok":true}"#), Ok(()));
        assert_eq!(
            s(r#"{"ok":false,"reason":"mic-denied"}"#),
            Err(PlatformError::MicDenied)
        );
        assert_eq!(
            s(r#"{"ok":false,"reason":"background"}"#),
            Err(PlatformError::Background)
        );
        assert_eq!(
            s(r#"{"ok":false,"reason":"boom"}"#),
            Err(PlatformError::Other("boom".into()))
        );
        assert!(matches!(s("{}"), Err(PlatformError::Other(_))));
    }

    #[test]
    fn service_messages_become_events() {
        assert_eq!(event(r#"{"event":"hangup"}"#), Some(PlatformEvent::HangUp));
        assert_eq!(
            event(r#"{"event":"failed","reason":"no mic"}"#),
            Some(PlatformEvent::Failed("no mic".into()))
        );
        assert!(matches!(
            event(r#"{"event":"failed"}"#),
            Some(PlatformEvent::Failed(r)) if !r.is_empty()
        ));
        assert_eq!(event(r#"{"event":"later"}"#), None);
        assert_eq!(event("not json"), None);
    }

    #[test]
    fn routes_have_the_plugins_names() {
        assert_eq!(route_name(AudioRoute::Speaker), "speaker");
        assert_eq!(route_name(AudioRoute::Earpiece), "earpiece");
    }

    #[test]
    fn messages_reach_only_the_current_call() {
        let got: Arc<Mutex<Vec<PlatformEvent>>> = Arc::default();
        let current: Current = Arc::default();
        deliver(&current, r#"{"event":"hangup"}"#);
        let sink: call::PlatformSink = {
            let got = got.clone();
            Arc::new(move |e| got.lock().unwrap().push(e))
        };
        *current.lock().unwrap() = Some(sink);
        deliver(&current, r#"{"event":"hangup"}"#);
        *current.lock().unwrap() = None;
        deliver(&current, r#"{"event":"hangup"}"#);
        assert_eq!(*got.lock().unwrap(), vec![PlatformEvent::HangUp]);
    }
}
