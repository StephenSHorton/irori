package identity

import (
	"crypto/ed25519"
	"crypto/rand"
	"errors"
	mrand "math/rand/v2"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// The membership fold inside the board Verifier and a board store, with
// real keys: board.created and board.member.added pin members, removal and
// leaving cut, and the store honours both past the writer's own cut.

// stranger is a person the board doesn't anchor: only an owner's
// board.member.added pins them.
func (f *bfold) stranger(name, root string) *bperson {
	pub, key, _ := ed25519.GenerateKey(rand.Reader)
	p := &bperson{pid: PersonIDOf(pub), root: root, keys: map[int]ed25519.PrivateKey{1: key}}
	f.ppl[name] = p
	return p
}

func (p *bperson) pin() MemberPin {
	return MemberPin{PersonID: p.pid, PersonPub: pubHex(p.keys[1]), RootDeviceID: p.root, PersonEpoch: 1}
}

func (f *bfold) boardCreated(p *bperson) store.Event {
	r := BoardCreated{BoardID: "", Name: "mlg", Tag: "mlg", Creator: p.pin(), At: time.Unix(1, 0).UTC()}
	r.PersonSig = signHex(f.t, p.keys[1], r)
	return f.row(p.root, "person:"+p.pid, store.KindBoardCreated, r)
}

func (f *bfold) memberAdded(by *bperson, writer string, who *bperson, role string) store.Event {
	pin := who.pin()
	r := BoardMemberAdded{PersonID: pin.PersonID, PersonPub: pin.PersonPub, RootDeviceID: pin.RootDeviceID, PersonEpoch: 1,
		Name: "x", Role: role, By: by.pid, ByEpoch: 1, At: time.Unix(2, 0).UTC()}
	r.PersonSig = signHex(f.t, by.keys[1], r)
	return f.row(writer, "person:"+by.pid, store.KindBoardMemberAdded, r)
}

func (f *bfold) memberRemoved(by *bperson, writer string, who *bperson, cut map[string]int64) store.Event {
	r := BoardMemberRemoved{PersonID: who.pid, By: by.pid, Cut: cut, At: time.Unix(3, 0).UTC(), PersonEpoch: 1}
	r.PersonSig = signHex(f.t, by.keys[1], r)
	return f.row(writer, "person:"+by.pid, store.KindBoardMemberRemoved, r)
}

func (f *bfold) memberLeft(who *bperson, writer string, cut map[string]int64) store.Event {
	r := BoardMemberLeft{PersonID: who.pid, Cut: cut, At: time.Unix(4, 0).UTC(), PersonEpoch: 1}
	r.PersonSig = signHex(f.t, who.keys[1], r)
	return f.row(writer, "person:"+who.pid, store.KindBoardMemberLeft, r)
}

// shuffles runs check over n seeded random orders of evs, each into a
// fresh Verifier anchored on anchors.
func shuffles(t *testing.T, n int, evs []store.Event, anchors []MemberPin, check func(v *BoardVerifier)) {
	t.Helper()
	r := mrand.New(mrand.NewPCG(1, 2))
	for range n {
		idx := r.Perm(len(evs))
		v := NewBoardVerifier(anchors...)
		for _, i := range idx {
			v.Observe(evs[i])
		}
		check(v)
	}
}

// TestBoardMembershipPinsAndCuts: the anchored creator's board.created
// starts the board; an owner's board.member.added pins Blake, whose root
// self-cert then verifies from that pin alone; removing him cuts every
// device of his and moves the board epoch, in every arrival order.
func TestBoardMembershipPinsAndCuts(t *testing.T) {
	f := newBFold(t)
	s := f.person("stephen", "pc")
	b := f.stranger("blake", "bpc")
	sroot := f.cert(s, 1, "pc", "pc")
	created := f.boardCreated(s)
	broot := f.cert(b, 1, "bpc", "bpc")
	if err := f.v.VerifyRow(broot); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("blake's self-cert before anyone added him: err = %v", err)
	}
	add := f.memberAdded(s, "pc", b, BoardMember)
	f.feed(sroot, created, add)
	if err := f.v.VerifyRow(broot); err != nil {
		t.Fatalf("blake's self-cert once an owner added him: %v", err)
	}
	if _, ok := f.v.Pinned(b.pid); !ok {
		t.Fatal("the add didn't pin blake")
	}
	blap := f.cert(b, 1, "bpc", "blap")
	p1 := f.row("blap", "irori", store.KindMemberProfile, MemberProfile{PersonID: b.pid})
	rm := f.memberRemoved(s, "pc", b, map[string]int64{"bpc": 2})
	// A forged add: Blake's device writes as Stephen.
	forged := f.memberAdded(s, "bpc", f.stranger("carol", "cpc"), BoardOwner)
	evs := []store.Event{sroot, created, add, broot, blap, p1, rm, forged}
	anchor := s.pin()
	shuffles(t, 300, evs, []MemberPin{anchor}, func(v *BoardVerifier) {
		m := v.Membership()
		if !m.Created || !m.Owner(s.pid) {
			t.Fatalf("stephen isn't the board's owner: %s", roles(m))
		}
		if p, _ := m.Person(b.pid); p.State != MemberRemoved {
			t.Fatalf("blake is %q", p.State)
		}
		if _, ok := m.Person(f.ppl["carol"].pid); ok {
			t.Fatal("a forged add put carol on the board")
		}
		if c, ok := v.Cut("bpc"); !ok || c != 2 {
			t.Fatalf("bpc cut %d %v, want 2", c, ok)
		}
		if c, ok := v.Cut("blap"); !ok || c != 0 {
			t.Fatalf("blap, which the removal didn't name, cut %d %v, want 0", c, ok)
		}
		if _, ok := v.Cut("pc"); ok {
			t.Fatal("the remover is cut")
		}
		if e := v.BoardEpoch(); e != 2 {
			t.Fatalf("board epoch %d, want 2", e)
		}
	})
	// The self-cert of a person nobody added never verifies.
	g := newBFold(t)
	gs := g.person("stephen", "pc")
	g.feed(g.cert(gs, 1, "pc", "pc"), g.boardCreated(gs))
	other := g.stranger("blake", "bpc")
	if err := g.v.VerifyRow(g.cert(other, 1, "bpc", "bpc")); !errors.Is(err, store.ErrNoCert) {
		t.Fatalf("an unadded person's self-cert: err = %v", err)
	}
	// And board.* rows bind their acting person to the writer.
	if err := g.v.VerifyRow(g.row("pc", "person:"+gs.pid, store.KindBoardRenamed, BoardRenamed{Name: "x", By: other.pid})); !errors.Is(err, store.ErrBoardActor) {
		t.Fatalf("a rename naming another person as by: err = %v", err)
	}
}

// TestBoardRemovalInTheStore: two owners who remove each other while apart
// are both removed whichever removal a member hears first, since a store
// honours a removal past its writer's cut (it is kept in refused_events and
// folded at Open); a removed member's later rows are refused; a leave cuts
// the leaver after its own row.
func TestBoardRemovalInTheStore(t *testing.T) {
	ctx := t.Context()
	f := newBFold(t)
	s := f.person("stephen", "pc")
	b := f.stranger("blake", "bpc")
	c := f.stranger("carol", "cpc")
	base := []store.Event{
		f.cert(s, 1, "pc", "pc"), f.boardCreated(s), f.memberAdded(s, "pc", b, BoardOwner), f.memberAdded(s, "pc", c, BoardMember),
		f.cert(b, 1, "bpc", "bpc"), f.cert(c, 1, "cpc", "cpc"),
	}
	sRemovesB := f.memberRemoved(s, "pc", b, map[string]int64{"bpc": 1})                     // pc seq 5
	bRemovesS := f.memberRemoved(b, "bpc", s, map[string]int64{"pc": 4})                     // bpc seq 2
	bLater := f.row("bpc", "irori", store.KindMemberProfile, MemberProfile{PersonID: b.pid}) // bpc seq 3
	cLeaves := f.memberLeft(c, "cpc", map[string]int64{"cpc": 1})                            // cpc seq 2
	cLater := f.row("cpc", "irori", store.KindMemberProfile, MemberProfile{PersonID: c.pid})

	type result struct {
		Roles string
		Cuts  map[string]int64
		Epoch int
	}
	run := func(t *testing.T, order []store.Event) result {
		dir := t.TempDir()
		open := func() (*store.Store, *BoardVerifier) {
			v := NewBoardVerifier(s.pin())
			st, err := store.OpenWith(ctx, filepath.Join(dir, "b.db"), "self", hlc.New(nil), store.OpenOptions{Verifier: v, Board: true})
			if err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() { st.Close() }) // Close is safe twice (sql.DB)
			return st, v
		}
		st, v := open()
		for _, ev := range append(append([]store.Event{}, base...), order...) {
			if err := st.Insert(ctx, ev); err != nil {
				t.Fatalf("%s %s/%d: %v", ev.Kind, ev.DeviceID, ev.Seq, err)
			}
		}
		for _, ev := range []store.Event{bLater, cLater} {
			if err := st.Insert(ctx, ev); !errors.Is(err, store.ErrPastCut) {
				t.Fatalf("%s/%d after its person left the board: err = %v", ev.DeviceID, ev.Seq, err)
			}
		}
		got := result{Roles: roles(v.Membership()), Cuts: v.Membership().Cuts, Epoch: v.BoardEpoch()}
		st.Close()
		st, v = open()
		defer st.Close()
		if again := (result{Roles: roles(v.Membership()), Cuts: v.Membership().Cuts, Epoch: v.BoardEpoch()}); !reflect.DeepEqual(again, got) {
			t.Fatalf("after reopen %+v, before %+v", again, got)
		}
		return got
	}
	first := run(t, []store.Event{sRemovesB, bRemovesS, cLeaves})
	second := run(t, []store.Event{bRemovesS, cLeaves, sRemovesB})
	want := result{
		Roles: roles(Membership{Members: []BoardPerson{
			{PersonID: s.pid, Role: BoardOwner, State: MemberRemoved},
			{PersonID: b.pid, Role: BoardOwner, State: MemberRemoved},
			{PersonID: c.pid, Role: BoardMember, State: MemberLeft},
		}}),
		Cuts:  map[string]int64{"bpc": 1, "pc": 4, "cpc": 2},
		Epoch: 4,
	}
	if !reflect.DeepEqual(first, want) || !reflect.DeepEqual(second, want) {
		t.Fatalf("orders fold to\n %+v\n %+v\nwant %+v", first, second, want)
	}
}

// TestBoardRevokedDeviceHasNoSayPastItsCut: only a revoke is honoured past
// a revoked device's cut. A laptop its person revoked can't make the
// person leave (or remove anyone) with a row it signs afterwards, however
// it stamps it; and no leave or removal cuts below board.created, which
// anchors the board.
func TestBoardRevokedDeviceHasNoSayPastItsCut(t *testing.T) {
	ctx := t.Context()
	f := newBFold(t)
	s := f.person("stephen", "pc")
	v := NewBoardVerifier(s.pin())
	st, err := store.OpenWith(ctx, filepath.Join(t.TempDir(), "b.db"), "self", hlc.New(nil), store.OpenOptions{Verifier: v, Board: true})
	if err != nil {
		t.Fatal(err)
	}
	defer st.Close()
	insert := func(evs ...store.Event) {
		t.Helper()
		for _, ev := range evs {
			if err := st.Insert(ctx, ev); err != nil {
				t.Fatalf("%s %s/%d: %v", ev.Kind, ev.DeviceID, ev.Seq, err)
			}
		}
	}
	insert(f.cert(s, 1, "pc", "pc"), f.boardCreated(s), f.cert(s, 1, "pc", "laptop"),
		f.row("laptop", "irori", store.KindMemberProfile, MemberProfile{PersonID: s.pid}))
	insert(f.revoke(s, 1, "pc", "laptop", 1))
	// The stolen laptop signs a leave for its person, cutting the pc to 0.
	left := f.memberLeft(s, "laptop", map[string]int64{"pc": 0, "laptop": 2})
	if err := st.Insert(ctx, left); !errors.Is(err, store.ErrPastCut) {
		t.Fatalf("a revoked device's leave past its cut: err = %v", err)
	}
	m := v.Membership()
	if !m.Created || !m.Active(s.pid) {
		t.Fatalf("the board after a revoked device's leave: %+v", m)
	}
	// A revoke past its writer's cut is honoured, but never stamped
	// before its writer's last row here.
	back := f.revoke(s, 1, "laptop", "pc", 0)
	back.HLC = 1
	back.Sig = ed25519.Sign(f.dev("laptop"), store.SigningBytes(back))
	if err := st.Insert(ctx, back); !errors.Is(err, store.ErrHLCOrder) {
		t.Fatalf("a backdated revoke past its writer's cut: err = %v", err)
	}

	// A leave the laptop signed before its revoke, cutting the pc to 0,
	// still keeps board.created (pc seq 2): the cut is floored there.
	f2 := newBFold(t)
	s2 := f2.person("stephen", "pc")
	v2 := NewBoardVerifier(s2.pin())
	st2, err := store.OpenWith(ctx, filepath.Join(t.TempDir(), "b2.db"), "self", hlc.New(nil), store.OpenOptions{Verifier: v2, Board: true})
	if err != nil {
		t.Fatal(err)
	}
	defer st2.Close()
	for _, ev := range []store.Event{f2.cert(s2, 1, "pc", "pc"), f2.boardCreated(s2), f2.cert(s2, 1, "pc", "laptop"),
		f2.memberLeft(s2, "laptop", map[string]int64{"pc": 0})} {
		if err := st2.Insert(ctx, ev); err != nil {
			t.Fatalf("%s %s/%d: %v", ev.Kind, ev.DeviceID, ev.Seq, err)
		}
	}
	if cut, ok := v2.Cut("pc"); !ok || cut != 2 {
		t.Fatalf("pc cut %d %v; board.created is pc/2", cut, ok)
	}
	if !v2.Membership().Created {
		t.Fatal("a leave cut board.created away")
	}
}
