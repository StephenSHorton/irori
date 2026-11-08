//! Card #702: every names theme is on until the person turns one off; the
//! switches survive a restart, reach their other devices, and narrow the
//! next name.

use crate::common;

use common::pair::paired;
use common::{Rig, options, wait_for};
use irori_core::Core;

#[test]
fn names_themes_are_on_until_turned_off_and_survive_a_restart() {
    let r = Rig::new();
    assert!(r.core.names_themes_off().is_empty(), "all on (decision 52)");
    assert!(
        r.core.set_names_theme("westerns", false).is_err(),
        "not a theme"
    );
    assert!(r.core.names_themes_off().is_empty());

    for t in coord::names::THEMES {
        if t.id != "myth" {
            r.core.set_names_theme(t.id, false).unwrap();
        }
    }
    assert_eq!(
        r.core.names_themes_off().len(),
        coord::names::THEMES.len() - 1
    );
    let myth = coord::names::theme("myth").unwrap().words;
    let got = r.core.names().assign("theme-session");
    assert!(myth.contains(&got.as_str()), "{got}: only mythology is on");

    r.core.set_names_theme("nature", true).unwrap();
    r.core.set_names_theme("nature", true).unwrap();
    let off = r.core.names_themes_off();
    assert!(!off.contains(&"nature".to_string()) && off.contains(&"anime".to_string()));

    r.core.close().unwrap();
    let again = Core::start(options(&r.data, &r.claude)).unwrap();
    assert_eq!(again.names_themes_off(), off, "after a restart");
    for t in coord::names::THEMES {
        again.set_names_theme(t.id, true).unwrap();
    }
    assert!(again.names_themes_off().is_empty(), "all back on");
    again.close().unwrap();
}

#[test]
fn names_themes_reach_the_persons_other_device() {
    let (a, b) = paired();
    a.core.set_names_theme("anime", false).unwrap();
    wait_for(45, "B hears A's switch", || {
        b.core.names_themes_off() == ["anime"]
    });
    b.core.set_names_theme("anime", true).unwrap();
    wait_for(45, "A hears B's switch", || {
        a.core.names_themes_off().is_empty()
    });
    a.core.close().unwrap();
    b.core.close().unwrap();
}
