//! B1 through two Cores (card #63, v0.5-shared-boards §6.2): two made-up
//! people, each at a "home" with its own home server, share a board by
//! invite. Each server writes its `board.server` row into the board, so
//! after both Cores restart, with the invite's address hints gone and no
//! LAN lookup, each device reaches the other through the other person's
//! home relay, which admits it as a member of a board it holds. Every
//! socket is on 127.0.0.1.

mod common;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use irori_core::{Core, devices};
use irori_net::endpoint::IpMode;
use irori_net::transport::IrohNet;

fn wait(what: &str, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "timed out waiting for: {what}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A Core on loopback iroh with no LAN lookup and a home server on
/// 127.0.0.1.
fn core(data: &Path, claude: &Path) -> Core {
    let mut o = common::options(data, claude);
    o.pair_net = Some(Arc::new(move |ident, chain, guests| {
        let mut nc = devices::default_net_config(ident, chain, guests);
        nc.ip = IpMode::Loopback;
        nc.lan_config = None;
        nc.gossip = false;
        IrohNet::new(nc)
    }));
    o.home = Some(Arc::new(|hc: &mut home::Config| {
        hc.bind_ip = Some(Ipv4Addr::LOCALHOST.into());
    }));
    Core::start(o).unwrap()
}

fn new_task(title: &str) -> coord::NewTask {
    coord::NewTask {
        title: title.into(),
        ..Default::default()
    }
}

fn has_task(c: &Core, board: &str, id: &str) -> bool {
    c.board_coord(board)
        .is_ok_and(|co| co.get_task(id).is_some())
}

/// The relay port of `c`'s home server, once it runs.
fn relay_port(c: &Core) -> u16 {
    let mut port = None;
    wait("the home server runs", || {
        port = c.home().and_then(|h| h.relay_port());
        port.is_some()
    });
    port.unwrap()
}

/// Whether `c`'s copy of the board holds `server`'s board.server row with
/// a public address.
fn knows_server(c: &Core, board: &str, server: &str) -> bool {
    let (Some(log), Some(v)) = (c.board_store(board), c.board_verifier(board)) else {
        return false;
    };
    mesh::boardmesh::board_servers(&log, &v)
        .unwrap_or_default()
        .iter()
        .any(|s| s.device_id == server && s.public.is_some())
}

fn connected(c: &Core, board: &str, peer: &str) -> bool {
    c.net().board_conns(board).iter().any(|x| x.peer == peer)
}

#[test]
fn two_homes_reconnect_after_both_restart() {
    let dirs: Vec<tempfile::TempDir> = (0..4).map(|_| tempfile::tempdir().unwrap()).collect();
    let paths: Vec<PathBuf> = dirs.iter().map(|d| d.path().to_path_buf()).collect();
    let a = core(&paths[0], &paths[1]);
    let b = core(&paths[2], &paths[3]);
    let (ad, bd) = (a.device_id().to_string(), b.device_id().to_string());

    // A board between the two people, by invite.
    let board = a.create_board("MLG & co", "mlg").unwrap();
    let (ra, rb) = (a.pairing_runtime().clone(), b.pairing_runtime().clone());
    let link = ra.block_on(a.invite(&board)).unwrap().link;
    rb.block_on(b.join_invite(&link)).unwrap();
    wait("the words", || {
        a.invite_status().status.state == "words" && b.invite_status().status.state == "words"
    });
    let words = b.invite_status().status.words.join(" ");
    a.confirm_invite_words(&words).unwrap();
    wait("the invite is done", || {
        a.invite_status().status.state == "done" && b.invite_status().status.state == "done"
    });

    // A board with another person on it runs each home server (B1); each
    // names its address, here 127.0.0.1 and its relay port.
    for c in [&a, &b] {
        c.set_home_server(true, true, "").unwrap();
    }
    let (pa, pb) = (relay_port(&a), relay_port(&b));
    a.set_home_server(true, true, &format!("127.0.0.1:{pa}"))
        .unwrap();
    b.set_home_server(true, true, &format!("127.0.0.1:{pb}"))
        .unwrap();
    wait("both servers' board.server rows are on both sides", || {
        [&a, &b]
            .iter()
            .all(|c| knows_server(c, &board, &ad) && knows_server(c, &board, &bd))
    });
    // The other person's server is fed as a relay for that person's
    // devices.
    wait("b knows a's server as a board relay", || {
        b.net()
            .board_servers()
            .iter()
            .flat_map(|r| r.servers.clone())
            .any(|s| s.url.port() == Some(pa))
    });

    // Both restart: the invite's in-memory hints are gone, and there is no
    // LAN lookup.
    a.close().unwrap();
    b.close().unwrap();
    let a = core(&paths[0], &paths[1]);
    let b = core(&paths[2], &paths[3]);
    assert_eq!(relay_port(&a), pa, "a's server moved port");
    assert_eq!(relay_port(&b), pb, "b's server moved port");
    // Each endpoint homes on its own server's relay, even though the server
    // came up while the endpoint was binding.
    for (c, port) in [(&a, pa), (&b, pb)] {
        wait("the endpoint homes on its own relay", || {
            c.net()
                .home_relays()
                .iter()
                .any(|(u, up)| *up && u.port() == Some(port))
        });
    }
    wait("the two devices reconnect on the board", || {
        connected(&a, &board, &bd) && connected(&b, &board, &ad)
    });
    // a's relay serves b's device as a member of a board a holds.
    let bkey = iroh::EndpointId::from_bytes(&b.identity().device_pub()).unwrap();
    let akey = iroh::EndpointId::from_bytes(&a.identity().device_pub()).unwrap();
    wait("each relay serves the other person's device", || {
        a.home().unwrap().relay_clients().contains(&bkey)
            || b.home().unwrap().relay_clients().contains(&akey)
    });

    let me_a = format!("person:{}", a.identity().current().id);
    let t = a
        .board_coord(&board)
        .unwrap()
        .create_task(&me_a, new_task("after the restart"))
        .unwrap();
    a.kick_board(&board);
    wait("b gets a's card", || has_task(&b, &board, &t.id));
    let me_b = format!("person:{}", b.identity().current().id);
    let u = b
        .board_coord(&board)
        .unwrap()
        .create_task(&me_b, new_task("and back"))
        .unwrap();
    b.kick_board(&board);
    wait("a gets b's card", || has_task(&a, &board, &u.id));
    // The private meshes never met.
    assert!(!a.net().conns().iter().any(|c| c.peer == bd));
    assert!(!b.net().conns().iter().any(|c| c.peer == ad));
    a.close().unwrap();
    b.close().unwrap();
}

/// fennel's live dry run (#48): the inviter's home server is on, with
/// outside reach off and no public address, and the Core restarts before any
/// board has another person on it. Nothing else wants the server then; an
/// open invite does (as a pairing window does), so the endpoint homes on its
/// own relay and the link names it for an invitee away from home.
#[test]
fn an_open_invite_starts_the_home_server_and_the_link_names_its_relay() {
    let dirs: Vec<tempfile::TempDir> = (0..2).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = core(dirs[0].path(), dirs[1].path());
    a.set_home_server(true, false, "").unwrap();
    a.close().unwrap();
    let a = core(dirs[0].path(), dirs[1].path());
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        a.home().and_then(|h| h.relay_port()),
        None,
        "the server runs with nothing wanting it"
    );
    let board = a.create_board("MLG", "mlg").unwrap();
    let rt = a.pairing_runtime().clone();
    let st = rt.block_on(a.invite(&board)).unwrap();
    let port = a
        .home()
        .and_then(|h| h.relay_port())
        .expect("the server runs for the invite");
    let t = irori_invite::Ticket::parse(&st.link).unwrap();
    assert_eq!(
        t.relay,
        format!("https://127.0.0.1:{port}/"),
        "the link's relay"
    );
    assert_eq!(
        t.relay_key,
        hex::encode(a.identity().device_pub()),
        "the link pins the inviter's own server"
    );
    assert!(
        a.net()
            .home_relays()
            .iter()
            .any(|(u, up)| *up && u.port() == Some(port)),
        "the endpoint isn't on its own relay"
    );
    // The invite ends: nothing wants the server any more.
    a.cancel_invite();
    wait("the server stops with the invite", || {
        a.home().and_then(|h| h.relay_port()).is_none()
    });
    a.close().unwrap();
}

/// The link names the inviter's relay as another device reaches it, not the
/// 127.0.0.1 the endpoint itself homes on (fennel, #48): with an address for
/// outside home, that address (here a typed one from the documentation
/// range, never dialled).
#[test]
fn the_link_names_the_relay_as_others_reach_it() {
    let dirs: Vec<tempfile::TempDir> = (0..2).map(|_| tempfile::tempdir().unwrap()).collect();
    let a = core(dirs[0].path(), dirs[1].path());
    a.set_home_server(true, true, "203.0.113.7:24817").unwrap();
    let board = a.create_board("MLG", "mlg").unwrap();
    let rt = a.pairing_runtime().clone();
    let st = rt.block_on(a.invite(&board)).unwrap();
    wait("the server names its outside address", || {
        a.home()
            .is_some_and(|h| h.status().address == "203.0.113.7:24817")
    });
    let t = irori_invite::Ticket::parse(&st.link).unwrap();
    assert_eq!(t.relay, "https://203.0.113.7:24817/");
    assert_eq!(t.relay_quic_port, 24817);
    assert_eq!(t.relay_key, hex::encode(a.identity().device_pub()));
    a.cancel_invite();
    a.close().unwrap();
}
