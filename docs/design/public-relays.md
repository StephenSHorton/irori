# Public relays as a fallback (card #328)

*2026-11-03. Stephen chose "the Toru way": when no member's home server can be reached, devices meet through n0's free public iroh relays, as Toru meets through public MQTT brokers and STUN. This reverses part of decision 33 ("no third party"); the rest of it stands.*

## What changes

- **Relays only.** The four n0 production relays (`iroh_relay::defaults`, `use1-1`, `usw1-1`, `euc1-1`, `aps1-1` `.relay.n0.iroh.link`) join every endpoint's relay map. n0's DNS/pkarr address directory stays off: nothing is published under a device id, TXT lookups stay refused, and `dns.iroh.link` is never resolved.
- **Every device meets on n0.** iroh keeps one home relay, picked by latency, and an endpoint can be reached only on its home relay: a non-home relay connection closes after 60 s idle (`RELAY_INACTIVE_CLEANUP_TIME`, iroh 1.4.1). If a home server's relay sat in the map, the PC would always home on its own relay (it is local) and the Mac at home would home on the PC's. Neither could then be reached from outside, so an invite from either would fail. So with the switch on, **the relay map holds only n0's four relays**, and home-server relays are left out of it. They stay known (pins, `board.server`, `PeerAddr.relay`), and iroh still dials them on demand: a relay outside the map gets an actor too. When the internet is out and no n0 relay answers, `home_relay_watch` puts the home relays back into the map, so the Mac and PC keep syncing at home.
- **What home servers still do:** the LAN lookup (UDP 38419), direct addresses, store-and-forward for boards, pairing guests, and the relay for anyone with the switch off.
- **Finding a peer with no directory.** Every device homes on the nearest n0 relay. When dialing another device, the dialer adds all four n0 URLs to its `EndpointAddr`, next to its `board.server` relay and direct addresses. iroh tries every relay address it is given (`to_transports_addr`), and one of them is the peer's home relay. This applies to the mesh, board, invite and pairing dials alike. Once connected, iroh hole-punches to a direct path when it can.
- **Invites.** The ticket format is unchanged. The invitee adds the n0 URLs to its dial, so it reaches the inviter on the inviter's n0 home relay. The guest token is still used only for an inviter's own home-server relay.

## Trust

- n0 relays use ordinary web certificates (WebPKI roots). Irori home relays stay pinned to their device keys. The relay TLS verifier picks by host: an n0 relay host gets WebPKI, and anything else gets a device-key pin.
- n0 can't read or inject anything. Connections are QUIC with TLS 1.3, end to end on device keys, and Irori's endpoint gate still admits only certified devices, board members and open invite guests.
- n0 does see device ids (stable), IP addresses, when a device is online, which ids talk to each other, and how much traffic moves. That's the same kind of exposure Toru has with its public brokers.

## Switch

`Use public relays` in Settings, on by default. Off restores decision 33 exactly: the relay map holds home servers only, n0 names are refused, and the no-public-contact test passes. In code: `NetConfig.public_relays`, an `Arc<AtomicBool>`, false by default so the loopback suite never touches the internet.

## Not done here

- Our own hosted relay. The relay list is a setting, so one can replace n0 later.
- The board warning "Can't reach any home server for X" becomes "Can't reach anyone on X". With public relays on, a board can sync even when every home server is unreachable.
