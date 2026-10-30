# Port spec: identity, pairing, device, owner

Source (Go, read-only): `../irori-go/internal/{identity,pair,device,owner}` (Go be67a86).
All `file:line` references are relative to `irori/internal/` unless they start with `go-room/`.
Scope: key types and files, sealing at rest, person certs/revokes/rotations/epochs, the exact bytes
every person signature covers, the private cert fold (`Chain`), the board membership fold and board
verifier, board secrets (HKDF), the pairing protocol, the owner table, and `turn.key`.

Golden vectors in §7 were produced on 2026-10-29 by a standalone Go 1.26.5 program that copies the
Go code's stdlib calls verbatim, plus one root-cert row (a made-up fixture identity, signed with a fixed-seed key) that the Go build accepts.
Port them as unit tests first.

---

## 1. What the area does and its interface

* `device` keeps `device.json`: a random device id, display name, GOOS, and from v0.4 the person id and
  device public key (`device/device.go:21-32`).
* `identity` owns every private key. It loads or makes `device.key` (ed25519 seed) and `person.json`
  (person key, epoch, older epochs), seals them with an OS keystore, signs rows (device key) and person
  objects (person key), and runs two `store.Verifier` folds: `Chain` (one person's devices, private log)
  and `BoardVerifier` (every member's devices plus membership, one per board log).
* `pair` runs the "add a device" flow: a one-use 16-symbol code, a pairing room derived from it with
  scrypt, mutual HMAC proofs, 5 PGP-word SAS, a typed key fingerprint, then A certifies B and hands
  over the person secret and every `person.*` row.
* `owner` is a static table of which device may write owner-only row kinds (`owner/owner.go:35-52`).

### Public interface used by other areas

| Consumer | Calls |
|---|---|
| store (Signer) | `Identity.SignRow(msg) -> sig` = ed25519(device key, `store.SigningBytes(ev)`) (`identity/identity.go:271-273`) |
| store (Verifier) | `Chain` / `BoardVerifier`: `Observe(ev)`, `VerifyRow(ev) -> err` (wrapping `store.ErrNoCert`, `ErrBadSig`, `ErrBoardActor`, `ErrPastCut`), `Cut(dev) -> (seq, ok)`, `CutClaims() -> [{deviceId, seq, hash}]`, `Reset()` (`identity/chain.go:111-117,138,386,431-444`; `identity/board.go:340,359,844,984-996`) |
| mesh | `Sign(msg)`, `DevicePub()`, `Current()`, `Self()`, `PersonSeed(epoch)`, `PrevEpoch()`, `OlderEpochs()`, `Rotate(at)`, `AdoptSecret(epoch, seed, pub)`, `ReplaceSecret(...)`, `Forget(below)`, `Chain.Learn(ev)`, `Chain.Devices()`, `Chain.Cert(dev)` (`identity/rotate.go:17-238`; mesh callers `mesh/node.go:413,919,921,1017`) |
| core | `Load`, `EnsureSelfCert`, `CertFor`, `RevokeOf`, `ProfileRow`, `BoardPin`, `BoardCreatedRow`/`BoardRenamedRow`/`BoardMemberAddedRow`/`BoardMemberRoleRow`/`BoardMemberRemovedRow`/`BoardMemberLeftRow` (`identity/membership.go:131-178`), `MemberCertFor`/`MemberRevokeOf`/`MemberProfileRow`/`MemberRotation` (`identity/board.go:89-137`), `NewBoardVerifierFor`, `BoardVerifier.Pin/Pinned/Membership/BoardEpoch/PersonOf/CardPerson/KeyPerson/Cert/Epoch/Writable/Profile/Devices/Ambiguous/Bad`, `OpenBoardSecrets`, `Keystore()`, `ValidBoardName/ValidBoardTag` |
| home | `LoadOrCreateSecret("turn.key") -> [32]byte` (`identity/identity.go:126-134`; `home/home.go:46,413`) |
| pair | `AdoptPerson(person, seed)` (`identity/rotate.go:197-224`), `Chain.Repin` (`identity/chain.go:123-128`), `Store.ReloadVerifier` |
| inbox/coord/api/ask | `Chain.IsOwner(key)`, `identity.UserKey(dev)` = `"user:"+dev`, `OwnerOnly(dev)` (`identity/chain.go:489-516`); `owner.DeviceOf`, `owner.MayWrite`, `owner.EntityOf` (`owner/owner.go:65-89`) |
| GUI (JSON to React) | `Person` (`identity/identity.go:63-69`), `DeviceState` (`identity/chain.go:469-473`), `Membership`/`BoardPerson`/`Conflict`/`RowAt` (`identity/membership.go:181-269`), `BoardDevice` (`identity/board.go:1077-1081`), pair `Status`/`Joiner` (`pair/pair.go:84-119`) |

JSON field names of GUI types: `Person{personId,name,epoch,rootDeviceId,personPub}`;
`DeviceState{cert,revoked,cut(omitempty)}`; `RowAt{hlc,device,seq}`;
`BoardPerson{personId,name(omitempty),role,state,pin,since}`;
`Conflict{at,kind,personId(omitempty),by(omitempty),reason}`;
`Membership{created,boardId,name,tag,creator,members,cuts(omitempty),gone(omitempty),conflicts(omitempty)}` (Pins is `json:"-"`);
`Status{mode,state,code?,serverAddr?,via?,expires(omitzero),role?,fingerprint?,joiner?,words?,peer?,keyOk?,misses?,error?}`
(`?` = omitempty); `Joiner{deviceId,name,os,fingerprint?}`.

---

## 2. Data formats

### 2.1 Files in the data dir

Data dir: `%LOCALAPPDATA%\irori` on Windows, `~/.irori` elsewhere, `IRORI_HOME` overrides (`paths/paths.go:12-25`).
A PC's data dir (e.g. `C:\Users\pat\AppData\Local\irori`) holds `device.json`, `device.key`, `person.json`, `turn.key`, `irori.db` (`boards/` once a board exists).

| File | Shape | Writer rules | Ref |
|---|---|---|---|
| `device.json` | `{"id","name","os","personId"?,"devicePub"?}` (`?` omitempty) | `json.MarshalIndent(v,"","  ")`, no trailing newline, write `device.json.tmp` mode 0600 then rename; `MkdirAll(dir,0700)` first | `device/device.go:21-32,61-72` |
| `device.key` | wrapped (below), plaintext = 32-byte ed25519 seed | created on first `Load`, never rewritten | `identity/identity.go:34,102,143-174` |
| `turn.key` | wrapped, plaintext = 32 random bytes (made by the same seed routine; used raw as TURN HMAC key) | home server only | `identity/identity.go:126-134`, `home/home.go:46` |
| `person.json` | `{"personId","name","epoch","rootDeviceId","personPub","secretKey":wrapped,"older":[{"epoch","personPub","secretKey":wrapped}]?,"prev":{...}?}` | MarshalIndent 2 spaces, `person.json.tmp` 0600 + rename. `older` oldest first, omitted when empty. `prev` is read only (legacy) | `identity/identity.go:38-60,232-254` |
| `boards/<boardId>.secrets` | wrapped, plaintext = compact JSON `{"epochs":{"<epoch int as string>":{"<makerDeviceId>":"<b64 32 bytes>"}},"settled":N}` | `json.Marshal` (compact), then sealed, then written like a wrapped file | `identity/boardsecret.go:80-86,264-274`; path `core/boards.go:377`, `BoardsDir="boards"` `core/boards.go:52` |
| `pair-rows.json` | compact JSON array of mesh rows `[{"d","s","h","a","k","p","sig"}]`, `p`/`sig` std base64 | `pair-rows.json.tmp` 0600 + rename; no secret inside; deleted once every row is stored | `pair/saved.go:23-39,44-72`; row type `mesh/frames.go:91-99` |

**Wrapped secret** (`identity/keystore.go:35-39,45-51,68-81`):
```
{
  "v": 1,
  "keystore": "dpapi" | "keychain" | "file",
  "blob": "<std base64 with padding of the sealed bytes>"
}
```
MarshalIndent with 2 spaces; field order `v, keystore, blob`. Reading: if `keystore` differs from this
device's keystore name -> `ErrWrongKeystore` (`identity/keystore.go:53-56`). `v` is written as 1 and never checked.
Live `person.json` on the PC: `epoch 1`, `keystore "dpapi"`, blob 376 base64 chars, no `older`.

**person.json load rules** (`identity/identity.go:176-228`):
* Unreadable JSON or empty `personId` -> error (no regeneration).
* Unseal `secretKey`; must be 32 bytes; ed25519 public of that seed hex-encoded must equal `personPub` exactly, else error.
* `older` used; if `older` empty and `prev` present, `older = [prev]`. Each older entry unsealed and checked the same way.
* Missing file -> new person: fresh ed25519 key, `personId = PersonIDOf(pub)`, `name = defaultName()`, `epoch = 1`, `rootDeviceId = this device id`.
* `defaultName()` = OS user name with everything up to the last `\` or `/` removed; `"you"` when empty (`identity/identity.go:257-268`).

**Identity.Load** (`identity/identity.go:98-121`): load/create `device.key`; if `device.json.devicePub` is non-empty and differs
from the key's pub hex -> fatal "data dir is mixed from two installs". Load/create person. If `devicePub` or `personId` in device.json differ, rewrite device.json.

**device.json create** (`device/device.go:35-58`): `id` = hex of 16 random bytes; `name` = `os.Hostname()` (error ignored -> ""); `os` = Go `runtime.GOOS` (`windows`, `darwin`, `linux`; Rust must write `darwin`, not `macos`). Unreadable file or empty `id` is fatal.

### 2.2 Row payloads (JSON inside `store` rows)

Row kinds: `store/kinds.go:22-65`. Payload bytes are stored exactly as written (Go struct field order, compact `json.Marshal`); the row signature covers `sha256(payload)` (§3.1), so stored bytes are never re-encoded.

| Kind | Payload struct (field order as written) | Ref |
|---|---|---|
| `person.device.added` | `{"cert":Cert,"personPub","personSig","personEpoch"?}` | `identity/rows.go:36-41` |
| `person.device.revoked` | `{"revoke":Revoke,"personPub","personSig","personEpoch"?}` | `identity/rows.go:55-60` |
| `person.key.rotated` | `{"personId","epoch","personPub","at","by"?,"prevSig"?}` | `identity/rows.go:67-74` |
| `person.profile` | `{"personId","name","rootDeviceId","personPub","personSig"}` | `identity/rows.go:92-98` |
| `board.created` | `{"boardId","name","tag","creator":MemberPin,"at","personSig"?}` | `identity/membership.go:54-61` |
| `board.renamed` | `{"name","by","at","personEpoch","personSig"?}` | `identity/membership.go:64-70` |
| `board.member.added` | `{"personId","personPub","rootDeviceId","personEpoch","name","role","by","byEpoch","at","personSig"?}` | `identity/membership.go:76-87` |
| `board.member.role` | `{"personId","role","by","at","personEpoch","personSig"?}` | `identity/membership.go:95-102` |
| `board.member.removed` | `{"personId","by","reason"?,"cut":{dev:seq}|null,"at","personEpoch","personSig"?}` | `identity/membership.go:108-116` |
| `board.member.left` | `{"personId","cut":{dev:seq}|null,"at","personEpoch","personSig"?}` | `identity/membership.go:121-127` |
| `member.device.added` | `{"cert":MemberCert,"personSig"}` | `identity/board.go:47-50` |
| `member.device.revoked` | `{"revoke":MemberRevoke,"personSig"}` | `identity/board.go:65-68` |
| `member.key.rotated` | `{"personId","personEpoch","personPub","by","at","prevSig"}` | `identity/board.go:73-80` |
| `member.profile` | `{"personId","name","personEpoch","personSig"}` | `identity/board.go:95-100` |
| `agent.card` (only `cardId`,`personId` read here) | — | `identity/board.go:400-410` |

Sub-objects:
* `Cert{personId,deviceId,devicePub(hex),label,role,at}` (`identity/rows.go:21-28`); roles `desktop`/`node` (`identity/rows.go:14-17`).
* `Revoke{personId,deviceId,cut(int64),cutHash(hex sha256 of the cut row's signed bytes and sig),at,reason?}` (`identity/rows.go:44-51`).
* `MemberPin{personId,personPub,rootDeviceId,personEpoch}` (`identity/board.go:27-32`).
* `MemberCert{personId,deviceId,devicePub,role,personEpoch,at}` — no label (`identity/board.go:36-43`).
* `MemberRevoke{personId,deviceId,boardCut,boardCutHash,personEpoch,at}` (`identity/board.go:55-62`).

All keys/sigs are lowercase hex strings (Go `hex.EncodeToString`); `at` is a Go `time.Time` (§3.3).
Stored shape (a real PC row confirmed the field order and that older builds wrote no `personEpoch`; the values here are the §7.1 fixture):
`{"cert":{"personId":"3097…","deviceId":"a1b2…","devicePub":"22fc…","label":"DESKTOP-FIXTURE","role":"desktop","at":"2026-10-28T14:25:23.9472209Z"},"personPub":"2152…","personSig":"f038…"}`.
A PC's log can also hold another device's first-run self-cert and profile for a different person: stored and inert, exactly the "another person's" case (`identity/chain.go:157-158`).

### 2.3 Pairing wire formats (`pair/pair.go`)

Room envelopes (go-room signaling, `mesh.Envelope{From,Type,Body}`): `pair.hello` body `{"deviceId","name","os"}` (`pair/pair.go:260-264,641`), `pair.busy` body `{}` (`pair/pair.go:622`).
Connection frames, one JSON object per message (`pair/pair.go:267-281`):
`{"type","proof"?(b64),"name"?,"os"?,"person"?:Person,"seed"?(b64 32 bytes),"rows"?:[Row],"servers"?:{id:[addr]},"reason"?}`;
types `pair.proof`, `pair.grant`, `pair.done`, `pair.fail`, `pair.cancel`, `pair.key` (`pair/pair.go:39-46`). A frame without `type` is "bad frame" (`pair/pair.go:695`).

---

## 3. Signatures: exact bytes

### 3.1 Row signature (device key)
`ed25519(deviceKey, SigningBytes(ev))`, where (`store/store.go:297-311`):
```
"irori/row/1" || lp(device_id) || u64be(seq) || u64be(hlc) || lp(actor) || lp(kind) || sha256(payload bytes)
lp(s) = u32be(len(s)) || s
```
`identity` only verifies it: `Chain.VerifyRow` (`identity/chain.go:386-411`), `BoardVerifier.VerifyRow` (`identity/board.go:844-876`).

### 3.2 Person signatures: Canonical
`Canonical(v)` (`identity/rows.go:111-129`) = `json.Marshal(v)` -> decode into generic `any` with `UseNumber` -> encode with
`SetEscapeHTML(false)` -> trim trailing `\n`. Result: compact JSON, **object keys sorted bytewise at every level**, numbers copied
literally, Go string escaping (§3.4). Signature = `hex(ed25519(personKey, Canonical(body)))`.
Verification **re-marshals from the decoded struct** (`identity/rows.go:132-149`): unknown fields are dropped, zero/omitempty rules reapply, times are re-formatted.

| Object | Body signed (sorted keys) | Sig field | Signer key | Ref |
|---|---|---|---|---|
| device cert | `Cert`: `at,deviceId,devicePub,label,personId,role` | `personSig` beside `cert` | person key of `personEpoch` | `identity/identity.go:311-316` |
| device revoke | `Revoke`: `at,cut,cutHash,deviceId,personId[,reason]` | `personSig` | same | `identity/identity.go:319-324` |
| person profile | `profileBody`: `name,personId,personPub,rootDeviceId` | `personSig` | current | `identity/identity.go:327-332`, `identity/rows.go:101-106` |
| rotation | `RotationBody`: `at,by,personEpoch,personId,personPub` | `prevSig` | the **previous** epoch's key | `identity/rotate.go:89-93`, `identity/rows.go:78-89` |
| board.created | whole payload minus `personSig`: `at,boardId,creator{personEpoch,personId,personPub,rootDeviceId},name,tag` | `personSig` | creator, `creator.personEpoch` | `identity/membership.go:131-136,315-320` |
| board.renamed | `at,by,name,personEpoch` | `personSig` | `by`, `personEpoch` | `identity/membership.go:140-145` |
| board.member.added | `at,by,byEpoch,name,personEpoch,personId,personPub,role,rootDeviceId` | `personSig` | `by`, **`byEpoch`** | `identity/membership.go:148-154,334` |
| board.member.role | `at,by,personEpoch,personId,role` | `personSig` | `by`, `personEpoch` | `identity/membership.go:157-162` |
| board.member.removed | `at,by,cut,personEpoch,personId[,reason]` | `personSig` | `by`, `personEpoch` | `identity/membership.go:165-170` |
| board.member.left | `at,cut,personEpoch,personId` | `personSig` | `personId`, `personEpoch` | `identity/membership.go:173-178` |
| member.device.added | `MemberCert`: `at,deviceId,devicePub,personEpoch,personId,role` | `personSig` | `cert.personEpoch` | `identity/board.go:118-122` |
| member.device.revoked | `MemberRevoke`: `at,boardCut,boardCutHash,deviceId,personEpoch,personId` | `personSig` | `revoke.personEpoch` | `identity/board.go:125-129` |
| member.profile | `memberProfileBody`: `name,personEpoch,personId` | `personSig` | `personEpoch` | `identity/board.go:102-106,132-137` |
| member.key.rotated | `RotationBody` from its fields | `prevSig` | previous epoch | `identity/board.go:82-84` |

Asymmetries to get right:
* `personEpoch` sits **outside** `Cert` and `Revoke` in the private rows, so a v0.4 device that re-marshals its own struct still verifies (comment at `identity/rows.go:30-35`; test `identity/board_test.go:491`).
* `person.key.rotated` names the epoch `epoch`; `RotationBody` names it `personEpoch` (`identity/rows.go:69,80`).
* `person.key.rotated` carries **no person signature over itself**; the private `Chain` trusts it because an ever-certified device wrote it (row signature). It does not check `by` or `prevSig` (`identity/rows.go:62-66`; `identity/chain.go:264-268`).
* board.* rows are signed over the full payload with `personSig` set to `""` and so **absent** (omitempty) (`identity/membership.go:319-355`).
* `cut` maps: Go `nil` encodes `null`, an empty map encodes `{}`; decoding `null` gives nil and `{}` gives empty, and verification re-marshals the decoded value, so both must round-trip distinctly.
* `reason` omitempty in `Revoke` and `BoardMemberRemoved`; `by`, `prevSig` omitempty in `KeyRotated`.
* `signEpoch` refuses to sign if the current epoch moved away from the requested epoch (`identity/identity.go:297-308`); `signPerson` (profile only) signs with whatever is current (`identity/identity.go:284-292`).

### 3.3 Time encoding
Go `time.Time` JSON = RFC 3339 with nanoseconds, **trailing fractional zeros trimmed, fraction omitted when zero**, offset `Z`
when the offset is 0, else `+hh:mm`. Writers always call `.UTC()` (`identity/identity.go:313,321`). Live rows show 7 digits on
Windows (`.9472209Z`) and 6 on macOS (`.868679Z`). A missing `at` decodes to the zero time and canonicalises as `"0001-01-01T00:00:00Z"`.
Verification re-formats the parsed time: `"…05.100Z"` canonicalises as `"…05.1Z"` (vector REPARSE, §7), so a row signed over
a non-Go spelling does not verify in Go. Go refuses to marshal years outside 0..9999 (Canonical errors, signature fails).

### 3.4 Go string escaping in Canonical (verified, §7 ESC)
* `"` -> `\"`, `\` -> `\\`, `\n \r \t` -> `\n \r \t`, U+0008 -> `\b`, U+000C -> `\f`, other U+0000..U+001F -> `\u00xx` (lowercase hex).
* U+2028 -> `\u2028`, U+2029 -> `\u2029` **always** (even with EscapeHTML off).
* `< > &` raw (EscapeHTML off; test `identity/identity_test.go:119-128`). U+007F raw. All other non-ASCII raw UTF-8.
* Invalid UTF-8 -> U+FFFD.

### 3.5 Ids
* `PersonIDOf(pub) = hex(sha256(pub32)[0:16])` (`identity/rows.go:153-156`). Checked against the live PC person: matches.
* Device id: hex of 16 random bytes (`device/device.go:48-52`). Board id: 16 bytes hex (`identity/boardsecret.go:41-44`).

---

## 4. Rules, constants and timings

### 4.1 Keys and epochs (`identity/rotate.go`)
* `Rotate(at)` (`:77-98`): new ed25519 key; `next = epoch+1`; row `KeyRotated{personId, epoch: next, personPub: new, at: at.UTC(), by: deviceId}`; `prevSig` = old key over `Canonical(RotationBody)`; then `adoptLocked`.
* `adoptLocked(epoch,key)` (`:228-238`): `older += (current epoch, current key)`, current = new; rewrite person.json; update memory only after the write succeeds.
* `AdoptSecret(epoch, seed, pub)` (`:107-118`): seed must be 32 bytes and derive exactly `pub` (hex compare), and `epoch > current` else `ErrStaleEpoch`.
* `ReplaceSecret` (`:126-146`): same checks, `epoch == current` required; no-op when pub already current; replaces current key, older untouched, losing key dropped.
* `Forget(below)` (`:152-169`): drops older epochs `< below`; writes only when something changed.
* `PersonSeed(epoch)` current or kept older (`:36-48`); `PrevEpoch` newest older or 0 (`:52-60`).
* `AdoptPerson(p, seed)` (`:197-224`): seed 32 bytes; `p.ID`, `p.RootDeviceID` non-empty, `p.Epoch >= 1`; seed must derive `p.Pub` (`ErrNotThisKey`); rewrite person.json with no older epochs; set device.json `personId`.

### 4.2 EnsureSelfCert (`identity/identity.go:339-371`)
Only when this device is the person's `rootDeviceId`. If `Chain` has no cert for self: append `person.device.added` (actor `"irori"`)
with label = device name or device id, role given, `at = now`; then it must certify (else error). Then if `!HasProfile`, append `person.profile`.
Order matters: the cert first, it carries its own verifying key.

### 4.3 The private cert fold `Chain` (`identity/chain.go`)
State: `personID`, `rootID`, `selfID`; observed rows keyed `(deviceId, seq)`, each folded once (`:138-148`).
Position order `rowPos.less`: `hlc` asc, then writer id bytewise, then seq (`:58-72`).

`Observe(ev)` (`:149-211`) per kind (rows for another `personId` are ignored, not counted bad):
* `person.device.added`: bad if JSON fails, person sig fails over `Cert`, `devicePub` not 32-byte hex, or empty `deviceId`.
* `person.device.revoked`: bad if sig fails, empty `deviceId`, or `cut < 0`.
* `person.key.rotated`: bad if `personPub` not 32-byte hex.
* `person.profile`: bad if sig over `profileBody` fails.
* Every accepted row triggers `refold()`.

`refold()` (`:245-305`):
1. Sort adds and revokes by position. `trusted(pub) = keys[pub] || PersonIDOf(pub) == personID` (first key).
   Loop: `signed = adds with trusted(pub)`; `ever = certifyRows(root, signed, no cuts)`; add to `keys` every rotation pub whose
   writer is in `ever` (rotations iterated in **arrival order**, not sorted; result is a set so order is irrelevant); repeat until no key added.
2. Cuts: for each revoke in sorted order whose writer is in `ever` and whose `pub` is trusted:
   `cut = revoke.cut`; `f = floorOf(root, ever, writer, target)`; if found and `f > cut` then `cut = f`; `cuts[target] = min`.
   If `cutHash != ""` and `cut(original) > 0`, add claim `{target, revoke.cut, cutHash}` (the original cut, not the raised one).
3. `valid = certifyRows(root, signed, cuts)`.
4. `profile = true` if any profile has writer in `ever`, trusted pub, and `rootDeviceId == rootID`.

`certifyRows(root, rows sorted, cuts?)` (`:325-356`), shared with boards:
`within(pos) = cut absent for pos.writer || pos.seq <= cut`. First (in sorted order) row with `dev == root && writer == root && within` seeds `valid[root]`; none -> empty.
Then repeat until no change: for each row in order, skip if `dev` already valid or `dev == writer`; count if `valid[writer]` and `within`. First counted row per device wins.

`floorOf(root, ever, writer, target)` (`:359-378`): walk from `writer` up its cert chain (`ever[cur].pos.writer`) at most `len(ever)` steps, stopping after the root; return the highest `seq` among those cert rows written by `target`.

`VerifyRow(ev)` (`:386-411`): writer must hold a valid cert, except the bootstrap: a `person.device.added` written by the pinned root, for itself, for this person, whose `personPub` is the **first key** and whose person sig verifies; then the row is checked with that cert's `devicePub`. Else `ErrNoCert`. Bad key/sig -> `ErrBadSig`.
`Learn(ev)` (`:419-428`): only `person.*` kinds; `VerifyRow` then `Observe`.
`IsOwner(key)` (`:497-509`): exactly `"user:"+dev` with non-empty dev, and dev is self or has a valid cert (revoked devices still count).
`Devices()` sorted by device id, `revoked = cut exists` (`:476-486`).

### 4.4 Membership fold (`identity/membership.go:369-596`, pure)
Constants: roles `owner`/`member`, states `active`/`removed`/`left` (`:42-49`); `MaxBoardName = 80` runes (`:603`).
`decodeMembership` (`:311-361`) extracts `by`, `who`, signing `epoch`, `body` (payload with `personSig` cleared), `name/tag/board/role/pin/cut`. For `board.created` `by = who = creator.personId`, epoch `creator.personEpoch`; for `member.added` epoch is `byEpoch`; for `left` `by = who = personId`. A row decodes only when `by != ""`.
`authored(d) = k.PersonOf(d.device) == d.by && k.Signed(d.by, d.epoch, d.at, d.sig, d.body)`.

1. Sort by position. Created = the **first** `board.created` whose `creator` pin equals the anchor for that person exactly, written by `pin.rootDeviceId`, `boardId` matches (unless the verifier's boardID is `""`), and authored. None -> `Membership{BoardID, Pins: anchors}` with `Created=false`.
2. Authority: `since = {creator: created.at}`; `ownerBefore(p, at) = since[p] < at`. Fixed point: an `added`/`role` row with `role == owner`, non-empty `who`, `ownerBefore(by, at)` and authored sets `since[who] = min(existing, at)`.
3. Pins: for each `member.added` in order with valid pin (`personId`,`rootDeviceId` non-empty, `personEpoch >= 1`, 32-byte hex pub) not already pinned, with `ownerBefore(by)` and authored: pin it (earliest counted add wins).
4. Honoured gone rows (only for pinned `who`): `removed` with `who != by`, `ownerBefore(by)`, authored; or `left` authored.
   Cuts per gone person, for each device in `k.Devices(who)` (sorted): `v = r.cut[dev]` (missing = 0); for a `left` written by that device, `v = max(v, row seq)`; take the minimum over the person's rows; then if `dev == created.device` and `cut < created.seq` raise to `created.seq`. `Gone` sorted.
5. State, rows in order; `past(at)` = above `m.Cuts[device]` or above `k.Cut(device)`; non-gate rows (`removed`/`left` are gates) that are past are skipped silently. Rows before `created` -> conflict "it comes before the board was created". Then per kind (conflict reasons verbatim):
   * `board.created`: the created one puts the creator active owner; others "the board was already created".
   * `board.renamed`: "its signature doesn't count" / "only an owner renames the board" / "the name is not a board name" / else rename.
   * `board.member.added`: "its signature doesn't count" / "only an owner adds members" / "the role is not owner or member" / "it pins another key than the one this board holds for them" (not pinned, or the fold's pin differs from the row's pin and `!k.HasKey(who, rowPin.personEpoch, rowPin.personPub)`, `identity/membership.go:539`) / "they are already a member" (entry active) / else put active with the **fold's pin** (not the row's), `since = at`.
   * `board.member.role`: signature / "only an owner changes roles" / role / "they are not on the board" / "they were removed or left before it; the removal stands" / "the last owner can't stop being one" (owner->member when active owners == 1) / else set role.
   * `removed`/`left`: not honoured -> "it isn't signed by the person leaving" (left) or "only an owner removes members"; a removal whose remover is no longer an owner still applies with conflict "its remover was no longer an owner, but a removal always stands"; new entry -> put with role member, state, pin; existing active -> set state. A re-add after removal is a `member.added` with a later position.
   `Members` keep first-seen order.
`ValidBoardName`: non-empty, `TrimSpace(name) == name`, <= 80 runes, valid UTF-8, no `unicode.IsControl` (Cc) rune (`:607-617`).
`ValidBoardTag`: 1..16 bytes, first `a-z`, rest `a-z0-9-` (`:623-634`).

### 4.5 BoardVerifier (`identity/board.go:139-1118`)
State kept across `Reset`: `boardID` and anchor `pins`. Observed rows folded once by `(device, seq)`.
`Observe` (`:359-419`): `member.device.added` bad unless `deviceId` set and pub valid; `member.device.revoked` bad unless `deviceId` set and `boardCut >= 0`; `member.key.rotated` bad unless pub valid and `by == writing device`; `member.profile` bad if JSON fails; `agent.card` bad unless `cardId` set (refolds cards only); membership kinds go to `brows`; others ignored.

`refold` (`:426-524`):
1. Sort certs/revokes/rotations/profiles by position.
2. `pins = anchors`; loop at most `len(brows)+2` times: `members = foldMembers(pins, nil)`; `ms = FoldMembership(boardID, anchors, brows, keys(members))`; stop when `ms.Pins == pins`, else `pins = ms.Pins`.
3. If `ms.Cuts` non-empty, `members = foldMembers(pins, ms.Cuts)`.
4. Members' names from counted `member.profile` (the latest counted one per member).
5. Device owner: for personIds sorted, each valid device collects owners. One owner -> that one. Several -> the one whose pin's `rootDeviceId == dev` (last such in sorted order), else none; mark `ambig`.
6. Revoke cuts count only for devices owned by that member; `revokedN = len(cuts)`; `revCuts` = copy. Then for each device of a gone person, `ms.Cuts` adds a cut or lowers the existing one (min) (`identity/board.go:510-521`).
7. `foldCards`: first `agent.card` per cardId (sorted) whose writer's person equals its `personId`.

`foldMember(pin, extra)` (`:632-718`): filter rows by `personId`. For each revoke (sorted): build chain skipping rotations by the revoke's target (`freeze=false`); honoured if writer in `ever`, `signedIn(keys, rev.personEpoch, pos, sig, rev)`, and target in `ever`. Then the chain skipping all revoked devices with `freeze=true`. Signed certs = `signedIn` by `cert.personEpoch`. Cuts = min over honoured revokes of `boardCut` raised by `floorOf(pin.rootDeviceId, ever, writer, target)`; claim when `boardCutHash != "" && boardCut > 0`. `within` = those cuts plus `extra` for devices in `ever` (min). `valid = certifyRows(pin.root, signed, within)`.

`memberChain(pin, certs, rots, skip, freeze)` (`:726-744`): `keys = [{pin.epoch, pin.pub, end 0}]`; at most `len(rots)+2` passes: signed certs -> `ever = certifyRows(root, signed, nil)` -> `next = epochsFrom(...)`; stop when `next == keys`.
`epochsFrom` (`:750-784`): at most `len(rots)` steps: candidates for `cur.epoch+1` with `by == writer`, `prevSig` verifying by `cur.pub` over the body, `by` in `ever`. Skipped (`skip(by)`) candidates record `frozen = min hlc`. Best = lowest `by` (bytewise), tie lowest position. None: if `freeze && frozen > 0` then `cur.end = frozen`; stop. Else `cur.end = best.hlc`, append the new epoch.
`signedIn(keys, epoch, pos, sig, body)` (`:801-812`): find the entry for `epoch`; if `end != 0 && pos.hlc >= end` -> false; else verify. Memoised by `(writer, seq, pub)` (`:817-832`).

`VerifyRow` (`:844-876`): cert = counted cert of writer, else `rootSelfCert` (`member.device.added` for itself, pinned person, writer is the pin's root, `cert.personEpoch == pin.personEpoch`, sig by pin key) (`:880-894`); none -> `ErrNoCert`. A writer with a revoke cut writing `board.member.removed`/`left` above that cut -> `ErrPastCut`. Then row signature, then `bindActor`.
`bindActor` (`:900-981`): membership kinds: actor person (`by`; `creator.personId` for created; `personId` for left) must equal the device's person. `agent.card`: `personId` must. Other `member.*`: `cert.personId`/`revoke.personId`/`personId` must. `board.server`: payload `deviceId` must equal writer. Else any payload `by` must equal the row's actor. Then actor `person:<id>` must be the device's person; `agent:<card>` (except on `agent.card`) must name a held card of that person, else `ErrNoCert` (wait).
`BoardEpoch() = 0` if not created, else `1 + len(Gone) + revokedN` (`:330-337`). `Writable(p)` = newest epoch has `end == 0` (`:1058-1063`). `Pin` refuses a different pin for an anchored or fold-pinned person (`:273-294`).

### 4.6 Board secrets (`identity/boardsecret.go`)
* `DeriveBoardTopic(boardId, epoch, secret)` (`:40-56`): boardId must be 16 bytes hex, `epoch >= 1`, secret 32 bytes.
  `out = HKDF-SHA256(ikm=secret, salt="irori/board/1", info=boardIdBytes(16) || u64be(epoch), L=48)`; key = `out[0:32]`; topic = `"irori/board/1/" + hex(out[32:48])`.
* Keep only the newest epoch and the newest one below it after every add (`:174-181`). `Mint` no-op if maker already has one; `Add` no-op if identical (`:139-164`). Maker id non-empty, epoch >= 1 (`:167-169`).
* `Secret(epoch)`: among makers passing `SetMakers` (nil = all), the lowest device id bytewise (`:204-216`).
* `Topics()`: newest epoch, plus the previous while `settled < newest`; epochs with no counting maker are skipped (`:221-245`). `Settle(e)` raises `settled` and deletes epochs `< e` (`:249-262`).

### 4.7 Pairing (`pair/code.go`, `pair/pair.go`)
| Constant | Value | Ref |
|---|---|---|
| App (scrypt salt, topic prefix) | `irori-pair-v1` | `pair/code.go:34` |
| Code alphabet / length | `23456789ABCDEFGHJKMNPQRSTUVWXYZ` (31) / 16, shown `XXXX-XXXX-XXXX-XXXX` | `pair/code.go:39-42,65-74` |
| Code sampling | 16 random bytes per batch, accept `v < 248`, symbol `v % 31` | `pair/code.go:48-63` |
| Room secret | `scrypt(N=2^15, r=8, p=1, len 48)` over canonical code (with dashes), salt `irori-pair-v1`; key `dk[0:32]`, topic `irori-pair-v1/` + hex(`dk[32:48]`) | `pair/code.go:99-112` |
| Info | `lp32(aPub) lp32(bPub) lp32(fpA) lp32(fpB) lp32(aID) lp32(bID)`, always A then B | `pair/code.go:117-124` |
| SAS | HKDF-SHA256(ikm=room key, salt `irori/pair/1`, info, 5 bytes); byte i -> `evenWords[b]` at even i, `oddWords[b]` at odd i | `pair/code.go:127-149` |
| Proof | HMAC-SHA256(room key, side byte `'A'`/`'B'` \|\| info), compared constant time | `pair/code.go:154-164` |
| Fingerprint | hex(sha256(pub)[0:8]) as `xxxx xxxx xxxx xxxx` | `pair/code.go:196-200` |
| Window / HelloEvery / MaxMisses | 10 min / 2 s / 2 | `pair/pair.go:22,27,30` |
| B waits after `pair.done` for close | 3 s | `pair/pair.go:505-510` |
| A waits for `done` after conn closes | 2 s | `pair/pair.go:880-886` |
| fail-frame send timeout | 2 s | `pair/pair.go:1032` |
| Guest TTL on hubs (A only) | = Window | `pair/pair.go:969-974` |

Word lists: copy `pair/wordlist.go` byte for byte (2 x 256 lowercase words; `evenWords[0]="aardvark"`, `[255]="zulu"`, `oddWords[0]="adroitness"`, `[255]="yucatan"`).
`NormalizeCode` (`pair/code.go:78-95`): TrimSpace, Unicode upper-case, drop `-`, space, tab; any other rune not in the alphabet rejects; exactly 16.
`ParseWords`: Unicode lower-case, split on space `,` `-` tab `\n` `\r` `.` (`pair/code.go:168-172`). `WordsMatch`: same count, each equal (`:175-185`).
`KeyMatch`: lower-case, keep `0-9a-f` only, want non-empty, constant-time equal (`:204-216`).

Flow (state names `pair/pair.go:49-63`; user texts `:66-70`):
1. A `Add(role)` (role other than `node` -> `desktop`): needs its own valid, unrevoked cert (`ErrNotCertified`); one session at a time (`ErrBusy`); joins the room as offerer; state `waiting` with code, expiry, fingerprint, serverAddr (`:304-338`).
2. B `Join/JoinVia(code, server)`: refused when `InMesh` (root is not self, or any other device certified) (`:289-299,347-376`); sends `pair.hello` every `HelloEvery` until connected (`:639-658`).
3. A: first hello pins the joiner; a hello from another device -> `pair.busy` to it and A fails "Someone else tried to use this code…"; revoked devices' hellos ignored (`:593-636`). `accept`: refuses self, an already-certified device (A side), and a second key for the pinned peer (`:563-591`). A replaced connection fails the session (`:668-675`).
4. Both send `pair.proof` (with name/os), check the other's (wrong -> fail), compute SAS. A -> `confirm` (code cleared, `Joiner` shown), B -> `words` (`:702-768`).
5. A `Confirm(words)`: miss increments, 2 misses -> fail; match -> `granted`, waits for B's `pair.key` (`:407-449`). B `ConfirmKey(typed)` checks `Fingerprint(conn.Pub())`; 2 misses fail; match sends `pair.key` and adopts a held grant (`:457-491`).
6. A `grant` once words and key are in: appends the cert row (label = hello name or joiner id), collects every `person.*` row (paged), sends `pair.grant{person, seed(current epoch), rows, servers}` (`:823-889`).
7. B `adopt` (`:913-958`): all rows `person.*`; a trial `Chain` must certify B under its own key and A under the handshake key; still not `InMesh`; save `pair-rows.json`; `AdoptPerson`; `Chain.Repin`; reload verifier; learn rows; hint servers. Then `pair.done`.
`learnAll` retries `Learn` passes until no progress (`:978-992`). `LearnSaved` at start learns saved rows whose `s > vector[d]`, removes the file when all are held or it is unreadable (`pair/saved.go:44-72`).

### 4.8 Owner table (`owner/owner.go`)
| Entity | Owner-only kinds | Request kinds |
|---|---|---|
| delivery | `delivery.state`, `wake.issued`, `turn.engaged`, `turn.ended` | `delivery.decided` |
| lease | `lease.acquired`, `lease.released`, `lease.refused` | `lease.stolen` |
| ask | `ask.resolved` | `ask.answered` |
| revive | — | `revive.requested` |
| server | `node.addr` | — |
| take | `take.state` | `take.requested` |
`DeviceOf(key)`: `"user:<dev>"` -> dev; else prefix before the first `/` if non-empty and containing no `:`; else `""` (`owner/owner.go:73-81`). `MayWrite(ownerKey, writer) = DeviceOf == "" || == writer` (`:86-89`).

---

## 5. Platform-specific code

| OS | Keystore name | Mechanism | Ref |
|---|---|---|---|
| Windows | `dpapi` | `CryptProtectData(plain, description L"Irori key", entropy "irori/key/1", flags CRYPTPROTECT_UI_FORBIDDEN (0x1))`, current-user scope; `CryptUnprotectData(blob, NULL, same entropy, …, UI_FORBIDDEN)`; output freed with `LocalFree` | `identity/keystore_windows.go:13-49` |
| macOS | `keychain` | AES-256-GCM, 12-byte random nonce, blob = `nonce \|\| ciphertext+tag`, AAD `"irori/key/1"`; KEK = 32 bytes in the login Keychain | `identity/keystore.go:85-128`, `identity/keystore_darwin.go:17-34` |
| other | `file` | blob = plaintext | `identity/keystore.go:28-32`, `identity/keystore_other.go:6` |

macOS KEK (`identity/keychain.go`):
* Accessed only through `/usr/bin/security` (no cgo): find `find-generic-password -s Irori -a <acct> -w` (stdout = 64 hex chars, trimmed); add `add-generic-password -s Irori -a <acct[0]> -l "Irori device key" -w <hex>` (`:91-134`). Exit 44 = not found, 45 = duplicate (re-find) (`:24-25`).
* Account names (`:60-75`): `"kek-" + hex(sha256(path)[0:8])`, first for `filepath.Clean(filepath.Abs(dataDir))`, second (legacy) for `dataDir` as given when different. Example for the fixture Mac dir `/Users/pat/.irori`: `kek-8a2f5e0399ac9edc` (Go's name function over `path.Clean`, as on macOS).
* Not found under any name: if `person.json`'s `secretKey.keystore` or any `*.key` wrapped file says `keychain`, fail with `ErrKEKMissing` naming service/account; else create a new key (`:91-113,146-166`).
* The key is cached after the first success; failures are retried next call and reported as "keystore's key is unavailable", not as a copied data dir (`identity/keychain.go:77-89`, `identity/keystore.go:57-63`).

Other platform bits: `device.json.os` is Go GOOS; `defaultName` strips a Windows `DOMAIN\`; file mode 0600 / dir 0700 are no-ops on Windows.

---

## 6. Rust crates (crates.io, checked 2026-10-29)

| Need | Crate | Version | Note |
|---|---|---|---|
| ed25519 | `ed25519-dalek` | 3.0.0 | new major; use the non-strict `verify` to match Go (`crypto/ed25519.Verify`: canonical S required, no small-order rejection). Not `verify_strict`. Seed = `SigningKey::from_bytes(&[u8;32])` |
| SHA-256 | `sha2` | 0.11.1 | |
| HKDF | `hkdf` | 0.13.0 | `Hkdf::<Sha256>::new(Some(salt), ikm).expand(info, &mut out)` = Go `hkdf.Key` |
| HMAC | `hmac` | 0.13.0 | `verify_slice` is constant time |
| scrypt | `scrypt` | 0.12.0 | log_n 15, r 8, p 1, out 48; check the 0.12 `Params` signature |
| AES-GCM | `aes-gcm` | 0.11.1 | `Aes256Gcm`, 96-bit nonce, `Payload{msg, aad}` |
| JSON | `serde` 1.0.229, `serde_json` 1.0.154 | | `Value` maps default to `BTreeMap` (sorted) unless `preserve_order` is on; keep it off for the canonical encoder or write the encoder by hand |
| hex / base64 | `hex` 0.4.3, `base64` 0.23.1 | | base64 `STANDARD` with padding |
| time | `time` | 0.3.55 | or a hand-written formatter; the acceptance test in §7 decides |
| randomness | `rand` 0.10.3 / `getrandom` 0.4.3 | | |
| constant time | `subtle` 2.6.1 | | |
| secrets in memory | `zeroize` 1.9.1 | | optional |
| DPAPI | `windows` 0.62.2 (features `Win32_Security_Cryptography`, `Win32_Foundation`) | | or `windows-sys` 0.61.2 |
| Keychain | shell out to `/usr/bin/security` with `std::process::Command` | | see trap 7.4; `security-framework` 3.7.0 only if the ACL question is settled |
| errors | `thiserror` 2.0.21 | | |
| networking | `iroh` | 1.4.1 is current (task says 1.0) | the networking area decides |

---

## 7. Tests to port, golden vectors, traps

### 7.1 Golden vectors (Go-produced)
```
CANON  {"at":"2026-10-28T14:25:23.9472209Z","deviceId":"a1b2c3d4e5f60718293a4b5c6d7e8f90","devicePub":"22fc297792f0b6ffc0bfcfdb7edb0c0aa14e025a365ec0e342e86e3829cb74b6","label":"DESKTOP-FIXTURE","personId":"3097e2dee2cb4a34b53840cdb705aed7","role":"desktop"}
       personPub 2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12   (seed 32x0x42; personId = PersonIDOf(personPub))
       devicePub from seed 32x0x43
       personSig f0383b98004df4c4794e59cefc6bcb1d5ab3dbd382591aa536aa9df1d5a84a372d53799fd1363ae8ee62e871278ade9cad0391baebd8c48ab48fea07f8d68708
       VERIFY true   (a fixture root self-cert signed by Go; decode a payload shaped as in §2.2, re-canonicalise, compare, verify)
TIMES  2026-01-02T03:04:05 UTC -> "2026-01-02T03:04:05Z"; +120ms -> "2026-01-02T03:04:05.12Z"; offset +1h -> "2026-01-02T03:04:05+01:00"
REPARSE "2026-01-02T03:04:05.100Z" -> "2026-01-02T03:04:05.1Z"
ESC    map{big:1<<62, m:nil map, n:Number("1"), nil:nil, s:...} -> {"big":4611686018427387904,"m":null,"n":1,"nil":null,"s":"a<b>&\u2028\u2029\u0001\b\f\n\t\"\\é\u2028\u2029"}
       (input s = "a<b>&" U+2028 U+2029 U+0001 U+0008 U+000C "\n\t" U+007F "\"\\é" U+2028 U+2029; the U+007F is emitted raw between \t and \")
ROOM   code "ABCD-EFGH-JKMN-PQRS": key 2af19e2f964ffa81e4b2b86b3c41d76a332049519de49a5c6cf9de80bf80aeb8, topic irori-pair-v1/d8a3057400706e013dee34210c2002ca
INFO   Info(aPub=32x0x01, bPub=32x0x02, fpA "aa:bb", fpB "cc:dd", "devA", "devB") with that room key:
       SAS bytes 90428d9556 -> [orca detergent offload narrative egghead]
       Proof('A') 370ea8b304c15b6de9e9ad5461e34dbd5b779e5f24203d24ecf5bf4934e06343
FP     Fingerprint(32x0x01) = "72cd 6e84 22c4 07fb"
BOARD  DeriveBoardTopic("0123456789abcdef0123456789abcdef", 1, 32x0x07): key f6bf38c7a9bacd14cd0a27ae71e6706dc7d654b127d2a39f12ddb74bdaaba311, topic irori/board/1/aaa89e14cf3a94a8b97e0b4c3fb50621
MESH   (mesh area, same HKDF shape) MeshSecret(32x0x09, "0f1e2d3c4b5a69788796a5b4c3d2e1f0", 1): key a0b026925f9167650beb3769636212286e4c32635329b9a8174d40dc92a15e6e, topic irori/mesh/1/567df962e162255932cc3daada564329 (`mesh/mesh.go:42-56`)
SEED   seed 32x0x05 -> pub 6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1, personId 7599776c3085e3f9da0d13071eb0b4ab
```
The ROOM, INFO (SAS, Proof) and FP vectors belong to `pair`, which is M7 scope; they go into `testdata/golden` with that crate, not with the M2 identity goldens.

Fixtures from Go tests: `Canonical(Cert{p,d,k,"a<b",desktop,2026-10-27 UTC})` = `{"at":"2026-10-27T00:00:00Z","deviceId":"d","devicePub":"k","label":"a<b","personId":"p","role":"desktop"}` (`identity/identity_test.go:119-128`); epoch-less cert at Unix 1 = `{"at":"1970-01-01T00:00:01Z",…}` (`identity/board_test.go:491-496`).
Also add a DPAPI read test on the PC and a Keychain read test on the Mac that unseal the live `device.key`/`person.json` and check pub equality, without printing anything.

### 7.2 Existing Go tests and what each proves
| Test | Proves |
|---|---|
| `identity_test.go` `TestKeystoresRoundTrip` | file/AES/default keystores round-trip; sealed blob doesn't contain the plaintext (non-file); wrong keystore name -> `ErrWrongKeystore`; wrong KEK fails GCM |
| `TestLoadCreatesOnceAndRecordsKeysInDeviceJSON` | first Load makes epoch 1, root = self, personId = PersonIDOf(pub); device.json gets personId/devicePub; second Load same keys; files contain `"keystore": "<name>"`; other keystore fails; mismatched devicePub fails |
| `TestCanonicalSortsKeys` | sorted keys, no HTML escaping, time format |
| `TestRootSelfCertIsPinned` | root self-certifies once, survives restart, fold rebuilt at Open, IsOwner |
| `TestCertChainAndSync` | non-root can't self-certify; uncertified rows `ErrNoCert`; certified rows accepted both ways; owner key accepted |
| `TestSelfCertOtherThanTheRootIsRefused` | non-root self-cert inert, also via a device it certified |
| `TestRevokeCutsAndMovesRows` | revoke sets cut; rows above cut moved to `refused_events`; vector capped |
| `TestMutualRevokeInEitherOrder` | two devices revoking each other: both revoked, any order, after restart too |
| `TestMinCutWins` | lowest honoured cut is effective |
| `TestOwnRowsComeBackAndFutureRowsWait` | restored data dir without DB must get own rows back before appending |
| `TestRevokeBelowItsWritersCertIsRaised` | floor raising; certs above cut stop counting |
| `TestCutHashMismatchFlagsAFork` / `TestOnlyASignedCopyIsAFork` | cut claims; fork only when the other copy is validly signed (`ErrBadSig`, `ErrUnsigned` otherwise) |
| `chain_keys_test.go` (5 tests) | stranger-key certs/revokes/root self-cert inert; rotated key counts in either arrival order; rotation from uncertified device ignored; profile checks; VerifyRow bootstrap needs first key and matching sig |
| `rotate_test.go` (4 tests) | rotate keeps older epoch across reload; AdoptSecret checks pub and epoch; every older epoch kept until Forget; ReplaceSecret same-epoch swap |
| `keychain_test.go` (6 tests, fake `security`) | KEK made once then loaded; missing KEK with sealed files is not remade; moved dir errors; legacy name found; add race uses winner; failures retried and reported as unavailable |
| `board_test.go` (9 tests) | member chain to pin; ended epoch refused after rotation HLC; rotation needs prevSig and By device; lower By wins; revoked device's rotation discarded; mutual revoke freezes person (`Writable` false); revoke cuts in store, only own devices; `bindActor` refusals (7); private rows gain `personEpoch`/`prevSig` while v0.4 structs still verify |
| `boardsecret_test.go` (3 tests) | topic derivation deterministic and sensitive to epoch/board/secret; bad id or short secret rejected; rotation keeps prev topic until Settle; Mint idempotent; lower maker wins; non-counting makers never chosen |
| `membership_test.go` `TestFoldMembership`, `TestFoldMembershipNeedsItsBoard`, `TestBoardNamesAndTags` | every membership rule and conflict, order independence; nothing counts without anchored created; name/tag validity table |
| `membership_board_test.go` (3 tests) | pins from adds allow invitee self-certs; forged add refused; mutual removal order-independent and stable after reopen; revoked device's leave past its cut is `ErrPastCut` |
| `pair_test.go` `TestCodeCodec`, `TestWordlists`, `TestSASBindsKeysFingerprintsAndCode` | code format/normalise; room secret spelling-independent; 512 distinct lowercase words; SAS changes with key, fingerprint, code, side order; WordsMatch; proof side binding |
| `pair_test.go` flow tests (9) | full pair copies person and syncs; survives restart before first sync (pair-rows.json); second joiner busy; two wrong word typings close; bad proof closes; window expiry; revoked device can't re-pair; code holder posing as A can't hand B a person; wrong key twice leaves no cert |
| `device_test.go`, `owner_test.go` | device.json stable; DeviceOf/MayWrite table; every owner/request kind is replicated |

### 7.3 Traps
1. **Canonical JSON is Go's, not serde's.** serde_json does not escape U+2028/U+2029; Go always does. Encode via sorted maps, then rewrite the UTF-8 sequences `E2 80 A8`/`E2 80 A9` to `\u2028`/`\u2029` (they only occur inside strings), or hand-write the encoder. `<>&` stay raw.
2. **Time format.** Must trim trailing fractional zeros and write `Z` for offset 0. chrono `to_rfc3339()` pads to 3/6/9 digits and breaks the live root cert (7 digits). Store `at` as (instant, offset) and format like Go; the CANON/VERIFY vector is the acceptance test.
3. **Canonical bytes are not the stored bytes.** Stored payloads come from plain `json.Marshal`, which HTML-escapes `<>&` as `<` etc. and keeps struct field order; Canonical sorts keys and leaves `<>&` raw. The row signature covers the stored bytes verbatim, so never re-encode stored payloads.
   **Re-marshal from the struct.** Verify over Canonical(decoded struct), never over the raw payload bytes; drop unknown fields; keep `null` vs `{}` for `cut`; zero `at` when missing; omitempty fields absent when empty.
4. **macOS Keychain account.** Go `filepath.Abs`+`Clean` is lexical: no symlink resolution (`fs::canonicalize` resolves symlinks), and `..` removed (`std::path::absolute` keeps it). A different spelling changes the hash and the Mac fails with `ErrKEKMissing`. Keep using `/usr/bin/security`: the item was created by that tool, so its ACL most likely trusts it, and a new binary reading it through `security-framework` may trigger a Keychain prompt (not tested; verify on the Mac).
5. **DPAPI entropy** must be exactly `irori/key/1`; the description string is not checked on unseal.
6. **GOOS strings** in `device.json.os` (`darwin`, `windows`, `linux`).
7. **Go `json.Unmarshal` leniency**: field names match case-insensitively, duplicate keys take the last value, invalid UTF-8 becomes U+FFFD. serde is case-sensitive and rejects invalid UTF-8. Only crafted rows differ, but such a row can be "bad" in one build and folded in the other.
8. **Loop bounds** are part of the semantics: refold `len(brows)+2`, `memberChain` `len(rots)+2`, `epochsFrom` `len(rots)`, `floorOf` `len(ever)`. Copy them; do not iterate "until convergence".
9. **Go map-order nondeterminism**: `membershipKeys.PersonOf` returns `""` early when it meets two non-root owners before the root owner, and `membershipKeys.Cut` returns the first member's cut (`identity/board.go:571-627`); with three or more members certifying one device the Go result depends on map order. Rust should iterate sorted personIds and accept that Go may differ only for devices certified by several members.
10. **Claim uses the original cut** (`identity/chain.go:287-289`, `identity/board.go:685-687`), not the floor-raised one.
11. **First key rule**: a person sig counts only by the key whose sha256 prefix is the personId, or a rotated key from an ever-certified writer; VerifyRow bootstrap accepts only the first key.
12. **`signEpoch` epoch guard** and the person.json write-before-swap ordering must stay, or a crash mid-rotation loses a key.
13. **`older` vs `prev`**: read `prev` only when `older` is empty; never write `prev`.
14. **Pairing SAS binds DTLS fingerprints** (`pair/code.go:117-124`, `mesh/mesh.go:137`): go-room gives `sha256(DER cert)` as lowercase hex pairs joined by `:` (`go-room/peer.go:344-364,523-536`). iroh has no DTLS, so a Rust device can't pair with a Go device in any case, and the Rust build needs a replacement channel binding.
15. **`turn.key`** is only go-room's TURN HMAC key (`go-room/turn.go:278,316`). Keep reading it (it is sealed like `device.key`) even if the iroh build has no TURN server.

### 7.4 Open questions
* What replaces `fpA`/`fpB` in `Info` under iroh (TLS exporter or the two endpoint ids)? Networking area decides; the SAS words change either way.
* The Mac's actual data dir and Keychain account need confirming on the Mac (expected `kek-` + the hash of its clean data dir path, as in §5), and whether reading the item from a new binary prompts.
* Whether `time` 0.3's Rfc3339 output trims fractions exactly like Go; dalek `verify` vs Go `Verify` on non-canonical point encodings; the scrypt 0.12 `Params` API. The vectors in 7.1 decide each.
* No Go test fixes the GUI JSON of `Membership`/`Status`; check the React code for which fields it reads.
