//! The tray menu's lines (Go `TrayController.rebuild`, I:tray.go:30-268;
//! surface.md 3 "Tray"), as plain data, so they are tested without a
//! tray. The menu is rebuilt whole from these on every change.
//!
//! ```text
//! [Restart to update to vX, sep]   only while an update is ready
//! Open Irori
//! <status line>                    disabled
//! [N held messages…]               only while some are held
//! sep
//! [x] Stop everywhere…             once the window opener is wired (card #220);
//!                                  a switch (cards #773, #787), "Stopped
//!                                  everywhere (start)" while on
//! [x] Launch at login
//! sep
//! Quit Irori
//! ```
//!
//! The old "Pause wakes on <device>" checkbox is gone (card #787): Stop
//! everywhere is the one switch the person sees.

/// Menu item ids, shared by the builder and the click handler.
pub mod id {
    pub const UPDATE: &str = "update";
    pub const OPEN: &str = "open";
    pub const STATUS: &str = "status";
    pub const HELD: &str = "held";
    pub const WIND_DOWN: &str = "wind_down";
    pub const LOGIN: &str = "login";
    pub const QUIT: &str = "quit";
}

/// Go `InboxCounts` as the tray reads it (inbox:changed's payload).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub unread: usize,
    pub held: usize,
}

/// Everything one rebuild reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Model {
    pub version: String,
    /// The roster line; empty is the version line.
    pub status: String,
    pub counts: Counts,
    /// Cards waiting on a decision, across devices.
    pub decide: usize,
    /// Whether the held line opens the Held list (Go `SetHeldOpener`);
    /// without an opener it is disabled.
    pub can_open_held: bool,
    /// Whether "Stop everywhere…" opens the window's dialog (card #220);
    /// without an opener it is hidden.
    pub can_wind_down: bool,
    /// Stop everywhere's switch (cards #773, #787; Wind down in the code):
    /// on holds agent starts on every machine; the line is ticked and a
    /// click starts everywhere.
    pub wind_down_on: bool,
    /// The version of an update that is downloaded and ready.
    pub update_ready: Option<String>,
    /// The live launch-at-login state.
    pub login: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Item {
        id: &'static str,
        label: String,
        enabled: bool,
    },
    Check {
        id: &'static str,
        label: String,
        checked: bool,
    },
    Separator,
}

/// Go `versionLine`.
pub fn version_line(version: &str) -> String {
    if version.is_empty() || version == "dev" {
        "Irori (dev build)".to_string()
    } else {
        format!("Irori v{version}")
    }
}

/// Go `statusLine`: status plus ` · N need a decision · N unread` for the
/// nonzero counts. Held messages get their own line.
pub fn status_line(status: &str, c: Counts, decide: usize) -> String {
    let mut parts = vec![status.to_string()];
    if decide > 0 {
        parts.push(format!("{decide} need a decision"));
    }
    if c.unread > 0 {
        parts.push(format!("{} unread", c.unread));
    }
    parts.join(" · ")
}

/// Go `heldLine`.
pub fn held_line(n: usize) -> String {
    if n == 1 {
        "1 held message…".to_string()
    } else {
        format!("{n} held messages…")
    }
}

/// Go `RosterService.trayStatus`'s line for `agents` local agents of
/// which `busy` have status "busy".
pub fn agents_line(agents: usize, busy: usize) -> String {
    match agents {
        0 => "No agents running".to_string(),
        1 => format!("1 agent ({busy} busy)"),
        n => format!("{n} agents ({busy} busy)"),
    }
}

// The words for the switches (cards #773, #787), together; the window's
// copy is frontend/src/lib/stop-labels.ts. This machine's Stop here isn't
// in the tray yet.

/// The tray's Stop everywhere (card #220; Wind down in the code): opens
/// the window's dialog, where the person confirms.
pub const WIND_DOWN_LABEL: &str = "Stop everywhere…";
/// The same line while Stop everywhere is on (card #787): a click starts
/// everywhere again.
pub const WIND_DOWN_ON_LABEL: &str = "Stopped everywhere (start)";

/// Go `rebuild`: the whole menu.
pub fn lines(m: &Model) -> Vec<Line> {
    let mut out = Vec::new();
    if let Some(v) = &m.update_ready {
        out.push(Line::Item {
            id: id::UPDATE,
            label: format!("Restart to update to v{v}"),
            enabled: true,
        });
        out.push(Line::Separator);
    }
    out.push(Line::Item {
        id: id::OPEN,
        label: "Open Irori".into(),
        enabled: true,
    });
    let status = if m.status.is_empty() {
        version_line(&m.version)
    } else {
        m.status.clone()
    };
    out.push(Line::Item {
        id: id::STATUS,
        label: status_line(&status, m.counts, m.decide),
        enabled: false,
    });
    if m.counts.held > 0 {
        out.push(Line::Item {
            id: id::HELD,
            label: held_line(m.counts.held),
            enabled: m.can_open_held,
        });
    }
    out.push(Line::Separator);
    if m.can_wind_down {
        let label = if m.wind_down_on {
            WIND_DOWN_ON_LABEL
        } else {
            WIND_DOWN_LABEL
        };
        out.push(Line::Check {
            id: id::WIND_DOWN,
            label: label.into(),
            checked: m.wind_down_on,
        });
    }
    out.push(Line::Check {
        id: id::LOGIN,
        label: "Launch at login".into(),
        checked: m.login,
    });
    out.push(Line::Separator);
    out.push(Line::Item {
        id: id::QUIT,
        label: "Quit Irori".into(),
        enabled: true,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Go TestTrayLines.
    #[test]
    fn tray_lines_like_go() {
        let c = |unread, held| Counts { unread, held };
        assert_eq!(status_line("2 agents", c(0, 0), 0), "2 agents");
        assert_eq!(status_line("2 agents", c(1, 0), 0), "2 agents · 1 unread");
        assert_eq!(
            status_line("2 agents", c(1, 2), 3),
            "2 agents · 3 need a decision · 1 unread"
        );
        assert_eq!(
            status_line("2 agents", c(0, 0), 1),
            "2 agents · 1 need a decision"
        );
        assert_eq!(held_line(1), "1 held message…");
        assert_eq!(held_line(3), "3 held messages…");
    }

    #[test]
    fn version_and_agents_lines() {
        assert_eq!(version_line("dev"), "Irori (dev build)");
        assert_eq!(version_line(""), "Irori (dev build)");
        assert_eq!(version_line("0.6.0"), "Irori v0.6.0");
        assert_eq!(agents_line(0, 0), "No agents running");
        assert_eq!(agents_line(1, 1), "1 agent (1 busy)");
        assert_eq!(agents_line(3, 2), "3 agents (2 busy)");
    }

    #[test]
    fn whole_menu_in_go_order() {
        let bare = lines(&Model {
            version: "dev".into(),
            ..Model::default()
        });
        let labels: Vec<String> = bare
            .iter()
            .map(|l| match l {
                Line::Item { label, .. } | Line::Check { label, .. } => label.clone(),
                Line::Separator => "--".into(),
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Open Irori",
                "Irori (dev build)",
                "--",
                "Launch at login",
                "--",
                "Quit Irori"
            ]
        );

        let full = lines(&Model {
            version: "0.6.0".into(),
            status: "2 agents (1 busy)".into(),
            counts: Counts { unread: 4, held: 2 },
            decide: 1,
            can_open_held: false,
            can_wind_down: true,
            wind_down_on: false,
            update_ready: Some("0.6.1".into()),
            login: true,
        });
        assert_eq!(
            full,
            vec![
                Line::Item {
                    id: id::UPDATE,
                    label: "Restart to update to v0.6.1".into(),
                    enabled: true
                },
                Line::Separator,
                Line::Item {
                    id: id::OPEN,
                    label: "Open Irori".into(),
                    enabled: true
                },
                Line::Item {
                    id: id::STATUS,
                    label: "2 agents (1 busy) · 1 need a decision · 4 unread".into(),
                    enabled: false
                },
                Line::Item {
                    id: id::HELD,
                    label: "2 held messages…".into(),
                    enabled: false
                },
                Line::Separator,
                Line::Check {
                    id: id::WIND_DOWN,
                    label: WIND_DOWN_LABEL.into(),
                    checked: false
                },
                Line::Check {
                    id: id::LOGIN,
                    label: "Launch at login".into(),
                    checked: true
                },
                Line::Separator,
                Line::Item {
                    id: id::QUIT,
                    label: "Quit Irori".into(),
                    enabled: true
                },
            ]
        );
    }

    // Card #787: while Stop everywhere is on its line is ticked and starts.
    #[test]
    fn wind_down_line_is_a_switch() {
        let line = |on| {
            lines(&Model {
                can_wind_down: true,
                wind_down_on: on,
                ..Model::default()
            })
            .into_iter()
            .find(|l| {
                matches!(
                    l,
                    Line::Check {
                        id: id::WIND_DOWN,
                        ..
                    }
                )
            })
        };
        assert_eq!(
            line(false),
            Some(Line::Check {
                id: id::WIND_DOWN,
                label: WIND_DOWN_LABEL.into(),
                checked: false
            })
        );
        assert_eq!(
            line(true),
            Some(Line::Check {
                id: id::WIND_DOWN,
                label: WIND_DOWN_ON_LABEL.into(),
                checked: true
            })
        );
    }
}
