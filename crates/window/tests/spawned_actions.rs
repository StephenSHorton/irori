//! Attach and Logs on an agent Irori started (card #456, sa-2): only for a
//! spawned agent, and only on the device it runs on.

mod common;

use common::*;

#[test]
fn attach_and_logs_refuse_an_agent_irori_didnt_start_or_another_devices() {
    let r = Rig::new();
    let w = &r.w;
    let me = w.core.device_id().to_string();

    let elsewhere = "other-device/0a1b2c3d-0000";
    for e in [
        w.agents_logs(elsewhere).unwrap_err(),
        w.agents_attach(elsewhere, 80, 24).unwrap_err(),
    ] {
        assert!(e.contains("device it runs on"), "{e}");
    }

    // A terminal agent here (the rig's alpha) wasn't started by Irori.
    for e in [
        w.agents_logs(&r.key_a).unwrap_err(),
        w.agents_attach(&r.key_a, 80, 24).unwrap_err(),
    ] {
        assert!(e.contains("wasn't started by Irori"), "{e}");
    }
    let unknown = format!("{me}/0a1b2c3d-1111");
    assert!(w.agents_logs(&unknown).unwrap_err().contains("wasn't started"));
}
