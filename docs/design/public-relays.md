# Public relays as a fallback (card #328)

*2026-11-03. Stephen chose "the Toru way": when no member's home server can be reached, devices meet through n0's free public iroh relays, as Toru meets through public MQTT brokers and STUN. This reverses part of decision 33 ("no third party"); the rest of it stands.*

## What changes

- **Relays only.** The four n0 production relays (`iroh_relay::defaults`, `use1-1`, `usw1-1`, `euc1-1`, `aps1-1` `.relay.n0.iroh.link`) join every endpoint's relay map. n0's DNS/pkarr address directory stays off: nothing is published under a device id, TXT lookups stay refused, and `dns.iroh.link` is never resolved.
- **Home servers still win when they work.** iroh picks one home relay by latency (with hysteresis). At home, the Mac's home relay stays the PC; a device with no reachable home server settles on the nearest n0 relay. No ordering code is needed.
- **Finding a peer with no directory.** An endpoint receives only on its home relay (iroh 1.4.1 closes a non-home relay connection after 60 s idle, `RELAY_INACTIVE_CLEANUP_TIME`). So when dialing another device, the dialer adds all four n0 URLs to the peer's `EndpointAddr`, next to its `board.server` relays and direct addresses. One of them is the peer's home relay; iroh's multipath tries them all.
- **Who dials.** It's unchanged: the lower id dials first, and the higher one dials after `dial_grace`. So when one side's home relay can't be reached (the PC behind double NAT), the other side still connects through n0.
- **Invites.** The ticket also carries the inviter's current home relay URLs (`home_relay_status`), so a guest can dial through n0.

## Trust

- n0 relays use ordinary web certificates (WebPKI roots). Irori home relays stay pinned to their device keys. The relay TLS verifier picks by host: an n0 relay host gets WebPKI, and anything else gets a device-key pin.
- n0 can't read or inject anything. Connections are QUIC with TLS 1.3, end to end on device keys, and Irori's endpoint gate still admits only certified devices, board members and open invite guests.
- n0 does see device ids (stable), IP addresses, when a device is online, which ids talk to each other, and how much traffic moves. That's the same kind of exposure Toru has with its public brokers.

## Switch

`Use public relays` in Settings, on by default. Off restores decision 33 exactly: the relay map holds home servers only, n0 names are refused, and the no-public-contact test passes.

## Not done here

- Our own hosted relay. The relay list is a setting, so one can replace n0 later.
- The board warning "Can't reach any home server for X" becomes "Can't reach anyone on X". With public relays on, a board can sync even when every home server is unreachable.
