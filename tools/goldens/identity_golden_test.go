// Golden exporter for crates/identity (identity.md §3, §4, §7.1;
// architecture.md §6.2).
//
// Not part of the Go repo: tools/goldens/export.sh injects it into
// I:internal/identity with `go test -overlay`, as an internal test file, so
// it can reach Canonical, profileBody and the Identity fields directly. It
// runs only when IRORI_GOLDEN_OUT is set, and writes
// $IRORI_GOLDEN_OUT/identity/*.json.
//
// Every key comes from a fixed seed and every time is fixed, so a re-run
// writes the same bytes. Payloads are written as the exact stored bytes
// (strings); signatures as hex.
package identity

import (
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/device"
	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/owner"
	"github.com/StephenSHorton/irori/internal/store"
)

func goldOut(t *testing.T) string {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	return dir
}

func goldWrite(t *testing.T, name string, v any) {
	path := filepath.Join(goldOut(t), "identity", name)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

// goldKey is a fixed ed25519 key named by label.
func goldKey(label string) ed25519.PrivateKey {
	sum := sha256.Sum256([]byte("irori-golden/" + label))
	return ed25519.NewKeyFromSeed(sum[:])
}

func goldPub(k ed25519.PrivateKey) string { return hex.EncodeToString(k.Public().(ed25519.PublicKey)) }

func goldSign(t *testing.T, k ed25519.PrivateKey, v any) string {
	msg, err := Canonical(v)
	if err != nil {
		t.Fatal(err)
	}
	return hex.EncodeToString(ed25519.Sign(k, msg))
}

func goldMarshal(t *testing.T, v any) []byte {
	b, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// goldRow is a row as the vectors carry it; the payload is hex, since
// some hold invalid UTF-8 on purpose.
type goldRow struct {
	Device  string `json:"device"`
	Seq     int64  `json:"seq"`
	HLC     uint64 `json:"hlc"`
	Actor   string `json:"actor"`
	Kind    string `json:"kind"`
	Payload string `json:"payloadHex"`
	Sig     string `json:"sig"`
}

func goldRowOf(ev store.Event) goldRow {
	return goldRow{ev.DeviceID, ev.Seq, uint64(ev.HLC), ev.Actor, ev.Kind, hex.EncodeToString(ev.Payload), hex.EncodeToString(ev.Sig)}
}

func hlcOf(u uint64) hlc.Timestamp { return hlc.Timestamp(u) }

// goldLog writes rows with per-device seqs and an increasing HLC, each
// signed by its device's fixed key.
type goldLog struct {
	seq  map[string]int64
	hlc  uint64
	rows []store.Event
}

func newGoldLog() *goldLog { return &goldLog{seq: map[string]int64{}, hlc: 1 << 16} }

func (l *goldLog) add(writer, actor, kind string, payload []byte) store.Event {
	l.seq[writer]++
	l.hlc += 1 << 16
	ev := store.Event{DeviceID: writer, Seq: l.seq[writer], HLC: hlcOf(l.hlc), Actor: actor, Kind: kind, Payload: payload}
	ev.Sig = ed25519.Sign(goldKey("dev/"+writer), store.SigningBytes(ev))
	l.rows = append(l.rows, ev)
	return ev
}

func errText(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}

// TestGoldenIdentitySigned pins every signed shape: the stored payload
// bytes, the canonical bytes the person signature covers, and the
// signature, from the real builders.
func TestGoldenIdentitySigned(t *testing.T) {
	pk := goldKey("person/s/1")
	dk := goldKey("dev/pc")
	pid := PersonIDOf(pk.Public().(ed25519.PublicKey))
	id := &Identity{
		Device:    device.Identity{ID: "pc-0001", Name: "DESKTOP <&>", OS: "windows", PersonID: pid, DevicePub: goldPub(dk)},
		Person:    Person{ID: pid, Name: "Stephen <&> é ", Epoch: 1, RootDeviceID: "pc-0001", Pub: goldPub(pk)},
		deviceKey: dk, personKey: pk, dataDir: t.TempDir(), ks: FileKeystore{},
	}
	at := time.Date(2026, 10, 28, 14, 25, 23, 947220900, time.UTC)
	atLocal := time.Date(2026, 1, 2, 3, 4, 5, 120000000, time.FixedZone("x", 3600))
	type entry struct {
		Name      string `json:"name"`
		Payload   string `json:"payload"`
		Canonical string `json:"canonical"`
		Signer    string `json:"signer"`
		Sig       string `json:"sig"`
	}
	var out []entry
	push := func(name string, payload any, body any, sig string) {
		c, err := Canonical(body)
		if err != nil {
			t.Fatal(err)
		}
		out = append(out, entry{name, string(goldMarshal(t, payload)), string(c), goldPub(pk), sig})
	}
	must := func(err error) {
		if err != nil {
			t.Fatal(err)
		}
	}
	add, err := id.CertFor("mac-0002", goldPub(goldKey("dev/mac")), "Mac <b>", RoleDesktop, at)
	must(err)
	push("cert", add, add.Cert, add.PersonSig)
	add2, err := id.CertFor("node-3", goldPub(goldKey("dev/node")), "", RoleNode, atLocal)
	must(err)
	push("cert-local-time", add2, add2.Cert, add2.PersonSig)
	rev, err := id.RevokeOf("mac-0002", 42, strings.Repeat("ab", 32), "lost <it>", at)
	must(err)
	push("revoke", rev, rev.Revoke, rev.PersonSig)
	rev2, err := id.RevokeOf("mac-0002", 0, "", "", at)
	must(err)
	push("revoke-no-reason", rev2, rev2.Revoke, rev2.PersonSig)
	prof, err := id.ProfileRow()
	must(err)
	push("profile", prof, profileBody{prof.PersonID, prof.Name, prof.RootDeviceID, prof.PersonPub}, prof.PersonSig)
	// A rotation as Rotate builds it, with a fixed new key.
	nk := goldKey("person/s/2")
	rot := KeyRotated{PersonID: pid, Epoch: 2, PersonPub: goldPub(nk), At: at, By: id.Device.ID}
	rot.PrevSig = goldSign(t, pk, rot.Body())
	push("rotation", rot, rot.Body(), rot.PrevSig)
	mrot, ok := MemberRotation(rot)
	if !ok {
		t.Fatal("member rotation")
	}
	push("member-rotation", mrot, mrot.body(), mrot.PrevSig)
	old := KeyRotated{PersonID: pid, Epoch: 2, PersonPub: goldPub(nk), At: at}
	push("rotation-v04", old, old.Body(), "")
	pin := id.BoardPin()
	bc, err := id.BoardCreatedRow("0123456789abcdef0123456789abcdef", "MLG & co", "mlg", pin, at)
	must(err)
	bcBody := bc
	bcBody.PersonSig = ""
	push("board.created", bc, bcBody, bc.PersonSig)
	br, err := id.BoardRenamedRow(1, "ours <x>", at)
	must(err)
	brBody := br
	brBody.PersonSig = ""
	push("board.renamed", br, brBody, br.PersonSig)
	other := MemberPin{PersonID: "b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1b1", PersonPub: goldPub(goldKey("person/b/1")), RootDeviceID: "bpc", PersonEpoch: 3}
	ba, err := id.BoardMemberAddedRow(1, other, "Blake", BoardOwner, at)
	must(err)
	baBody := ba
	baBody.PersonSig = ""
	push("board.member.added", ba, baBody, ba.PersonSig)
	bro, err := id.BoardMemberRoleRow(1, other.PersonID, BoardMember, at)
	must(err)
	broBody := bro
	broBody.PersonSig = ""
	push("board.member.role", bro, broBody, bro.PersonSig)
	for _, c := range []struct {
		name   string
		cut    map[string]int64
		reason string
	}{{"board.member.removed", map[string]int64{"bpc": 7, "blap": 0, "a<b": -1}, "spam"}, {"board.member.removed-nil", nil, ""}, {"board.member.removed-empty", map[string]int64{}, ""}} {
		bm, err := id.BoardMemberRemovedRow(1, other.PersonID, c.reason, c.cut, at)
		must(err)
		body := bm
		body.PersonSig = ""
		push(c.name, bm, body, bm.PersonSig)
	}
	for _, c := range []struct {
		name string
		cut  map[string]int64
	}{{"board.member.left", map[string]int64{"pc-0001": 9}}, {"board.member.left-nil", nil}} {
		bl, err := id.BoardMemberLeftRow(1, c.cut, at)
		must(err)
		body := bl
		body.PersonSig = ""
		push(c.name, bl, body, bl.PersonSig)
	}
	mc, err := id.MemberCertFor(1, "mac-0002", goldPub(goldKey("dev/mac")), RoleDesktop, at)
	must(err)
	push("member.device.added", mc, mc.Cert, mc.PersonSig)
	mr, err := id.MemberRevokeOf(1, "mac-0002", 5, strings.Repeat("cd", 32), at)
	must(err)
	push("member.device.revoked", mr, mr.Revoke, mr.PersonSig)
	mp, err := id.MemberProfileRow(1)
	must(err)
	push("member.profile", mp, memberProfileBody{mp.PersonID, mp.Name, mp.PersonEpoch}, mp.PersonSig)
	if _, err := id.MemberCertFor(2, "x", goldPub(goldKey("dev/x")), RoleDesktop, at); err == nil {
		t.Fatal("signed for an epoch the key isn't on")
	}

	// identity.md §7.1 vectors.
	seed5 := bytes.Repeat([]byte{5}, 32)
	k5 := ed25519.NewKeyFromSeed(seed5)
	topic, err := DeriveBoardTopic("0123456789abcdef0123456789abcdef", 1, bytes.Repeat([]byte{7}, 32))
	must(err)
	type topicVec struct {
		Board  string `json:"board"`
		Epoch  int    `json:"epoch"`
		Secret string `json:"secret"`
		Key    string `json:"key"`
		Topic  string `json:"topic"`
	}
	var topics []topicVec
	for _, c := range []struct {
		board  string
		epoch  int
		secret byte
	}{{"0123456789abcdef0123456789abcdef", 1, 7}, {"0123456789abcdef0123456789abcdef", 2, 7}, {"fedcba9876543210fedcba9876543210", 1, 7}, {"0123456789ABCDEF0123456789ABCDEF", 1, 8}, {"00000000000000000000000000000000", 1 << 40, 0}} {
		tp, err := DeriveBoardTopic(c.board, c.epoch, bytes.Repeat([]byte{c.secret}, 32))
		must(err)
		topics = append(topics, topicVec{c.board, c.epoch, hex.EncodeToString(bytes.Repeat([]byte{c.secret}, 32)), hex.EncodeToString(tp.Key[:]), tp.Topic})
	}
	type badTopic struct {
		Board  string `json:"board"`
		Epoch  int    `json:"epoch"`
		Secret string `json:"secret"`
	}
	var bad []badTopic
	for _, c := range []badTopic{{"nope", 1, "07"}, {"0123456789abcdef0123456789abcd", 1, strings.Repeat("07", 32)}, {"0123456789abcdef0123456789abcdef", 0, strings.Repeat("07", 32)}, {"0123456789abcdef0123456789abcdef", 1, strings.Repeat("07", 31)}} {
		s, _ := hex.DecodeString(c.Secret)
		if _, err := DeriveBoardTopic(c.Board, c.Epoch, s); err == nil {
			t.Fatalf("bad topic %+v derived", c)
		}
		bad = append(bad, c)
	}
	goldWrite(t, "signed.json", map[string]any{
		"personSeedLabel": "person/s/1",
		"deviceSeedLabel": "dev/pc",
		"person":          id.Person,
		"device":          id.Device,
		"at":              at,
		"atLocal":         atLocal,
		"entries":         out,
		"seed5":           map[string]string{"pub": goldPub(k5), "personId": PersonIDOf(k5.Public().(ed25519.PublicKey))},
		"boardTopic":      map[string]string{"key": hex.EncodeToString(topic.Key[:]), "topic": topic.Topic},
		"topics":          topics,
		"badTopics":       bad,
	})
}

// TestGoldenIdentityChain folds a private log with every rule and the
// decoding cases where Go and serde differ, and records Go's answers.
func TestGoldenIdentityChain(t *testing.T) {
	pk1 := goldKey("person/s/1")
	pk2 := goldKey("person/s/2")
	pid := PersonIDOf(pk1.Public().(ed25519.PublicKey))
	l := newGoldLog()
	at := time.Unix(1761600000, 947220900).UTC()
	cert := func(dev, label string) Cert {
		return Cert{PersonID: pid, DeviceID: dev, DevicePub: goldPub(goldKey("dev/" + dev)), Label: label, Role: RoleDesktop, At: at}
	}
	added := func(writer string, c Cert, k ed25519.PrivateKey, epoch int) store.Event {
		return l.add(writer, "irori", store.KindPersonDeviceAdded, goldMarshal(t, DeviceAdded{Cert: c, PersonPub: goldPub(k), PersonSig: goldSign(t, k, c), PersonEpoch: epoch}))
	}
	revoked := func(writer, dev string, cut int64, hash string, k ed25519.PrivateKey) store.Event {
		r := Revoke{PersonID: pid, DeviceID: dev, Cut: cut, CutHash: hash, At: at, Reason: "test"}
		return l.add(writer, "irori", store.KindPersonDeviceRevoked, goldMarshal(t, DeviceRevoked{Revoke: r, PersonPub: goldPub(k), PersonSig: goldSign(t, k, r)}))
	}
	profile := func(writer, root string, k ed25519.PrivateKey, name string) store.Event {
		b := profileBody{PersonID: pid, Name: name, RootDeviceID: root, PersonPub: goldPub(k)}
		return l.add(writer, "irori", store.KindPersonProfile, goldMarshal(t, Profile{b.PersonID, b.Name, b.RootDeviceID, b.PersonPub, goldSign(t, k, b)}))
	}
	added("root", cert("root", "PC <1>"), pk1, 1)
	profile("root", "root", pk1, "Stephen")
	added("root", cert("b", "b"), pk1, 1)
	added("b", cert("c", "c"), pk1, 0)
	l.add("c", "irori", "task.created", []byte(`{"taskId":"t"}`))
	cRow := l.add("c", "irori", "task.created", []byte(`{"taskId":"u"}`))
	l.add("root", "irori", store.KindPersonKeyRotated, goldMarshal(t, KeyRotated{PersonID: pid, Epoch: 2, PersonPub: goldPub(pk2), At: at, By: "root"}))
	added("root", cert("d", "by the rotated key"), pk2, 2)
	revoked("b", "c", cRow.Seq, store.RowHash(cRow), pk1)
	// A revoke of the root below the cert of its own writer is raised.
	revoked("b", "root", 0, "", pk1)
	// Go decodes case-insensitively, keeps the last duplicate, drops
	// unknown fields, and re-formats times; the signature covers the
	// decoded struct.
	ce := cert("e", "cased")
	l.add("b", "irori", store.KindPersonDeviceAdded, []byte(`{"CERT":{"PERSONID":"`+pid+`","deviceid":"e","DevicePub":"`+ce.DevicePub+`","label":"x","label":"cased","Role":"desktop","At":"`+at.Format(time.RFC3339Nano)+`","extra":[1,{"a":null}]},"personpub":"`+goldPub(pk2)+`","PersonSig":"`+goldSign(t, pk2, ce)+`","unknown":true}`))
	cf := cert("f", "reparsed")
	cf.At = time.Date(2026, 1, 2, 3, 4, 5, 100000000, time.UTC)
	l.add("b", "irori", store.KindPersonDeviceAdded, []byte(strings.Replace(string(goldMarshal(t, DeviceAdded{Cert: cf, PersonPub: goldPub(pk2), PersonSig: goldSign(t, pk2, cf)})), `"2026-01-02T03:04:05.1Z"`, `"2026-01-02T03:04:05.100Z"`, 1)))
	// A signature over the literal spelling, not Go's: it doesn't verify.
	cg := cert("g", "literal")
	cg.At = cf.At
	lit := strings.Replace(string(goldMarshal(t, cg)), `"2026-01-02T03:04:05.1Z"`, `"2026-01-02T03:04:05.100Z"`, 1)
	var generic any
	_ = json.Unmarshal([]byte(lit), &generic)
	var litBuf bytes.Buffer
	enc := json.NewEncoder(&litBuf)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(generic)
	litSig := hex.EncodeToString(ed25519.Sign(pk2, bytes.TrimRight(litBuf.Bytes(), "\n")))
	l.add("b", "irori", store.KindPersonDeviceAdded, []byte(`{"cert":`+lit+`,"personPub":"`+goldPub(pk2)+`","personSig":"`+litSig+`"}`))
	// A missing "at" is the zero time; null leaves it zero too.
	ch := cert("h", "no time")
	ch.At = time.Time{}
	l.add("b", "irori", store.KindPersonDeviceAdded, []byte(`{"cert":{"personId":"`+pid+`","deviceId":"h","devicePub":"`+ch.DevicePub+`","label":"no time","role":"desktop","at":null},"personPub":"`+goldPub(pk2)+`","personSig":"`+goldSign(t, pk2, ch)+`"}`))
	// A wrong type fails the whole payload (bad), as does bad JSON.
	l.add("b", "irori", store.KindPersonDeviceAdded, []byte(`{"cert":{"personId":"`+pid+`","deviceId":"i","role":5}}`))
	l.add("b", "irori", store.KindPersonProfile, []byte(`{"personId":`))
	// A profile whose signature doesn't cover it.
	l.add("b", "irori", store.KindPersonProfile, goldMarshal(t, Profile{pid, "forged", "root", goldPub(pk1), goldSign(t, pk1, profileBody{pid, "real", "root", goldPub(pk1)})}))
	// Another person's rows are inert; a stranger's key certifies nobody.
	other := goldKey("person/x/1")
	oc := Cert{PersonID: PersonIDOf(other.Public().(ed25519.PublicKey)), DeviceID: "x", DevicePub: goldPub(goldKey("dev/x")), Role: RoleDesktop, At: at}
	l.add("x", "irori", store.KindPersonDeviceAdded, goldMarshal(t, DeviceAdded{Cert: oc, PersonPub: goldPub(other), PersonSig: goldSign(t, other, oc)}))
	added("root", cert("j", "stranger-signed"), goldKey("person/stranger"), 0)
	// A cert with invalid UTF-8 in its label: Go reads it as U+FFFD.
	ck := cert("k", "bad � byte")
	raw := goldMarshal(t, DeviceAdded{Cert: ck, PersonPub: goldPub(pk2), PersonSig: goldSign(t, pk2, ck)})
	raw = bytes.Replace(raw, []byte("�"), []byte{0xff}, 1)
	l.add("b", "irori", store.KindPersonDeviceAdded, raw)
	l.add("b", "irori", "task.created", []byte(`{}`))

	chain := NewChain(pid, "root", "self")
	for _, ev := range l.rows {
		chain.Observe(ev)
	}
	// Verify every row against the final state, plus a forged row and a
	// bootstrap check on a fresh chain.
	var verify []string
	for _, ev := range l.rows {
		verify = append(verify, errText(chain.VerifyRow(ev)))
	}
	forged := l.rows[len(l.rows)-1]
	forged.Sig = ed25519.Sign(goldKey("dev/zzz"), store.SigningBytes(forged))
	fresh := NewChain(pid, "root", "self")
	var bootstrap []string
	for _, ev := range l.rows[:4] {
		bootstrap = append(bootstrap, errText(fresh.Learn(ev)))
	}
	type cut struct {
		Device string `json:"device"`
		Cut    int64  `json:"cut"`
		OK     bool   `json:"ok"`
	}
	var cuts []cut
	for _, d := range []string{"root", "b", "c", "d", "e", "f", "zz"} {
		c, ok := chain.Cut(d)
		cuts = append(cuts, cut{d, c, ok})
	}
	owners := map[string]bool{}
	for _, k := range []string{"user:root", "user:self", "user:c", "user:g", "user:", "root/s", "user:x", "user:made-up"} {
		owners[k] = chain.IsOwner(k)
	}
	var rows []goldRow
	for _, ev := range l.rows {
		rows = append(rows, goldRowOf(ev))
	}
	goldWrite(t, "chain.json", map[string]any{
		"personId":     pid,
		"root":         "root",
		"self":         "self",
		"rows":         rows,
		"devices":      string(goldMarshal(t, chain.Devices())),
		"cuts":         cuts,
		"claims":       string(goldMarshal(t, chain.CutClaims())),
		"hasProfile":   chain.HasProfile(),
		"bad":          chain.Bad(),
		"verify":       verify,
		"forged":       goldRowOf(forged),
		"forgedVerify": errText(chain.VerifyRow(forged)),
		"bootstrap":    bootstrap,
		"owners":       owners,
	})
}

// TestGoldenIdentityBoard folds a board log with two people, a rotation,
// a revoke, a promotion, a removal and a leave, and records the board
// Verifier's answers and its verdict on every row.
func TestGoldenIdentityBoard(t *testing.T) {
	const boardID = "0123456789abcdef0123456789abcdef"
	s1, s2 := goldKey("person/s/1"), goldKey("person/s/2")
	b1 := goldKey("person/b/1")
	spid := PersonIDOf(s1.Public().(ed25519.PublicKey))
	bpid := PersonIDOf(b1.Public().(ed25519.PublicKey))
	spin := MemberPin{PersonID: spid, PersonPub: goldPub(s1), RootDeviceID: "pc", PersonEpoch: 1}
	bpin := MemberPin{PersonID: bpid, PersonPub: goldPub(b1), RootDeviceID: "bpc", PersonEpoch: 1}
	l := newGoldLog()
	at := time.Unix(1761700000, 868679000).UTC()
	sa := "person:" + spid
	ba := "person:" + bpid
	mcert := func(pid, writer, dev string, epoch int, k ed25519.PrivateKey) store.Event {
		c := MemberCert{PersonID: pid, DeviceID: dev, DevicePub: goldPub(goldKey("dev/" + dev)), Role: RoleDesktop, PersonEpoch: epoch, At: at}
		return l.add(writer, "irori", store.KindMemberDeviceAdded, goldMarshal(t, MemberDeviceAdded{Cert: c, PersonSig: goldSign(t, k, c)}))
	}
	signed := func(v any, sig *string, k ed25519.PrivateKey) []byte {
		*sig = goldSign(t, k, v)
		return goldMarshal(t, v)
	}
	mcert(spid, "pc", "pc", 1, s1)
	{
		p := BoardCreated{BoardID: boardID, Name: "MLG <&>", Tag: "mlg", Creator: spin, At: at}
		var sig string
		signed(p, &sig, s1)
		p.PersonSig = sig
		l.add("pc", sa, store.KindBoardCreated, goldMarshal(t, p))
	}
	addB := BoardMemberAdded{PersonID: bpid, PersonPub: bpin.PersonPub, RootDeviceID: "bpc", PersonEpoch: 1, Name: "Blake", Role: BoardMember, By: spid, ByEpoch: 1, At: at}
	addB.PersonSig = goldSign(t, s1, addB)
	l.add("pc", sa, store.KindBoardMemberAdded, goldMarshal(t, addB))
	mcert(bpid, "bpc", "bpc", 1, b1)
	mcert(bpid, "bpc", "blap", 1, b1)
	mcert(spid, "pc", "mac", 1, s1)
	{
		p := MemberProfile{PersonID: spid, Name: "Stephen", PersonEpoch: 1}
		p.PersonSig = goldSign(t, s1, memberProfileBody{p.PersonID, p.Name, p.PersonEpoch})
		l.add("mac", "irori", store.KindMemberProfile, goldMarshal(t, p))
	}
	{
		p := MemberProfile{PersonID: bpid, Name: "Blake <&>", PersonEpoch: 1}
		p.PersonSig = goldSign(t, b1, memberProfileBody{p.PersonID, p.Name, p.PersonEpoch})
		l.add("blap", "irori", store.KindMemberProfile, goldMarshal(t, p))
	}
	{
		r := MemberKeyRotated{PersonID: spid, PersonEpoch: 2, PersonPub: goldPub(s2), By: "pc", At: at}
		r.PrevSig = goldSign(t, s1, r.body())
		l.add("pc", "irori", store.KindMemberKeyRotated, goldMarshal(t, r))
	}
	mcert(spid, "mac", "node", 2, s2)
	mcert(spid, "mac", "late", 1, s1) // the ended epoch's key
	{
		p := BoardMemberRole{PersonID: bpid, Role: BoardOwner, By: spid, At: at, PersonEpoch: 2}
		p.PersonSig = goldSign(t, s2, p)
		l.add("pc", sa, store.KindBoardMemberRole, goldMarshal(t, p))
	}
	{
		p := BoardRenamed{Name: "ours", By: bpid, At: at, PersonEpoch: 1}
		p.PersonSig = goldSign(t, b1, p)
		l.add("bpc", ba, store.KindBoardRenamed, goldMarshal(t, p))
	}
	macRow := l.rows[6] // mac/1, the member.profile
	{
		r := MemberRevoke{PersonID: spid, DeviceID: "mac", BoardCut: macRow.Seq, BoardCutHash: store.RowHash(macRow), PersonEpoch: 2, At: at}
		l.add("pc", "irori", store.KindMemberDeviceRevoked, goldMarshal(t, MemberDeviceRevoked{Revoke: r, PersonSig: goldSign(t, s2, r)}))
	}
	l.add("bpc", "agent:bc", store.KindAgentCard, []byte(`{"cardId":"bc","personId":"`+bpid+`","name":"lead"}`))
	l.add("pc", "agent:sc", store.KindAgentCard, []byte(`{"cardId":"sc","personId":"`+spid+`"}`))
	{
		p := BoardMemberRole{PersonID: "nobody", Role: BoardOwner, By: spid, At: at, PersonEpoch: 2}
		p.PersonSig = goldSign(t, s2, p)
		l.add("pc", sa, store.KindBoardMemberRole, goldMarshal(t, p))
	}
	{
		p := BoardRenamed{Name: " bad name", By: spid, At: at, PersonEpoch: 2}
		p.PersonSig = goldSign(t, s2, p)
		l.add("pc", sa, store.KindBoardRenamed, goldMarshal(t, p))
	}
	{
		p := BoardMemberRemoved{PersonID: bpid, By: spid, Reason: "done", Cut: map[string]int64{"bpc": 3}, At: at, PersonEpoch: 2}
		p.PersonSig = goldSign(t, s2, p)
		l.add("pc", sa, store.KindBoardMemberRemoved, goldMarshal(t, p))
	}
	{
		p := BoardMemberLeft{PersonID: bpid, Cut: map[string]int64{"blap": 0}, At: at, PersonEpoch: 1}
		p.PersonSig = goldSign(t, b1, p)
		l.add("blap", ba, store.KindBoardMemberLeft, goldMarshal(t, p))
	}
	l.add("pc", "irori", store.KindBoardServer, []byte(`{"deviceId":"pc","port":1}`))
	l.add("pc", "irori", store.KindMemberProfile, []byte(`{"personId":5}`))

	v := NewBoardVerifierFor(boardID, spin)
	for _, ev := range l.rows {
		v.Observe(ev)
	}
	var verify []string
	for _, ev := range l.rows {
		verify = append(verify, errText(v.VerifyRow(ev)))
	}
	// Rows the board refuses for their actor, or holds for a card.
	type attempt struct {
		Row goldRow `json:"row"`
		Err string  `json:"err"`
	}
	var attempts []attempt
	try := func(writer, actor, kind, payload string) {
		ev := store.Event{DeviceID: writer, Seq: 99, HLC: hlcOf(l.hlc + 1<<16), Actor: actor, Kind: kind, Payload: json.RawMessage(payload)}
		ev.Sig = ed25519.Sign(goldKey("dev/"+writer), store.SigningBytes(ev))
		attempts = append(attempts, attempt{goldRowOf(ev), errText(v.VerifyRow(ev))})
	}
	try("pc", ba, "task.comment", `{"taskId":"t"}`)
	try("pc", "agent:fake", store.KindAgentCard, `{"cardId":"fake","personId":"`+bpid+`"}`)
	try("pc", "agent:bc", "task.comment", `{"taskId":"t"}`)
	try("pc", "agent:sc", "task.comment", `{"taskId":"t"}`)
	try("pc", "irori", store.KindMemberProfile, `{"personId":"`+bpid+`"}`)
	try("pc", "irori", store.KindMemberDeviceAdded, `{"cert":{"personId":"`+bpid+`"}}`)
	try("pc", "irori", store.KindMemberDeviceRevoked, `{"personId":"`+spid+`","revoke":{"personId":"`+bpid+`"}}`)
	try("pc", "irori", store.KindBoardServer, `{"deviceId":"bpc"}`)
	try("pc", "irori", "task.claimed", `{"taskId":"t","by":"agent:sc"}`)
	try("pc", "agent:sc", "task.claimed", `{"taskId":"t","by":"agent:sc"}`)
	try("pc", "agent:later", "task.comment", `{}`)
	try("pc", sa, store.KindBoardCreated, `{"creator":{"personId":"`+bpid+`"}}`)
	try("pc", sa, store.KindBoardCreated, `{"creator":null}`)
	try("pc", sa, store.KindBoardMemberLeft, `{"personId":"`+spid+`"}`)
	try("mac", sa, store.KindBoardMemberLeft, `{"personId":"`+spid+`"}`)
	try("zz", "irori", "task.comment", `{}`)
	try("node", sa, "task.comment", `{"by":"`+sa+`"}`)
	try("pc", sa, "task.comment", `[1,2]`)

	type devAns struct {
		Device   string `json:"device"`
		Cut      int64  `json:"cut"`
		CutOK    bool   `json:"cutOk"`
		PersonOf string `json:"personOf"`
		Cert     string `json:"cert"`
	}
	var devs []devAns
	for _, d := range []string{"pc", "mac", "node", "late", "bpc", "blap", "zz"} {
		c, ok := v.Cut(d)
		p, _ := v.PersonOf(d)
		cert, cok := v.Cert(d)
		cs := ""
		if cok {
			cs = string(goldMarshal(t, cert))
		}
		devs = append(devs, devAns{d, c, ok, p, cs})
	}
	type personAns struct {
		Person   string `json:"person"`
		Epoch    int    `json:"epoch"`
		Writable bool   `json:"writable"`
		Profile  string `json:"profile"`
		Has      bool   `json:"hasProfile"`
		Devices  string `json:"devices"`
		Pinned   string `json:"pinned"`
	}
	var people []personAns
	for _, p := range []string{spid, bpid, "nobody"} {
		name, has := v.Profile(p)
		pin, _ := v.Pinned(p)
		people = append(people, personAns{p, v.Epoch(p), v.Writable(p), name, has, string(goldMarshal(t, v.Devices(p))), string(goldMarshal(t, pin))})
	}
	keys := map[string]string{}
	for _, k := range []string{"agent:bc", "agent:sc", "agent:fake", "person:" + spid, "person:" + bpid, "person:nobody", "irori"} {
		keys[k] = v.KeyPerson(k)
	}
	var rows []goldRow
	for _, ev := range l.rows {
		rows = append(rows, goldRowOf(ev))
	}
	ms := v.Membership()
	goldWrite(t, "board.json", map[string]any{
		"boardId":    boardID,
		"anchor":     spin,
		"rows":       rows,
		"verify":     verify,
		"attempts":   attempts,
		"membership": string(goldMarshal(t, ms)),
		"pins":       string(goldMarshal(t, ms.Pins)),
		"boardEpoch": v.BoardEpoch(),
		"claims":     string(goldMarshal(t, v.CutClaims())),
		"devices":    devs,
		"people":     people,
		"keys":       keys,
		"ambiguous":  v.Ambiguous(),
		"bad":        v.Bad(),
	})
}

// TestGoldenIdentityFiles records the files Go writes: person.json with
// older epochs, a legacy prev-only person.json, and a board secrets file,
// all under the file keystore so the bytes are fixed.
func TestGoldenIdentityFiles(t *testing.T) {
	dir := t.TempDir()
	k1, k2, k3 := goldKey("person/s/1"), goldKey("person/s/2"), goldKey("person/s/3")
	p := Person{ID: PersonIDOf(k1.Public().(ed25519.PublicKey)), Name: "Stephen <&>", Epoch: 3, RootDeviceID: "pc", Pub: goldPub(k3)}
	path := filepath.Join(dir, PersonFile)
	if err := writePerson(path, FileKeystore{}, p, k3, []epochKey{{1, k1}, {2, k2}}); err != nil {
		t.Fatal(err)
	}
	withOlder, _ := os.ReadFile(path)
	if err := writePerson(path, FileKeystore{}, p, k3, nil); err != nil {
		t.Fatal(err)
	}
	noOlder, _ := os.ReadFile(path)
	// What an earlier build wrote: one prev epoch.
	w2, _ := seal(FileKeystore{}, k2.Seed())
	w3, _ := seal(FileKeystore{}, k3.Seed())
	legacy := personFile{PersonID: p.ID, Name: p.Name, Epoch: 3, RootDeviceID: "pc", PersonPub: p.Pub, SecretKey: w3, Prev: &prevKey{Epoch: 2, PersonPub: goldPub(k2), SecretKey: w2}}
	legacyRaw, _ := json.MarshalIndent(legacy, "", "  ")
	if err := os.WriteFile(path, legacyRaw, 0o600); err != nil {
		t.Fatal(err)
	}
	dk, _ := seal(FileKeystore{}, goldKey("dev/pc").Seed())
	if err := writeSealed(filepath.Join(dir, DeviceKeyFile), dk); err != nil {
		t.Fatal(err)
	}
	deviceKey, _ := os.ReadFile(filepath.Join(dir, DeviceKeyFile))
	dev := device.Identity{ID: "pc", Name: "PC", OS: "windows"}
	re, err := Load(dir, dev, FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	if re.PrevEpoch() != 2 {
		t.Fatal("legacy prev not read")
	}
	devJSON, _ := os.ReadFile(filepath.Join(dir, "device.json"))

	spath := filepath.Join(dir, "0123456789abcdef0123456789abcdef.secrets")
	s, err := OpenBoardSecrets(spath, "0123456789abcdef0123456789abcdef", FileKeystore{})
	if err != nil {
		t.Fatal(err)
	}
	steps := []string{}
	must := func(name string, err error) {
		if err != nil {
			t.Fatal(name, err)
		}
		raw, _ := os.ReadFile(spath)
		var w wrapped
		_ = json.Unmarshal(raw, &w)
		steps = append(steps, string(w.Blob))
	}
	must("add 1", s.Add(1, "pc", bytes.Repeat([]byte{7}, 32)))
	must("add 2", s.Add(2, "pc", bytes.Repeat([]byte{8}, 32)))
	must("add 2 mac", s.Add(2, "aaa-mac", bytes.Repeat([]byte{9}, 32)))
	must("add 10", s.Add(10, "pc", bytes.Repeat([]byte{10}, 32)))
	must("add 3", s.Add(3, "pc", bytes.Repeat([]byte{3}, 32)))
	must("settle 10", s.Settle(10))
	secretsFile, _ := os.ReadFile(spath)
	goldWrite(t, "files.json", map[string]any{
		"personWithOlder": string(withOlder),
		"personNoOlder":   string(noOlder),
		"personLegacy":    string(legacyRaw),
		"deviceJson":      string(devJSON),
		"deviceKey":       string(deviceKey),
		"secretSteps":     steps,
		"secretsFile":     string(secretsFile),
	})
}

// TestGoldenIdentityTables records the name, tag and owner-key tables.
func TestGoldenIdentityTables(t *testing.T) {
	names := []string{"mlg", "Blake & me", "", " mlg", "mlg ", "a\nb", "a\u0085b", " x", "x　", "x ", "a\u007fb",
		"a\u009fb", "a​b", "a­b", "日本語", "😀", "�", "\t", strings.Repeat("x", 80), strings.Repeat("x", 81),
		strings.Repeat("é", 80), strings.Repeat("é", 81), "x ", "᠎x", " "}
	tags := []string{"mlg", "a", "mlg-2", "abcdefghijklmnop", "", "Mlg", "12", "1abc", "ml#g", "abcdefghijklmnopq", "ml g", "-x", "a-", "a_b", "é", "z9"}
	keys := []string{"pc/aaaa-session", "mac/grok-1", "user:pc", "irori", "", "remote:github.com/a/b", "/x", "user:", "a:b/c", "pc/", "user:x/y", "agent:pc/s"}
	nameAns := map[string]bool{}
	for _, n := range names {
		nameAns[n] = ValidBoardName(n)
	}
	tagAns := map[string]bool{}
	for _, tg := range tags {
		tagAns[tg] = ValidBoardTag(tg)
	}
	type keyAns struct {
		Key      string `json:"key"`
		Device   string `json:"device"`
		MayPC    bool   `json:"mayPc"`
		MayOther bool   `json:"mayOther"`
	}
	var keyAnswers []keyAns
	for _, k := range keys {
		keyAnswers = append(keyAnswers, keyAns{k, owner.DeviceOf(k), owner.MayWrite(k, "pc"), owner.MayWrite(k, "other")})
	}
	entities := map[string]string{}
	for _, r := range owner.Table {
		for _, k := range append(append([]string{}, r.OwnerKinds...), r.RequestKinds...) {
			e, _ := owner.EntityOf(k)
			entities[k] = e
		}
	}
	goldWrite(t, "tables.json", map[string]any{"names": nameAns, "tags": tagAns, "keys": keyAnswers, "entities": entities})
}
