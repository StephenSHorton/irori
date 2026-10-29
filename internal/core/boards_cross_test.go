package core

import (
	"context"
	"errors"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

// TestBoardCardsAcrossOwnDevices: stage 1 is one person on several
// devices, so a shared card works with agents on any of them (§2.4,
// §5.2). The PC assigns a shared card to the Mac's agent, which hears it
// in its inbox; the Mac's devices both translate the agent's card; a card
// shared from the PC is refused for writes on the Mac too; and Withdraw
// carries the Mac agent's hold over to the private card.
func TestBoardCardsAcrossOwnDevices(t *testing.T) {
	ctx := context.Background()
	net := mesh.NewFakeNet()
	pc := newHCore(t, net, "pc", "aaaaaaaa-1111-4111-8111-00000000000a")
	mac := newHCore(t, net, "mac", "bbbbbbbb-2222-4222-8222-00000000000b")
	pairCores(t, pc, mac, identity.RoleDesktop)
	wait(t, "the pc sees the mac's session", func() bool { return hasCard(pc, mac.agent(), "connected") })

	id, err := pc.c.CreateBoard(ctx, "MLG", "mlg")
	if err != nil {
		t.Fatal(err)
	}
	wait(t, "the mac opens the board", func() bool {
		b, err := mac.c.board(id)
		return err == nil && b.verifier.Membership().Created
	})
	pcb := mustBoard(t, pc.c, id)

	// The PC puts the Mac's agent on a shared card.
	me, _ := pc.c.BoardKey(ctx, id, pc.user())
	card, err := pcb.coord.CreateTask(ctx, me, coord.NewTask{Title: "the mac builds it"})
	if err != nil {
		t.Fatal(err)
	}
	co, keys, err := pc.c.TaskCoord(ctx, card.ID, pc.user(), mac.agent())
	if err != nil {
		t.Fatalf("assigning the mac's agent from the pc: %v", err)
	}
	if keys[1] == mac.agent() || keys[1][:len(store.AgentActorPrefix)] != store.AgentActorPrefix {
		t.Fatalf("the mac's agent as the board knows it: %q", keys[1])
	}
	_, n, err := co.Assign(ctx, keys[0], card.ID, keys[1])
	if err != nil {
		t.Fatal(err)
	}
	if got := pc.c.PrivateNotice(n).To; len(got) != 1 || got[0] != mac.agent() {
		t.Fatalf("the pc's notice goes to %v, want the mac's agent", got)
	}
	pc.c.tellNotices(ctx, n)
	wait(t, "the mac's agent hears the assignment", func() bool {
		for _, d := range mac.c.Inbox().For(mac.agent(), true, 50) {
			if d.Message.Ref == "task:"+card.ID || d.Message.Ref == card.ID {
				return true
			}
		}
		return false
	})
	wait(t, "the mac translates the card the pc made", func() bool { return mac.c.PrivateKey(keys[1]) == mac.agent() })
	if k, err := mac.c.BoardKey(ctx, id, mac.agent()); err != nil || k != keys[1] {
		t.Fatalf("the mac's own key for its agent: %q, %v; want the pc's card %q", k, err, keys[1])
	}

	// A private card shared from the PC is closed to writes on the Mac.
	priv, err := pc.c.Coord().CreateTask(ctx, pc.agent(), coord.NewTask{Title: "go public"})
	if err != nil {
		t.Fatal(err)
	}
	p, err := pc.c.PreviewShare(priv.ID, id)
	if err != nil {
		t.Fatal(err)
	}
	shared, err := pc.c.ShareToBoard(ctx, priv.ID, id, p.Hash)
	if err != nil {
		t.Fatal(err)
	}
	wait(t, "the mac learns the shared copy", func() bool {
		sc, ok := mac.c.SharedCopyOf(priv.ID)
		return ok && sc.TaskID == shared.ID
	})
	wait(t, "the mac has the private card", func() bool { _, ok := mac.c.Coord().GetTask(priv.ID); return ok })
	if _, _, err := mac.c.TaskCoord(ctx, priv.ID, mac.agent()); !errors.Is(err, ErrShared) {
		t.Fatalf("a write to the shared private card on the mac: %v", err)
	}

	// The Mac's agent holds the shared card; Withdraw from the PC hands
	// its hold to the private card it came from.
	wait(t, "the mac has the shared card", func() bool { id, _, err := mac.c.Route(shared.ID); return err == nil && id != "" })
	mco, mkeys, err := mac.c.TaskCoord(ctx, shared.ID, mac.agent())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := mco.ClaimTask(ctx, mkeys[0], shared.ID); err != nil {
		t.Fatal(err)
	}
	wait(t, "the pc sees the mac's claim", func() bool {
		x, _ := pcb.coord.GetTask(shared.ID)
		return len(x.Holders) == 1
	})
	back, err := pc.c.WithdrawCard(ctx, shared.ID)
	if err != nil {
		t.Fatal(err)
	}
	if back.ID != priv.ID {
		t.Fatalf("withdraw carried on in %q, want the private card %q", back.ID, priv.ID)
	}
	got, _ := pc.c.Coord().GetTask(priv.ID)
	held := false
	for _, k := range append(got.Assigned, got.Holders...) {
		held = held || k == mac.agent()
	}
	if !held {
		t.Fatalf("the mac's agent lost its work on withdraw: %+v", got)
	}
}
