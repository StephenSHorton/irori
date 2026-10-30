# M1 networking spike: running the PC and Mac halves

The PC half (relay + endpoint) and the loopback tests live in `crates/net` (`spike-net`). This page is how the real PC↔Mac run goes: the Mac finds the PC from its own log, with no typed address or id (architecture §8 M1). Results go into `docs/port/mesh.md` §8.1.

Rules for both machines: only copies of data dirs are used, Go Irori keeps running untouched, the spike never writes into a data dir, nothing secret is printed.

## 0. Build

```sh
cargo build -p irori-net --bin spike-net --release
```

On the Mac, build without `--release`: a release build ignores `IRORI_DEV_KEK_DIR` (D14, debug builds only), so it can't unseal a copied key ("Irori's key is missing from the Keychain").

On the Mac this needs the Xcode command-line tools (rusqlite bundles SQLite's C source).

## 1. PC: copy the data dir, start the relay

1. Make a read-only copy of the PC data dir. `irori.db` is in WAL mode, so copy it with SQLite's backup rather than a file copy while Go runs, e.g. `sqlite3 -readonly "%LOCALAPPDATA%\irori\irori.db" ".backup 'C:\scratch\pc\irori.db'"`, then copy `device.json` and `device.key` beside it. DPAPI unseals the copy for the same Windows user.
2. Pick a dev port that is not Go's 25817, e.g. `35817` (TCP and UDP).
3. At home (Mac on the LAN), relay-only first:

   ```sh
   spike-net relay --home C:\scratch\pc --port 35817 --public --allow-db C:\scratch\pc\irori.db
   ```

   `--public` binds the relay (TCP+UDP 35817) and the endpoint's UDP socket on 0.0.0.0, so Windows Firewall asks once for this exe; allow Private networks only. Without `--public` everything stays on 127.0.0.1 and the Mac cannot reach it. `--allow-db` admits every device certified for this PC's `personId` in the copied log (no signature check in the spike); `--allow <devicePub>` adds one by hand.

   It prints `endpoint <hex devicePub>`, the relay addresses, the home-relay status, then one line per second per connection: `path=relay|direct rtt=… via=…`.

4. For an away-from-home run, forward TCP and UDP 35817 on the router to the PC, as Go's port N is.

## 2. Mac: copy, look the PC up in the log, dial

1. Stop nothing; copy `~/.irori/irori.db` with `sqlite3 -readonly ~/.irori/irori.db ".backup '/tmp/m1/irori.db'"`, and copy `device.json` and `device.key` to `/tmp/m1/`.
2. The device key is sealed with the Keychain. The KEK's account is named after the data dir path, so a copy needs the original path:

   ```sh
   export IRORI_DEV_KEK_DIR=$HOME/.irori
   ```

   (For example `kek-8a2f5e0399ac9edc` for `/Users/pat/.irori`.) Record whether macOS shows a Keychain prompt for `/usr/bin/security` when a new binary asks; it should not, because the spike reads through the same `security` tool that created the item, but this is one of the M1 questions.
3. Relay-only:

   ```sh
   spike-net dial --home /tmp/m1 --from-db /tmp/m1/irori.db --port 35817
   ```

   `--from-db` opens the copy read-only, takes the newest `node.addr` row not written by this Mac and not `reach: off` (the PC's), and the newest `person.device.added` cert for that device id. Host = the row's first `lan` IPv4 (then v6, then `public.ip` when `reach` is `ok`), relay URL = `https://<host>:<port>`, and the relay and endpoint are both pinned to the cert's `devicePub`, which is the PC's EndpointId (D2). `--port` overrides the row's port (Go's 25817) with the dev port; `--host <ip>` picks another address from the printed list (for example the public IP from outside).

   Expected: `connected in …`, then every second `echo N ok … path=relay rtt=…`.
4. Direct paths allowed:

   ```sh
   spike-net dial --home /tmp/m1 --from-db /tmp/m1/irori.db --port 35817 --direct --secs 60
   ```

   `--direct` binds UDP 0.0.0.0 on the Mac (macOS may ask to allow incoming connections). The path starts as `relay` and turns `direct` if hole punching works across the Mac's double NAT (its router sits behind a second router).

## 3. What to record in mesh.md §8.1

- The `from-db` line: which LAN address the PC's `node.addr` lists, and so which subnet the PC is on relative to the Mac's inner and outer subnets (double NAT).
- Relay-only works at home: yes/no, RTT.
- Direct allowed: did it reach `direct`, after how long, RTT before and after.
- From outside (phone hotspot on the Mac, `--host <public ip>`): relay-only and direct results.
- Keychain prompt: yes/no.

## 4. Loopback rehearsal on one PC

```sh
spike-net fixture --out C:\scratch\a        # throwaway key, DPAPI-sealed, prints its endpoint id
spike-net fixture --out C:\scratch\b
spike-net relay --home C:\scratch\a --port 45817 --allow <b's id>
spike-net dial  --home C:\scratch\b --relay https://127.0.0.1:45817/ --to <a's id>                    # path=relay
spike-net dial  --home C:\scratch\b --relay https://127.0.0.1:45817/ --to <a's id> --direct-loopback   # relay, then direct
```

A third fixture that is not `--allow`ed is denied by the relay and its dial times out. `fixture` is Windows (DPAPI) and Linux (file keystore) only; on macOS it would need a Keychain KEK and refuses.
