package identity

import (
	"encoding/json"
	"fmt"
	"reflect"
	"sort"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/store"
)

// fakeKeys is MembershipKeys without crypto: a device belongs to the
// person its name starts with ("s-pc" is s's), and a row is signed when
// its sig is "ok:<person>".
type fakeKeys struct {
	devs map[string][]string // person -> devices
	cuts map[string]int64
}

func (k fakeKeys) PersonOf(dev string) string {
	for p, ds := range k.devs {
		for _, d := range ds {
			if d == dev {
				return p
			}
		}
	}
	return ""
}

func (k fakeKeys) Signed(p string, epoch int, at RowAt, sig string, body any) bool {
	return epoch == 1 && sig == "ok:"+p
}

func (k fakeKeys) HasKey(p string, epoch int, pub string) bool { return false }
func (k fakeKeys) Devices(p string) []string                   { return k.devs[p] }
func (k fakeKeys) Cut(dev string) (int64, bool) {
	c, ok := k.cuts[dev]
	return c, ok
}

var testKeys = fakeKeys{devs: map[string][]string{
	"s": {"s-pc", "s-mac"}, "b": {"b-pc"}, "c": {"c-pc"}, "d": {"d-pc"},
}}

// pinOf is a person's pin in these tests: rooted at <person>-pc.
func pinOf(p string) MemberPin {
	return MemberPin{PersonID: p, PersonPub: strings.Repeat("a", 64), RootDeviceID: p + "-pc", PersonEpoch: 1}
}

// mb builds membership rows at increasing HLCs, per device seqs.
type mb struct {
	hlc  uint64
	seq  map[string]int64
	rows []MembershipRow
}

func newMB() *mb { return &mb{seq: map[string]int64{}} }

func (b *mb) add(dev, kind string, payload any) MembershipRow {
	b.hlc++
	b.seq[dev]++
	raw, _ := json.Marshal(payload)
	r := MembershipRow{At: RowAt{b.hlc, dev, b.seq[dev]}, Kind: kind, Payload: raw}
	b.rows = append(b.rows, r)
	return r
}

func (b *mb) created(by string) MembershipRow {
	return b.add(by+"-pc", store.KindBoardCreated, BoardCreated{BoardID: "bd", Name: "mlg", Tag: "mlg", Creator: pinOf(by), PersonSig: "ok:" + by})
}

func (b *mb) added(dev, by, who, role string) MembershipRow {
	p := pinOf(who)
	return b.add(dev, store.KindBoardMemberAdded, BoardMemberAdded{PersonID: who, PersonPub: p.PersonPub, RootDeviceID: p.RootDeviceID,
		PersonEpoch: 1, Name: who, Role: role, By: by, ByEpoch: 1, PersonSig: "ok:" + by})
}

func (b *mb) role(dev, by, who, role string) MembershipRow {
	return b.add(dev, store.KindBoardMemberRole, BoardMemberRole{PersonID: who, Role: role, By: by, PersonEpoch: 1, PersonSig: "ok:" + by})
}

func (b *mb) removed(dev, by, who string, cut map[string]int64) MembershipRow {
	return b.add(dev, store.KindBoardMemberRemoved, BoardMemberRemoved{PersonID: who, By: by, Cut: cut, PersonEpoch: 1, PersonSig: "ok:" + by})
}

func (b *mb) left(dev, who string, cut map[string]int64) MembershipRow {
	return b.add(dev, store.KindBoardMemberLeft, BoardMemberLeft{PersonID: who, Cut: cut, PersonEpoch: 1, PersonSig: "ok:" + who})
}

func (b *mb) renamed(dev, by, name string) MembershipRow {
	return b.add(dev, store.KindBoardRenamed, BoardRenamed{Name: name, By: by, PersonEpoch: 1, PersonSig: "ok:" + by})
}

// roles is "person=role/state" for every member, sorted.
func roles(m Membership) string {
	var out []string
	for _, p := range m.Members {
		out = append(out, p.PersonID+"="+p.Role+"/"+p.State)
	}
	sort.Strings(out)
	return strings.Join(out, " ")
}

func reasons(m Membership) []string {
	var out []string
	for _, c := range m.Conflicts {
		out = append(out, c.Kind+" "+c.PersonID+": "+c.Reason)
	}
	return out
}

var anchorS = map[string]MemberPin{"s": pinOf("s")}

// TestFoldMembership: the membership fold's rules, one row set each, every
// one folded in every arrival order to the same Membership (§4.2-4.4).
func TestFoldMembership(t *testing.T) {
	cases := []struct {
		name      string
		rows      func(b *mb)
		roles     string
		cuts      map[string]int64
		conflicts []string // substrings, one per conflict, in order
		boardName string
	}{
		{
			name:  "the creator is the first owner",
			rows:  func(b *mb) { b.created("s") },
			roles: "s=owner/active",
		},
		{
			name: "an owner adds members and owners",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.added("s-mac", "s", "c", BoardOwner)
			},
			roles: "b=member/active c=owner/active s=owner/active",
		},
		{
			name: "a non-owner's add, role and rename are ignored",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.added("b-pc", "b", "c", BoardMember)
				b.role("b-pc", "b", "b", BoardOwner)
				b.renamed("b-pc", "b", "blake's")
			},
			roles:     "b=member/active s=owner/active",
			conflicts: []string{"board.member.added c: only an owner adds", "board.member.role b: only an owner changes", "board.renamed : only an owner renames"},
			boardName: "mlg",
		},
		{
			name: "membership chains back to board.created: a promoted owner adds",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.role("s-pc", "s", "b", BoardOwner)
				b.added("b-pc", "b", "c", BoardMember)
				b.renamed("b-pc", "b", "ours")
			},
			roles:     "b=owner/active c=member/active s=owner/active",
			boardName: "ours",
		},
		{
			name: "the last owner can't demote themselves; with two owners one can",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.role("s-pc", "s", "s", BoardMember)
				b.role("s-pc", "s", "b", BoardOwner)
				b.role("b-pc", "b", "b", BoardMember)
			},
			roles:     "b=member/active s=owner/active",
			conflicts: []string{"board.member.role s: the last owner"},
		},
		{
			name: "a removal cuts every device of the person, named or not",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.removed("s-pc", "s", "b", map[string]int64{"b-pc": 7})
			},
			roles: "b=member/removed s=owner/active",
			cuts:  map[string]int64{"b-pc": 7},
		},
		{
			name: "a device the remover didn't know of is cut at 0",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "c", BoardMember)
				b.removed("s-pc", "s", "c", map[string]int64{"b-pc": 9})
			},
			roles: "c=member/removed s=owner/active",
			cuts:  map[string]int64{"c-pc": 0},
		},
		{
			name: "concurrent remove and promote: the removal stands and the promote is a conflict",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "c", BoardOwner)
				b.added("s-pc", "s", "d", BoardMember)
				b.removed("s-pc", "s", "d", map[string]int64{"d-pc": 2})
				b.role("c-pc", "c", "d", BoardOwner) // c hadn't heard
			},
			roles:     "c=owner/active d=member/removed s=owner/active",
			cuts:      map[string]int64{"d-pc": 2},
			conflicts: []string{"board.member.role d: they were removed or left before it; the removal stands"},
		},
		{
			name: "a promote before the removal is undone by it",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "c", BoardOwner)
				b.added("s-pc", "s", "d", BoardMember)
				b.role("c-pc", "c", "d", BoardOwner)
				b.removed("s-pc", "s", "d", map[string]int64{"d-pc": 0})
			},
			roles: "c=owner/active d=owner/removed s=owner/active",
			cuts:  map[string]int64{"d-pc": 0},
		},
		{
			name: "a removed person's rows are ignored until an owner adds them again",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardOwner)
				b.removed("s-pc", "s", "b", map[string]int64{"b-pc": 1})
				b.role("s-pc", "s", "b", BoardMember)
				b.added("s-pc", "s", "b", BoardMember)
			},
			roles:     "b=member/active s=owner/active",
			cuts:      map[string]int64{"b-pc": 1},
			conflicts: []string{"board.member.role b: they were removed or left"},
		},
		{
			name: "leaving cuts the leaver, never below the leave row itself",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.add("b-pc", "noise", nil) // b-pc seq 1, then the leave at 2
				b.left("b-pc", "b", map[string]int64{"b-pc": 1})
			},
			roles: "b=member/left s=owner/active",
			cuts:  map[string]int64{"b-pc": 2},
		},
		{
			name: "a leave for someone else, or a removal by a non-owner, counts nowhere",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.added("s-pc", "s", "c", BoardMember)
				b.add("b-pc", store.KindBoardMemberLeft, BoardMemberLeft{PersonID: "c", PersonEpoch: 1, PersonSig: "ok:b"})
				b.removed("b-pc", "b", "c", nil)
			},
			roles:     "b=member/active c=member/active s=owner/active",
			conflicts: []string{"board.member.left c: it isn't signed by the person leaving", "board.member.removed c: only an owner removes"},
		},
		{
			name: "two owners removing each other while apart are both removed",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardOwner)
				b.added("s-pc", "s", "c", BoardMember)
				b.removed("s-pc", "s", "b", map[string]int64{"b-pc": 0})
				b.removed("b-pc", "b", "s", map[string]int64{"s-pc": 3, "s-mac": 0})
			},
			roles:     "b=owner/removed c=member/active s=owner/removed",
			cuts:      map[string]int64{"b-pc": 0, "s-pc": 3, "s-mac": 0},
			conflicts: []string{"board.member.removed s: its remover was no longer an owner"},
		},
		{
			name: "rows by a forged writer or another person's key count nowhere",
			rows: func(b *mb) {
				b.created("s")
				b.add("b-pc", store.KindBoardMemberAdded, BoardMemberAdded{PersonID: "c", Role: BoardOwner, By: "s", ByEpoch: 1, PersonSig: "ok:s",
					PersonPub: pinOf("c").PersonPub, RootDeviceID: "c-pc", PersonEpoch: 1})
				b.add("s-pc", store.KindBoardMemberAdded, BoardMemberAdded{PersonID: "d", Role: BoardMember, By: "s", ByEpoch: 1, PersonSig: "ok:b",
					PersonPub: pinOf("d").PersonPub, RootDeviceID: "d-pc", PersonEpoch: 1})
			},
			roles:     "s=owner/active",
			conflicts: []string{"board.member.added c: its signature doesn't count", "board.member.added d: its signature doesn't count"},
		},
		{
			name: "only the anchored creator's board.created counts",
			rows: func(b *mb) {
				b.created("b") // b isn't anchored here
				b.created("s")
				b.created("s")
			},
			roles:     "s=owner/active",
			conflicts: []string{"board.created b: it comes before", "board.created s: the board was already created"},
		},
		{
			name: "a re-add can't change the key the board pins",
			rows: func(b *mb) {
				b.created("s")
				b.added("s-pc", "s", "b", BoardMember)
				b.removed("s-pc", "s", "b", map[string]int64{})
				b.add("s-pc", store.KindBoardMemberAdded, BoardMemberAdded{PersonID: "b", Role: BoardMember, By: "s", ByEpoch: 1, PersonSig: "ok:s",
					PersonPub: strings.Repeat("b", 64), RootDeviceID: "b-pc", PersonEpoch: 1})
			},
			roles:     "b=member/removed s=owner/active",
			cuts:      map[string]int64{"b-pc": 0},
			conflicts: []string{"board.member.added b: it pins another key"},
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			b := newMB()
			tc.rows(b)
			want := FoldMembership("bd", anchorS, b.rows, testKeys)
			if !want.Created {
				t.Fatal("board.created didn't count")
			}
			if got := roles(want); got != tc.roles {
				t.Errorf("members %q, want %q", got, tc.roles)
			}
			wantCuts := tc.cuts
			if wantCuts == nil {
				wantCuts = map[string]int64{}
			}
			if !reflect.DeepEqual(want.Cuts, wantCuts) {
				t.Errorf("cuts %v, want %v", want.Cuts, wantCuts)
			}
			got := reasons(want)
			if len(got) != len(tc.conflicts) {
				t.Fatalf("conflicts %q, want %d like %q", got, len(tc.conflicts), tc.conflicts)
			}
			for i, sub := range tc.conflicts {
				if !strings.Contains(got[i], sub) {
					t.Errorf("conflict %d = %q, want it to contain %q", i, got[i], sub)
				}
			}
			if tc.boardName != "" && want.Name != tc.boardName {
				t.Errorf("name %q, want %q", want.Name, tc.boardName)
			}
			if len(b.rows) > 7 {
				return // 8! orders is plenty elsewhere
			}
			perm(len(b.rows), func(idx []int) {
				rows := make([]MembershipRow, len(idx))
				for i, j := range idx {
					rows[i] = b.rows[j]
				}
				if m := FoldMembership("bd", anchorS, rows, testKeys); !reflect.DeepEqual(m, want) {
					t.Fatalf("order %v folds to %s, want %s", idx, dump(m), dump(want))
				}
			})
		})
	}
}

func dump(m Membership) string {
	return fmt.Sprintf("%s cuts=%v conflicts=%q", roles(m), m.Cuts, reasons(m))
}

// TestFoldMembershipNeedsItsBoard: without an anchored board.created for
// this board, nothing counts.
func TestFoldMembershipNeedsItsBoard(t *testing.T) {
	b := newMB()
	b.created("s")
	b.added("s-pc", "s", "b", BoardOwner)
	for name, m := range map[string]Membership{
		"another board": FoldMembership("other", anchorS, b.rows, testKeys),
		"no anchor":     FoldMembership("bd", nil, b.rows, testKeys),
		"another root": FoldMembership("bd", map[string]MemberPin{"s": {PersonID: "s", PersonPub: pinOf("s").PersonPub,
			RootDeviceID: "s-mac", PersonEpoch: 1}}, b.rows, testKeys),
	} {
		if m.Created || len(m.Members) != 0 {
			t.Errorf("%s: created %v, members %q", name, m.Created, roles(m))
		}
	}
}

func TestBoardNamesAndTags(t *testing.T) {
	for tag, ok := range map[string]bool{"mlg": true, "a": true, "mlg-2": true, "abcdefghijklmnop": true,
		"": false, "Mlg": false, "12": false, "1abc": false, "ml#g": false, "abcdefghijklmnopq": false, "ml g": false, "-x": false} {
		if ValidBoardTag(tag) != ok {
			t.Errorf("ValidBoardTag(%q) = %v", tag, !ok)
		}
	}
	for name, ok := range map[string]bool{"mlg": true, "Blake & me": true, "": false, " mlg": false, "a\nb": false,
		strings.Repeat("x", MaxBoardName): true, strings.Repeat("x", MaxBoardName+1): false} {
		if ValidBoardName(name) != ok {
			t.Errorf("ValidBoardName(%q) = %v", name, !ok)
		}
	}
}
