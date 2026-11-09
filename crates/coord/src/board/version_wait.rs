//! Waits on an app version (card #485): "Update the Mac to v0.11.0" as
//! a condition Irori can check. The blocker's ref is the least version and
//! the devices that must run it, `0.11.0 <device id> [<device id>…]`, and
//! the sweep closes it once every one of them has written a
//! `device.started` row with that version or a later one.

use gocompat::quote::quote;

use super::error::{Result, invalid};
use super::fold::Fold;

/// How many devices one version wait names at most.
pub const MAX_VERSION_DEVICES: usize = 8;

/// Splits a version wait's ref: the least version (no leading `v`) and the
/// device ids, in order, without repeats.
pub fn parse_version_wait(r: &str) -> (String, Vec<String>) {
    let mut it = r.split_whitespace();
    let min = it
        .next()
        .map(|v| v.strip_prefix('v').unwrap_or(v).to_string())
        .unwrap_or_default();
    let mut devs: Vec<String> = Vec::new();
    for d in it {
        if !devs.iter().any(|x| x == d) {
            devs.push(d.to_string());
        }
    }
    (min, devs)
}

/// Whether `v` reads as `X.Y.Z` (an optional `v`, then three dot-separated
/// numbers, then an optional `-prerelease` or `+build`).
pub fn valid_version(v: &str) -> bool {
    let v = v.strip_prefix('v').unwrap_or(v);
    let num = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = num.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Compares two versions as the updater does (Go `cmpSemver`): numeric
/// `X.Y.Z`, then a release above its prerelease, then prereleases as
/// byte strings. -1, 0 or 1.
pub fn cmp_version(a: &str, b: &str) -> i32 {
    fn parse(v: &str) -> ([u64; 3], &str) {
        let v = v.strip_prefix('v').unwrap_or(v);
        let v = v.split_once('+').map_or(v, |(a, _)| a);
        let (num, pre) = v.split_once('-').unwrap_or((v, ""));
        let mut out = [0u64; 3];
        for (i, p) in num.splitn(3, '.').enumerate() {
            out[i] = p.trim().parse().unwrap_or(0);
        }
        (out, pre)
    }
    let (na, pa) = parse(a);
    let (nb, pb) = parse(b);
    match na.cmp(&nb) {
        std::cmp::Ordering::Greater => return 1,
        std::cmp::Ordering::Less => return -1,
        std::cmp::Ordering::Equal => {}
    }
    match (pa.is_empty(), pb.is_empty()) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => -1,
        _ => match pa.cmp(pb) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        },
    }
}

/// Whether a running version `have` meets the least `min`. A dev build
/// (empty or "dev") never does: it can't say what it has in it.
pub fn version_meets(have: &str, min: &str) -> bool {
    !have.is_empty() && have != "dev" && valid_version(have) && cmp_version(have, min) >= 0
}

impl Fold {
    /// The app version device `dev` last started with: the `version` of
    /// its newest `device.started` row (Options::device_version on a
    /// board, which keeps none), or "" when none is known.
    pub(crate) fn device_version(&self, dev: &str) -> String {
        if let Some(f) = &self.device_version {
            return f(dev);
        }
        let Some(st) = &self.st else {
            return String::new();
        };
        let evs = st
            .list(&store::Filter {
                kinds: vec![store::kinds::DEVICE_STARTED.into()],
                device_id: dev.into(),
                desc: true,
                limit: 1,
                ..store::Filter::default()
            })
            .unwrap_or_default();
        evs.first()
            .and_then(|ev| serde_json::from_slice::<serde_json::Value>(&ev.payload).ok())
            .and_then(|v| {
                v.get("version")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
            .filter(|v| v.len() <= 40)
            .unwrap_or_default()
    }

    /// The devices of a version wait `r` still behind, with what each runs
    /// ("" for none known). A removed device is never waited on (card
    /// #1240): a wait only it held up clears.
    pub(crate) fn version_behind(&self, r: &str) -> Vec<(String, String)> {
        let (min, devs) = parse_version_wait(r);
        let revoked = self.revoked_fn();
        devs.into_iter()
            .filter(|d| !revoked(d))
            .map(|d| {
                let v = self.device_version(&d);
                (d, v)
            })
            .filter(|(_, v)| !version_meets(v, &min))
            .collect()
    }

    /// Validates and normalizes a version wait's ref: `<min> [<device>…]`,
    /// the devices by id; none names this device (the one writing the
    /// blocker). Refused when every device already runs it, as a wait on
    /// a card already done is.
    pub(crate) fn check_version_wait(&self, r: &str) -> Result<String> {
        let (min, mut devs) = parse_version_wait(r);
        if min.is_empty() || !valid_version(&min) {
            return Err(invalid(
                "blocked on a version needs the least app version first, e.g. version 0.11.0",
            ));
        }
        if devs.is_empty() {
            devs.push(self.dev.clone());
        }
        if devs.len() > MAX_VERSION_DEVICES {
            return Err(invalid(format!(
                "blocked on a version names at most {MAX_VERSION_DEVICES} devices"
            )));
        }
        for d in &devs {
            if d.is_empty()
                || d.len() > super::MAX_KEY
                || !d
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(invalid(format!(
                    "blocked on a version: {} is not a device id",
                    quote(d)
                )));
            }
            if self.revoked_fn()(d) {
                return Err(invalid("blocked on a version: that device was removed"));
            }
        }
        let out = format!("{min} {}", devs.join(" "));
        if self.version_behind(&out).is_empty() {
            return Err(invalid(format!(
                "every device named already runs {min} or later: nothing to wait for"
            )));
        }
        Ok(out)
    }
}
