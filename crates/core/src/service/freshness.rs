//! How fresh each log is (card #580, docs/design/no-server.md §4.3): when
//! this device was last in step with another device on the private log and
//! on each shared space, and which device. Irori has no server, so a log is
//! as fresh as the last time a device that holds it was on at the same time
//! as this one; the window says so, so old data doesn't look current.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::Services;

/// The private log's id in [`LogFreshness::log`].
pub const PERSONAL: &str = "personal";

/// One log's freshness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFreshness {
    /// [`PERSONAL`] or a shared space's id.
    pub log: String,
    /// Unix ms when this device was last in step with another on it; 0
    /// never, as far as it knows.
    pub at_ms: i64,
    /// The device it was in step with then; "" with `at_ms` 0.
    pub device_id: String,
    /// That device as the person would say it: "your PC", or for another
    /// member's, whose it is ("Blake").
    pub with: String,
    /// Another device holds this log at all. False: only this device has
    /// it, and there's nothing to say.
    pub shared: bool,
}

/// The newest of `(device, at_ms)`, ties to the first.
fn newest(v: impl IntoIterator<Item = (String, i64)>) -> Option<(String, i64)> {
    v.into_iter()
        .fold(None, |best: Option<(String, i64)>, (d, at)| match best {
            Some((_, b)) if b >= at => best,
            _ => Some((d, at)),
        })
}

/// A device's own word for itself from its OS: "PC", "Mac", "phone".
pub fn os_word(os: &str) -> &'static str {
    match os.to_ascii_lowercase().as_str() {
        "windows" => "PC",
        "macos" | "darwin" | "mac" => "Mac",
        "android" | "ios" => "phone",
        "linux" => "computer",
        _ => "",
    }
}

/// How the person says one of their own devices: its chosen name ("your
/// Studio"), else its kind ("your PC"), else its label.
pub fn own_phrase(chosen: &str, os: &str, label: &str) -> String {
    let name = if !chosen.is_empty() {
        chosen
    } else if !os_word(os).is_empty() {
        os_word(os)
    } else if !label.is_empty() {
        label
    } else {
        "other device"
    };
    // "Stephen's laptop" is already whose it is.
    if name.contains('\'') || name.contains('\u{2019}') {
        name.to_string()
    } else {
        format!("your {name}")
    }
}

/// How the person says another member's device: by whose it is,
/// "Blake". A board's member certs carry no label or OS to say more.
pub fn their_phrase(person: &str) -> String {
    if person.is_empty() {
        "another member".to_string()
    } else {
        person.to_string()
    }
}

impl Services {
    /// Every log's freshness: the private log first, then each shared
    /// space this device shows.
    pub(crate) fn freshness(&self) -> Vec<LogFreshness> {
        let node = &self.mesh.node;
        let chosen = self.device_names.chosen();
        // This person's devices, by id: their phrase.
        let own: BTreeMap<String, String> = node
            .devices(false)
            .into_iter()
            .filter(|d| !d.this)
            .map(|d| {
                let c = chosen.get(&d.device_id).cloned().unwrap_or_default();
                let p = own_phrase(&c, &d.os, &d.name);
                (d.device_id, p)
            })
            .collect();
        let phrase = |dev: &str, theirs: &dyn Fn(&str) -> String| -> String {
            own.get(dev).cloned().unwrap_or_else(|| theirs(dev))
        };

        let mut out = Vec::new();
        let chain = &self.chain;
        let personal: Vec<(String, i64)> = node
            .freshness()
            .into_iter()
            .map(|f| (f.device_id, f.at.map_or(0, unix_ms)))
            .collect();
        let personal = kept(personal, &|d| chain.cut(d).is_some());
        out.push(entry(PERSONAL, &personal, &|d| {
            phrase(d, &|_| String::new())
        }));

        for b in self.shown_boards() {
            let Some(m) = self.mesh.board(&b.id) else {
                continue;
            };
            let ms = b.verifier.membership();
            let mut whose: BTreeMap<String, String> = BTreeMap::new();
            for p in &ms.members {
                let name = b
                    .verifier
                    .profile(&p.person_id)
                    .unwrap_or_else(|| p.name.clone());
                for d in b.verifier.devices(&p.person_id) {
                    whose.insert(d.cert.device_id.clone(), name.clone());
                }
            }
            let theirs = |dev: &str| -> String { their_phrase(whose.get(dev).map_or("", |s| s)) };
            let devs = kept(m.freshness(), &|d| {
                chain.cut(d).is_some() || b.verifier.cut(d).is_some()
            });
            out.push(entry(&b.id, &devs, &|d| phrase(d, &theirs)));
        }
        out
    }
}

/// `devs` without the removed ones (card #1240): a removed device is never
/// the one a log was last in step with, and a log only it shared reads as
/// not shared.
fn kept(devs: Vec<(String, i64)>, removed: &dyn Fn(&str) -> bool) -> Vec<(String, i64)> {
    devs.into_iter().filter(|(d, _)| !removed(d)).collect()
}

fn entry(log: &str, devs: &[(String, i64)], phrase: &dyn Fn(&str) -> String) -> LogFreshness {
    let mut f = LogFreshness {
        log: log.to_string(),
        shared: !devs.is_empty(),
        ..Default::default()
    };
    if let Some((dev, at)) = newest(devs.iter().filter(|(_, at)| *at > 0).cloned()) {
        f.with = phrase(&dev);
        f.device_id = dev;
        f.at_ms = at;
    }
    f
}

fn unix_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases() {
        assert_eq!(own_phrase("", "windows", "DESKTOP-4F2K"), "your PC");
        assert_eq!(own_phrase("", "android", ""), "your phone");
        assert_eq!(own_phrase("Studio", "macos", "x"), "your Studio");
        assert_eq!(own_phrase("Stephen's laptop", "", ""), "Stephen's laptop");
        assert_eq!(own_phrase("", "", "box"), "your box");
        assert_eq!(their_phrase("Blake"), "Blake");
        assert_eq!(their_phrase(""), "another member");
    }

    #[test]
    fn the_newest_device_wins() {
        let f = entry(
            "b",
            &[("a".into(), 5), ("b".into(), 9), ("c".into(), 0)],
            &|d| format!("dev {d}"),
        );
        assert_eq!(f.at_ms, 9);
        assert_eq!(f.device_id, "b");
        assert_eq!(f.with, "dev b");
        assert!(f.shared);
    }

    #[test]
    fn a_removed_device_is_never_the_newest() {
        let devs = vec![("a".into(), 5), ("old".into(), 9)];
        let f = entry("b", &kept(devs, &|d| d == "old"), &|d| format!("dev {d}"));
        assert_eq!((f.device_id.as_str(), f.at_ms), ("a", 5));
        let only = kept(vec![("old".into(), 9)], &|d| d == "old");
        assert!(!entry("b", &only, &|_| "x".into()).shared);
    }

    #[test]
    fn never_in_step_says_so() {
        let f = entry("b", &[("a".into(), 0)], &|_| "x".into());
        assert_eq!(f.at_ms, 0);
        assert!(f.with.is_empty());
        assert!(f.shared);
        let alone = entry(PERSONAL, &[], &|_| "x".into());
        assert!(!alone.shared);
    }
}
