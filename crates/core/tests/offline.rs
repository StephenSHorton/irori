//! The offline Core (`irori_core::offline`, the rehearsal's
//! `IRORI_DEV_OFFLINE`): its transport never binds, so no pairing window
//! opens and nothing is dialed.

mod common;

use irori_core::Core;

#[test]
fn an_offline_core_binds_nothing() {
    let (d, c) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut o = common::options(d.path(), c.path());
    o.pair_net = Some(irori_core::offline::offline_net());
    o.no_home = true;
    let core = Core::start(o).unwrap();
    let rt = core.pairing_runtime().clone();
    let bound = rt.block_on(core.net().bind());
    assert!(bound.is_err(), "the offline transport bound an endpoint");
    let add = rt.block_on(irori_core::devices::add_device(&core, "desktop"));
    assert!(add.is_err(), "a pairing window opened offline: {add:?}");
    assert!(
        core.net().endpoint().is_none(),
        "the offline transport holds an endpoint"
    );
    core.close().unwrap();
}
