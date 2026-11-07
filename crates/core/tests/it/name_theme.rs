//! Card #702: the theme new agents' names come from is every theme mixed
//! until the person picks one, survives a restart, reaches their other
//! devices, and narrows the next name.

use crate::common;

use common::pair::paired;
use common::{Rig, options, wait_for};
use irori_core::Core;

#[test]
fn names_theme_is_mixed_until_picked_and_survives_a_restart() {
    let r = Rig::new();
    assert_eq!(r.core.names_theme(), "", "every theme mixed (decision 52)");
    assert!(r.core.set_names_theme("nope").is_err(), "not a theme");
    assert_eq!(r.core.names_theme(), "");

    r.core.set_names_theme("myth").unwrap();
    assert_eq!(r.core.names_theme(), "myth");
    let myth = coord::names::theme("myth").unwrap().words;
    let got = r.core.names().assign("theme-session");
    assert!(myth.contains(&got.as_str()), "{got}: not from mythology");

    r.core.close().unwrap();
    let again = Core::start(options(&r.data, &r.claude)).unwrap();
    assert_eq!(again.names_theme(), "myth", "after a restart");
    again.set_names_theme("").unwrap();
    assert_eq!(again.names_theme(), "", "back to every theme");
    again.close().unwrap();
}

#[test]
fn names_theme_reaches_the_persons_other_device() {
    let (a, b) = paired();
    a.core.set_names_theme("anime").unwrap();
    wait_for(45, "B hears A's theme", || b.core.names_theme() == "anime");
    b.core.set_names_theme("produce").unwrap();
    wait_for(45, "A hears B's theme", || {
        a.core.names_theme() == "produce"
    });
    a.core.close().unwrap();
    b.core.close().unwrap();
}
