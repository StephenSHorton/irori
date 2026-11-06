# Voice calls in Irori: what we found (card #571, 2026-11-06)

Huddles in Talk, a transcript anyone in the conversation (and their agents)
can act on, and screen sharing. Nothing is built: this is the exploration the
card asked for before choosing what to build. Rust only.

| File | What it is |
| --- | --- |
| [toru.md](toru.md) | How Toru (Go) does screen sharing, Send Files and separate audio sources, and what carries over |
| [audio-stack.md](audio-stack.md) | Rust libraries for the voice path, compared and fact-checked |
| [screen-share.md](screen-share.md) | Rust libraries for screen capture, encoding and viewing, compared and fact-checked |
| [irori-fit.md](irori-fit.md) | Where calls sit in Irori: spaces, row kinds, the iroh endpoint, transcripts, Whisper |
| [../../img/571](../../img/571) | Mock-up screenshots (the browser preview, `?call=start|live|share|transcript`) |

Each library claim in the two survey files was checked a second time against
crates.io, GitHub or the source by a separate fact-checker; their notes are at
the end of each file.

## Mock-ups

`bun run preview:look`, then `?call=<state>#talk/sample-ch-suzuri`
(frontend/PREVIEW.md). The code is `frontend/src/components/talk/call-mock.tsx`;
nothing in it is wired.

1. `start`: the channel's Huddle button. Who can join, the mic, and "Write a
   transcript", on by default with the notice that everyone who joins is told.
   Another channel shows "Huddle · Spencer, Aiko" in the list.
2. `live`: who's in it, on which device and over which link (direct or
   relay), the transcript as it's written, and a bar above your name that
   follows you to other pages. An agent can sit in as a listener that reads
   the transcript.
3. `share`: Spencer shares one window; it takes the posts' place.
4. `transcript`: the ended huddle's post (summary, decisions, to-dos), the
   full transcript beside it, and an agent making a card from it when asked.
5. Phone width: the call is the whole page.

## Transport: tested

`crates/net/src/bin/spike-call.rs` sent one 160-byte QUIC datagram every 20 ms
(voice-shaped, no real audio) over iroh between the real devices:

| Leg | Path | RTT p50 | p95 | Loss |
| --- | --- | --- | --- | --- |
| Mac ↔ Pixel, same Wi-Fi | direct within 1 s | 12 ms | 17 ms | 0 |
| Mac ↔ PC (double NAT) | direct within 1 s | 26 ms | 269 ms, spikes to 930 ms | 0 |
| Mac ↔ PC, relay only | relay | 124 ms | 656 ms | 0 |
| Pixel ↔ PC | direct within 1 s | 33 ms | 682 ms | 0 |

iroh is good enough for calls: every leg went direct in under a second, and
the LAN is clean. Every leg touching the PC had spikes, relay included, while
the PC was heavily loaded (#559).

### Rerun on an idle PC (#653)

Same probe, same commands, with the PC idle (CPU sampled every 5 s: mostly
20-50%, no builds during runs):

| Leg | Path | RTT p50 | p95 | p99 | Late (>300 ms) | Busy PC p95 |
| --- | --- | --- | --- | --- | --- | --- |
| Mac → PC | direct within 1 s | 3.9 ms | 6.6 ms | 18.6 ms | 0 of 3000 | 269 ms |
| Mac → PC, relay only (n0) | relay | 109 ms | 121 ms | 131 ms | 0 of 3000 | 656 ms |

**The spikes were the busy PC, not the double NAT.** With the PC idle the
direct path through both NATs is as clean as the LAN, and the relay adds only
its fixed ~100 ms (the n0 relay is usw1-1, in the US west). The Pixel legs
were not rerun (the phone wasn't visible on the Mac's USB); the Mac result
already separates load from the NAT, since the Mac crosses the same double
NAT.

Takeaway for the build: a busy PC (agents building) hurts its calls far more
than the network does. The call layer's audio and network threads should not
share a runtime with heavy work, and the jitter buffer should expect 300 ms+
stalls from a loaded machine.

### Screen sharing over relays (#653)

`spike-call dial --share` sends a 1080p 10 fps share's shape: one QUIC stream
per frame, 2.5 Mbps average, a 200 KB keyframe every 2 s (22 KB frames
between), and the voice datagrams running beside it on the same connection.
The receiver acks each whole frame; frame time is send start to ack (one way
plus the ack's return). Like an encoder it would skip frames with 2 s in
flight; no run needed to. `--home-relay` runs the same through an embedded
Irori home relay (the real `irori_net::relay` code) on the PC instead of n0's.

All runs relay only, 60 s, every frame delivered (600 of 600), at most one
lost QUIC packet per run:

| Sharer → viewer | Relay | Rate | Frame p50 / p95 | Keyframe p50 / max | Frames >500 ms | Voice beside it, p95 (late >300 ms) |
| --- | --- | --- | --- | --- | --- | --- |
| Mac → PC | n0 | 2.5 Mbps | 124 / 169 ms | 267 / 814 ms | 1 | 154 ms (0) |
| PC → Mac | n0 | 2.5 Mbps | 122 / 136 ms | 203 / 912 ms | 2 | 126 ms (0) |
| Mac → PC | PC home relay | 2.5 Mbps | 7 / 11 ms | 15 / 45 ms | 0 | 8 ms (0) |
| PC → Mac | PC home relay | 2.5 Mbps | 8 / 13 ms | 15 / 58 ms | 0 | 9 ms (0) |
| Mac → PC | n0 | 8 Mbps | 183 / 531 ms | 569 / 1568 ms | 76 | 488 ms (879) |
| PC → Mac | n0 | 8 Mbps | 123 / 141 ms | 203 / 1508 ms | 8 | 126 ms (0) |
| Mac → PC | PC home relay | 8 Mbps | 10 / 22 ms | 27 / 145 ms | 0 | 10 ms (0) |
| PC → Mac | PC home relay | 8 Mbps | 9 / 14 ms | 23 / 132 ms | 0 | 10 ms (0) |

- **A 1080p 10 fps share works relay only on both relays Irori uses.** n0's
  public relays (the default relay map, [public-relays.md](../../design/public-relays.md))
  carry 2.5 Mbps with no throttling: frames arrive in ~120 ms, keyframes in
  ~270 ms, and voice on the same connection is unaffected. The PC's home relay
  on the LAN adds almost nothing.
- **Headroom:** the home relay carries 8 Mbps as easily as 2.5. Through n0 at
  8 Mbps the PC as sharer is still fine, but the Mac as sharer is not: frames
  queue (p95 530 ms) and voice on the same connection goes late (879 of 3000
  over 300 ms). It reproduced with the PC quiet, and the other direction over
  the same relay is clean, so the limit is the Mac's upload side (its Wi-Fi
  and inner router), not n0. So: cap a share at ~2.5-4 Mbps, keep voice
  datagrams ahead of share streams in the send queue (QUIC stream priority),
  and let the encoder drop its rate when frame times climb.
- Keyframes are the slow frames on n0 (up to ~0.8-1.5 s once in a while). A
  share should send keyframes rarely (on a scene change or a viewer's ask),
  not every 2 s as this test does.
- Not measured: the phone as a viewer, and n0 relays other than usw1-1.

Run it: on the receiver `spike-call listen` (n0) or `spike-call listen
--home-relay-port 25930` (runs the relay; the port needs inbound TCP and UDP
in the firewall), then on the sharer `spike-call dial <id> --relay-only
--share [--home-relay https://<receiver IP>:25930/] [--kbps 8000 --key-kb
600]`. The SUMMARY lines are on card #653.

## Recommendation

**Voice:** our own small call layer on the iroh connection Irori already has.
Opus in QUIC datagrams (a lost packet never holds up later ones), full mesh
for 2 to 6 people.

| Part | Pick | Runner-up | Why not the others |
| --- | --- | --- | --- |
| Transport | iroh 1.4 datagrams, new ALPN `irori/call/1` | n0's iroh-live (MoQ over iroh) | webrtc-rs and str0m bring their own ICE and UDP and bypass iroh; LiveKit needs a Go server; callme and iroh-roq are stuck on old iroh |
| Mic and speaker | cpal 0.18 (already used) | oboe on Android | |
| Echo cancel, noise, gain | sonora 0.2 (pure-Rust WebRTC AEC3) | webrtc-audio-processing (C++) | nnnoiseless is noise only, no echo cancel; DeepFilterNet is heavy |
| Codec | `opus` 0.4 (libopus 1.6.1), FEC and DTX on | | |
| Jitter buffer | neteq 0.9, or a small one of our own | | nothing mature exists |
| Transcript | whisper-rs (already used), each device transcribes only its own mic | | |

iroh-live is the one whole stack built on iroh, by iroh's own team, and the
place to look again later. Today it calls itself an early preview, has never
run on Windows, sends audio on streams (a lost packet stalls the ones after
it) and its MoQ crates change weekly.

**Screen sharing:** capture with `screencapturekit` (Mac) and
`windows-capture` (PC), encode H.264 on each machine's hardware encoder
(moq-video's encoder, or our own on VideoToolbox and Media Foundation), send
over iroh streams, decode in the window with WebCodecs. Relay throughput is
measured above: 2.5 Mbps holds on n0 and the home relay. The Mac needs the
Screen Recording grant, which an ad-hoc-signed build loses on every update:
it waits on Developer ID signing (#537). Phone sharing is a later phase.

**From Toru:** keep each speaker's audio separate and transcribe it on their
own device (free speaker labels), and leave Irori's own audio out of a shared
screen's sound. Toru's calls run on Chromium's WebRTC inside a webview;
Rust-only rules that out for Irori.

## Where things go (irori-fit.md)

- **A huddle belongs to a conversation:** a space channel first, later person
  DMs. Its rows go in the log that conversation already uses.
- **Older Irori:** a space log stops reading a device's rows at an unknown
  kind, and v0.12.4's tolerant reader covers only `task.*` and `space.*`. So
  one release ahead of calls widens it to `call.*` and `talk.*`. Until every
  device has it, the start and end of a huddle ride as ordinary `talk.post`s
  (older Irori shows them as plain posts).
- **Transcript:** each device writes its own speaker's lines, signed. Whoever
  can read the channel can read it; agents read it with `talk_read`. On by
  default with a per-person off switch (decisions.md #52), and a notice
  before the mic opens. No audio is stored.
- **Phone:** it runs the full Core and endpoint, so it joins like a desktop.
  It needs an Android foreground service so the call survives the screen
  turning off.
