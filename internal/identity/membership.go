package identity

import (
	"encoding/json"
	"sort"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/store"
)

// A board's membership (design/v0.5-shared-boards.md §2.1, §4.2-4.4): its
// board.* rows, each signed by the acting person's key, folded into who is
// on the board, who owns it, who was removed or left, and with which cuts.
//
// The fold has two layers, because a cut can't be undone (the store moves
// rows past it out of the log for good, v0.4 §5.5):
//
//   - authority, which decides everything that cuts: who has been an owner
//     since when, which removals and leaves count, and so each removed
//     person's cuts. It only grows as rows arrive, so every arrival order
//     makes the same cuts. A removal counts when its by had been made an
//     owner (by board.created, or a grant from someone who already was one)
//     before it, whatever happened to by afterwards: as v0.4's "a revoke
//     counts if its writer was ever certified", two owners removing each
//     other while apart are both removed;
//   - state, the ordered fold in (hlc, device, seq) order that says who is
//     a member now and with which role: every board.member.* and
//     board.renamed row counts only when by is an owner at that point, and
//     a removal is sticky (§4.2): once a person is removed or has left,
//     role rows about them are ignored until a board.member.added for them
//     by an owner with a higher HLC. Every ignored row is a Conflict, so
//     the owner who promoted someone concurrently with their removal sees
//     it didn't take.
//
// FoldMembership is pure: the board Verifier hands it the rows and what its
// member chains say (MembershipKeys).

// Board roles and member states.
const (
	BoardOwner  = "owner"
	BoardMember = "member"

	MemberActive  = "active"
	MemberRemoved = "removed"
	MemberLeft    = "left"
)

// BoardCreated is the board.created payload: self-signed by the creator's
// person key, for the epoch its creator pin names. The creator's board root
// is the device that wrote it.
type BoardCreated struct {
	BoardID   string    `json:"boardId"`
	Name      string    `json:"name"`
	Tag       string    `json:"tag"`
	Creator   MemberPin `json:"creator"`
	At        time.Time `json:"at"`
	PersonSig string    `json:"personSig,omitempty"`
}

// BoardRenamed is the board.renamed payload, by an owner.
type BoardRenamed struct {
	Name        string    `json:"name"`
	By          string    `json:"by"`
	At          time.Time `json:"at"`
	PersonEpoch int       `json:"personEpoch"`
	PersonSig   string    `json:"personSig,omitempty"`
}

// BoardMemberAdded is the board.member.added payload: an owner pins a
// person on the board. PersonEpoch is the added person's, as proven at the
// invite (§3); ByEpoch is the epoch of by's key that signs the row, which
// §2.1's row leaves unnamed.
type BoardMemberAdded struct {
	PersonID     string    `json:"personId"`
	PersonPub    string    `json:"personPub"`
	RootDeviceID string    `json:"rootDeviceId"`
	PersonEpoch  int       `json:"personEpoch"`
	Name         string    `json:"name"`
	Role         string    `json:"role"`
	By           string    `json:"by"`
	ByEpoch      int       `json:"byEpoch"`
	At           time.Time `json:"at"`
	PersonSig    string    `json:"personSig,omitempty"`
}

// Pin is the pin the row gives its person.
func (p BoardMemberAdded) Pin() MemberPin {
	return MemberPin{PersonID: p.PersonID, PersonPub: p.PersonPub, RootDeviceID: p.RootDeviceID, PersonEpoch: p.PersonEpoch}
}

// BoardMemberRole is the board.member.role payload, by an owner.
type BoardMemberRole struct {
	PersonID    string    `json:"personId"`
	Role        string    `json:"role"`
	By          string    `json:"by"`
	At          time.Time `json:"at"`
	PersonEpoch int       `json:"personEpoch"`
	PersonSig   string    `json:"personSig,omitempty"`
}

// BoardMemberRemoved is the board.member.removed payload: Cut holds, for
// each of the person's certified devices, the highest seq of its stream in
// this board log the remover held. A device of theirs it doesn't name is
// cut at 0.
type BoardMemberRemoved struct {
	PersonID    string           `json:"personId"`
	By          string           `json:"by"`
	Reason      string           `json:"reason,omitempty"`
	Cut         map[string]int64 `json:"cut"`
	At          time.Time        `json:"at"`
	PersonEpoch int              `json:"personEpoch"`
	PersonSig   string           `json:"personSig,omitempty"`
}

// BoardMemberLeft is the board.member.left payload, written by the leaving
// person's own device with a cut over their own devices' streams. The
// writing device's cut is never below the row itself.
type BoardMemberLeft struct {
	PersonID    string           `json:"personId"`
	Cut         map[string]int64 `json:"cut"`
	At          time.Time        `json:"at"`
	PersonEpoch int              `json:"personEpoch"`
	PersonSig   string           `json:"personSig,omitempty"`
}

// BoardCreatedRow makes a board.created payload for a board this person
// creates, pinned as pin (BoardPin).
func (id *Identity) BoardCreatedRow(boardID, name, tag string, pin MemberPin, at time.Time) (BoardCreated, error) {
	p := BoardCreated{BoardID: boardID, Name: name, Tag: tag, Creator: pin, At: at.UTC()}
	sig, err := id.signEpoch(pin.PersonEpoch, p)
	p.PersonSig = sig
	return p, err
}

// BoardRenamedRow makes a board.renamed payload, signed for epoch (the
// board's newest for this person).
func (id *Identity) BoardRenamedRow(epoch int, name string, at time.Time) (BoardRenamed, error) {
	p := BoardRenamed{Name: name, By: id.Current().ID, At: at.UTC(), PersonEpoch: epoch}
	sig, err := id.signEpoch(epoch, p)
	p.PersonSig = sig
	return p, err
}

// BoardMemberAddedRow makes a board.member.added payload pinning pin.
func (id *Identity) BoardMemberAddedRow(epoch int, pin MemberPin, name, role string, at time.Time) (BoardMemberAdded, error) {
	p := BoardMemberAdded{PersonID: pin.PersonID, PersonPub: pin.PersonPub, RootDeviceID: pin.RootDeviceID, PersonEpoch: pin.PersonEpoch,
		Name: name, Role: role, By: id.Current().ID, ByEpoch: epoch, At: at.UTC()}
	sig, err := id.signEpoch(epoch, p)
	p.PersonSig = sig
	return p, err
}

// BoardMemberRoleRow makes a board.member.role payload.
func (id *Identity) BoardMemberRoleRow(epoch int, personID, role string, at time.Time) (BoardMemberRole, error) {
	p := BoardMemberRole{PersonID: personID, Role: role, By: id.Current().ID, At: at.UTC(), PersonEpoch: epoch}
	sig, err := id.signEpoch(epoch, p)
	p.PersonSig = sig
	return p, err
}

// BoardMemberRemovedRow makes a board.member.removed payload.
func (id *Identity) BoardMemberRemovedRow(epoch int, personID, reason string, cut map[string]int64, at time.Time) (BoardMemberRemoved, error) {
	p := BoardMemberRemoved{PersonID: personID, By: id.Current().ID, Reason: reason, Cut: cut, At: at.UTC(), PersonEpoch: epoch}
	sig, err := id.signEpoch(epoch, p)
	p.PersonSig = sig
	return p, err
}

// BoardMemberLeftRow makes a board.member.left payload for this person.
func (id *Identity) BoardMemberLeftRow(epoch int, cut map[string]int64, at time.Time) (BoardMemberLeft, error) {
	p := BoardMemberLeft{PersonID: id.Current().ID, Cut: cut, At: at.UTC(), PersonEpoch: epoch}
	sig, err := id.signEpoch(epoch, p)
	p.PersonSig = sig
	return p, err
}

// RowAt is a row's place in fold order.
type RowAt struct {
	HLC    uint64 `json:"hlc"`
	Device string `json:"device"`
	Seq    int64  `json:"seq"`
}

func (a RowAt) less(b RowAt) bool {
	return rowPos{a.HLC, a.Device, a.Seq}.less(rowPos{b.HLC, b.Device, b.Seq})
}

// MembershipRow is one board.* row as FoldMembership reads it.
type MembershipRow struct {
	At      RowAt
	Kind    string
	Payload json.RawMessage
}

// MembershipRowOf is ev as a MembershipRow.
func MembershipRowOf(ev store.Event) MembershipRow {
	return MembershipRow{At: RowAt{uint64(ev.HLC), ev.DeviceID, ev.Seq}, Kind: ev.Kind, Payload: ev.Payload}
}

// IsMembershipKind reports the board.* rows FoldMembership folds
// (board.server is the transport's, §6.2).
func IsMembershipKind(kind string) bool {
	switch kind {
	case store.KindBoardCreated, store.KindBoardRenamed, store.KindBoardMemberAdded,
		store.KindBoardMemberRole, store.KindBoardMemberRemoved, store.KindBoardMemberLeft:
		return true
	}
	return false
}

// MembershipKeys is what FoldMembership asks of the board's identity
// chains (§3).
type MembershipKeys interface {
	// PersonOf is the member whose chain ever certified device, cuts
	// ignored ("" for none).
	PersonOf(device string) string
	// Signed reports whether sig is personID's key of epoch over
	// canonical(body), and a row at at may still be signed by it.
	Signed(personID string, epoch int, at RowAt, sig string, body any) bool
	// HasKey reports whether personID's chain holds pub as epoch's key.
	HasKey(personID string, epoch int, pub string) bool
	// Devices lists every device ever certified to personID.
	Devices(personID string) []string
	// Cut is device's revocation cut (member.device.revoked).
	Cut(device string) (int64, bool)
}

// BoardPerson is one person as the membership fold sees them.
type BoardPerson struct {
	PersonID string    `json:"personId"`
	Name     string    `json:"name,omitempty"`
	Role     string    `json:"role"`
	State    string    `json:"state"`
	Pin      MemberPin `json:"pin"`
	Since    RowAt     `json:"since"`
}

// Conflict is a board.* row the fold ignored, with why: one line in the
// Boards dialog for the owners (§4.2).
type Conflict struct {
	At       RowAt  `json:"at"`
	Kind     string `json:"kind"`
	PersonID string `json:"personId,omitempty"`
	By       string `json:"by,omitempty"`
	Reason   string `json:"reason"`
}

// Membership is a board's folded membership.
type Membership struct {
	// Created: a board.created matching an anchored creator counted.
	// Nothing else counts without one.
	Created bool   `json:"created"`
	BoardID string `json:"boardId"`
	Name    string `json:"name"`
	Tag     string `json:"tag"`
	Creator string `json:"creator"`
	// Members is everyone the board ever added, in the order they joined.
	Members []BoardPerson `json:"members"`
	// Pins are the anchors and the pin of each person an owner added.
	Pins map[string]MemberPin `json:"-"`
	// Cuts is each removed or left person's devices' cuts.
	Cuts map[string]int64 `json:"cuts,omitempty"`
	// Gone lists the persons removed or left, by a counted row, sorted.
	Gone      []string   `json:"gone,omitempty"`
	Conflicts []Conflict `json:"conflicts,omitempty"`
}

// Person is personID's entry.
func (m Membership) Person(personID string) (BoardPerson, bool) {
	for _, p := range m.Members {
		if p.PersonID == personID {
			return p, true
		}
	}
	return BoardPerson{}, false
}

// Active reports personID is a current member.
func (m Membership) Active(personID string) bool {
	p, ok := m.Person(personID)
	return ok && p.State == MemberActive
}

// Owner reports personID is a current owner.
func (m Membership) Owner(personID string) bool {
	p, ok := m.Person(personID)
	return ok && p.State == MemberActive && p.Role == BoardOwner
}

// mrow is one decoded membership row.
type mrow struct {
	at    RowAt
	kind  string
	by    string // the acting person
	who   string // the person it is about
	epoch int    // by's epoch that signs it
	sig   string
	body  any // what sig covers
	name  string
	tag   string
	board string
	role  string
	pin   MemberPin
	cut   map[string]int64
	ok    bool // decoded
}

func decodeMembership(r MembershipRow) mrow {
	d := mrow{at: r.At, kind: r.Kind}
	switch r.Kind {
	case store.KindBoardCreated:
		var p BoardCreated
		if json.Unmarshal(r.Payload, &p) != nil {
			return d
		}
		d.sig, p.PersonSig = p.PersonSig, ""
		d.by, d.who, d.epoch, d.body, d.name, d.tag, d.board, d.pin = p.Creator.PersonID, p.Creator.PersonID, p.Creator.PersonEpoch, p, p.Name, p.Tag, p.BoardID, p.Creator
	case store.KindBoardRenamed:
		var p BoardRenamed
		if json.Unmarshal(r.Payload, &p) != nil {
			return d
		}
		d.sig, p.PersonSig = p.PersonSig, ""
		d.by, d.epoch, d.body, d.name = p.By, p.PersonEpoch, p, p.Name
	case store.KindBoardMemberAdded:
		var p BoardMemberAdded
		if json.Unmarshal(r.Payload, &p) != nil {
			return d
		}
		d.sig, p.PersonSig = p.PersonSig, ""
		d.by, d.who, d.epoch, d.body, d.name, d.role, d.pin = p.By, p.PersonID, p.ByEpoch, p, p.Name, p.Role, p.Pin()
	case store.KindBoardMemberRole:
		var p BoardMemberRole
		if json.Unmarshal(r.Payload, &p) != nil {
			return d
		}
		d.sig, p.PersonSig = p.PersonSig, ""
		d.by, d.who, d.epoch, d.body, d.role = p.By, p.PersonID, p.PersonEpoch, p, p.Role
	case store.KindBoardMemberRemoved:
		var p BoardMemberRemoved
		if json.Unmarshal(r.Payload, &p) != nil {
			return d
		}
		d.sig, p.PersonSig = p.PersonSig, ""
		d.by, d.who, d.epoch, d.body, d.cut = p.By, p.PersonID, p.PersonEpoch, p, p.Cut
	case store.KindBoardMemberLeft:
		var p BoardMemberLeft
		if json.Unmarshal(r.Payload, &p) != nil {
			return d
		}
		d.sig, p.PersonSig = p.PersonSig, ""
		d.by, d.who, d.epoch, d.body, d.cut = p.PersonID, p.PersonID, p.PersonEpoch, p, p.Cut
	default:
		return d
	}
	d.ok = d.by != ""
	return d
}

// FoldMembership folds a board's membership rows (the two layers above).
// anchors are the pins this device trusts from outside the log: the
// creator, from its own creation or an invite bundle (§2.1); a
// board.created counts only for an anchored creator, written by its board
// root, for this boardID ("" skips that check). It is pure and doesn't
// depend on the order of rows.
func FoldMembership(boardID string, anchors map[string]MemberPin, rows []MembershipRow, k MembershipKeys) Membership {
	ds := make([]mrow, 0, len(rows))
	for _, r := range rows {
		if d := decodeMembership(r); d.ok {
			ds = append(ds, d)
		}
	}
	sort.Slice(ds, func(i, j int) bool { return ds[i].at.less(ds[j].at) })

	m := Membership{BoardID: boardID, Pins: map[string]MemberPin{}}
	for id, p := range anchors {
		m.Pins[id] = p
	}
	authored := func(d mrow) bool {
		return d.by != "" && k.PersonOf(d.at.Device) == d.by && k.Signed(d.by, d.epoch, d.at, d.sig, d.body)
	}

	// The one board.created: the first that matches an anchor.
	created := -1
	for i, d := range ds {
		if d.kind != store.KindBoardCreated {
			continue
		}
		a, ok := anchors[d.pin.PersonID]
		if ok && a == d.pin && d.at.Device == d.pin.RootDeviceID && (boardID == "" || d.board == boardID) && authored(d) {
			created = i
			break
		}
	}
	if created < 0 {
		return m
	}
	c := ds[created]
	m.Created, m.Name, m.Tag, m.Creator = true, c.name, c.tag, c.by

	// Authority: owner since when, grown to a fixed point (more owners
	// count more grants), then the pins, removals and leaves that count.
	since := map[string]RowAt{c.by: c.at}
	ownerBefore := func(p string, at RowAt) bool {
		s, ok := since[p]
		return ok && s.less(at)
	}
	for changed := true; changed; {
		changed = false
		for _, d := range ds {
			grant := (d.kind == store.KindBoardMemberAdded || d.kind == store.KindBoardMemberRole) && d.role == BoardOwner
			if !grant || d.who == "" || !ownerBefore(d.by, d.at) || !authored(d) {
				continue
			}
			if s, ok := since[d.who]; !ok || d.at.less(s) {
				since[d.who], changed = d.at, true
			}
		}
	}
	for _, d := range ds {
		if d.kind != store.KindBoardMemberAdded || d.who == "" || !validPin(d.pin) {
			continue
		}
		if _, have := m.Pins[d.who]; have || !ownerBefore(d.by, d.at) || !authored(d) {
			continue
		}
		m.Pins[d.who] = d.pin // the earliest counted add pins
	}
	honoured := map[RowAt]bool{}
	gone := map[string][]mrow{}
	for _, d := range ds {
		if _, pinned := m.Pins[d.who]; !pinned {
			continue
		}
		switch {
		case d.kind == store.KindBoardMemberRemoved && d.who != d.by && ownerBefore(d.by, d.at) && authored(d),
			d.kind == store.KindBoardMemberLeft && authored(d):
			honoured[d.at] = true
			gone[d.who] = append(gone[d.who], d)
		}
	}
	m.Cuts = map[string]int64{}
	for who, rs := range gone {
		m.Gone = append(m.Gone, who)
		for _, dev := range k.Devices(who) {
			var cut int64 = -1
			for _, r := range rs {
				v := r.cut[dev]
				if r.kind == store.KindBoardMemberLeft && r.at.Device == dev && r.at.Seq > v {
					v = r.at.Seq // the row that cuts can't cut itself
				}
				if cut < 0 || v < cut {
					cut = v
				}
			}
			// No cut reaches below board.created: it anchors the board
			// (the creator's root self-cert comes before it), and a
			// removal or leave that cut it would undo the board itself.
			if dev == c.at.Device && cut < c.at.Seq {
				cut = c.at.Seq
			}
			m.Cuts[dev] = cut
		}
	}
	sort.Strings(m.Gone)

	// State: the ordered fold.
	people := map[string]*BoardPerson{}
	var order []string
	put := func(p BoardPerson) {
		if _, ok := people[p.PersonID]; !ok {
			order = append(order, p.PersonID)
		}
		people[p.PersonID] = &p
	}
	owner := func(p string) bool {
		e := people[p]
		return e != nil && e.State == MemberActive && e.Role == BoardOwner
	}
	owners := func() int {
		n := 0
		for _, e := range people {
			if e.State == MemberActive && e.Role == BoardOwner {
				n++
			}
		}
		return n
	}
	past := func(at RowAt) bool {
		if cut, ok := m.Cuts[at.Device]; ok && at.Seq > cut {
			return true
		}
		cut, ok := k.Cut(at.Device)
		return ok && at.Seq > cut
	}
	conflict := func(d mrow, reason string) {
		m.Conflicts = append(m.Conflicts, Conflict{At: d.at, Kind: d.kind, PersonID: d.who, By: d.by, Reason: reason})
	}
	for i, d := range ds {
		gate := d.kind == store.KindBoardMemberRemoved || d.kind == store.KindBoardMemberLeft
		if !gate && past(d.at) {
			continue // the store refuses it; nothing to show
		}
		if i < created {
			conflict(d, "it comes before the board was created")
			continue
		}
		switch d.kind {
		case store.KindBoardCreated:
			if i == created {
				put(BoardPerson{PersonID: c.by, Role: BoardOwner, State: MemberActive, Pin: c.pin, Since: c.at})
			} else {
				conflict(d, "the board was already created")
			}
		case store.KindBoardRenamed:
			switch {
			case !authored(d):
				conflict(d, "its signature doesn't count")
			case !owner(d.by):
				conflict(d, "only an owner renames the board")
			case !ValidBoardName(d.name):
				conflict(d, "the name is not a board name")
			default:
				m.Name = d.name
			}
		case store.KindBoardMemberAdded:
			pin, pinned := m.Pins[d.who]
			e := people[d.who]
			switch {
			case !authored(d):
				conflict(d, "its signature doesn't count")
			case !owner(d.by):
				conflict(d, "only an owner adds members")
			case d.role != BoardOwner && d.role != BoardMember:
				conflict(d, "the role is not owner or member")
			case !pinned || (pin != d.pin && !k.HasKey(d.who, d.pin.PersonEpoch, d.pin.PersonPub)):
				conflict(d, "it pins another key than the one this board holds for them")
			case e != nil && e.State == MemberActive:
				conflict(d, "they are already a member")
			default:
				put(BoardPerson{PersonID: d.who, Name: d.name, Role: d.role, State: MemberActive, Pin: pin, Since: d.at})
			}
		case store.KindBoardMemberRole:
			e := people[d.who]
			switch {
			case !authored(d):
				conflict(d, "its signature doesn't count")
			case !owner(d.by):
				conflict(d, "only an owner changes roles")
			case d.role != BoardOwner && d.role != BoardMember:
				conflict(d, "the role is not owner or member")
			case e == nil:
				conflict(d, "they are not on the board")
			case e.State != MemberActive:
				conflict(d, "they were removed or left before it; the removal stands")
			case e.Role == BoardOwner && d.role == BoardMember && owners() == 1:
				conflict(d, "the last owner can't stop being one")
			default:
				e.Role = d.role
			}
		case store.KindBoardMemberRemoved, store.KindBoardMemberLeft:
			state := MemberRemoved
			if d.kind == store.KindBoardMemberLeft {
				state = MemberLeft
			}
			if !honoured[d.at] {
				if d.kind == store.KindBoardMemberLeft {
					conflict(d, "it isn't signed by the person leaving")
				} else {
					conflict(d, "only an owner removes members")
				}
				continue
			}
			if d.kind == store.KindBoardMemberRemoved && !owner(d.by) {
				// It cuts all the same (cuts can't be undone, so they never
				// depend on fold order); the owners see why.
				conflict(d, "its remover was no longer an owner, but a removal always stands")
			}
			e := people[d.who]
			if e == nil {
				put(BoardPerson{PersonID: d.who, Role: BoardMember, State: state, Pin: m.Pins[d.who], Since: d.at})
				continue
			}
			if e.State == MemberActive {
				e.State = state
			}
		}
	}
	for _, id := range order {
		m.Members = append(m.Members, *people[id])
	}
	return m
}

func validPin(p MemberPin) bool {
	return p.PersonID != "" && p.RootDeviceID != "" && p.PersonEpoch >= 1 && validPub(p.PersonPub)
}

// MaxBoardName caps a board's name, in runes.
const MaxBoardName = 80

// ValidBoardName reports a board name: 1 to MaxBoardName runes, no
// surrounding space, no control characters.
func ValidBoardName(name string) bool {
	if name == "" || strings.TrimSpace(name) != name || utf8.RuneCountInString(name) > MaxBoardName || !utf8.ValidString(name) {
		return false
	}
	for _, r := range name {
		if unicode.IsControl(r) {
			return false
		}
	}
	return true
}

// ValidBoardTag reports a board's short handle as refs carry it (`mlg#12`,
// decision B3): a lowercase letter, then up to 15 lowercase letters,
// digits or '-'. It never holds '#' and is never a bare number, so a ref's
// tag and number always split one way.
func ValidBoardTag(tag string) bool {
	if tag == "" || len(tag) > 16 || tag[0] < 'a' || tag[0] > 'z' {
		return false
	}
	for i := 1; i < len(tag); i++ {
		c := tag[i]
		if (c < 'a' || c > 'z') && (c < '0' || c > '9') && c != '-' {
			return false
		}
	}
	return true
}
