package inbox

import (
	"errors"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

func searchDocs(t *testing.T, h *harness, since time.Time) []search.Doc {
	t.Helper()
	docs, err := h.ib.SearchDocs(bg, since, 100)
	if err != nil {
		t.Fatal(err)
	}
	return docs
}

func msgRefs(docs []search.Doc) []string {
	var out []string
	for _, d := range docs {
		out = append(out, d.Ref)
	}
	return out
}

func TestSearchDocsMessages(t *testing.T) {
	h := newHarness(t)
	h.abc()
	m1, _ := h.send(claude(keyA, "alpha"), to(keyB), "the walrus is loose")
	h.clk.advance(time.Minute)
	m2, _ := h.send(person(), to(keyC), "check the pen")
	h.clk.advance(time.Minute)
	h.ib.SetReachable(keyA, PathMod, true) // repo1's front desk
	m3, d3 := h.send(claude(keyC, "gamma"), toRepo("repo1"), "repo work")

	docs := searchDocs(t, h, time.Time{})
	if got := msgRefs(docs); !slices.Equal(got, []string{"msg:" + m1.ID, "msg:" + m2.ID, "msg:" + m3.ID}) {
		t.Fatalf("docs %v (%+v)", got, d3)
	}
	tests := []struct {
		d                  search.Doc
		title, agent, repo string
		body               string
	}{
		{docs[0], "alpha -> beta", keyA, "", "the walrus is loose"},
		{docs[1], "Stephen -> gamma", keyC, "", "check the pen"}, // a person's message belongs to its recipient
		{docs[2], "gamma -> repo1", keyC, "repo1", "repo work"},
	}
	for _, tt := range tests {
		if tt.d.Kind != search.KindMsg || tt.d.Title != tt.title || tt.d.Agent != tt.agent || tt.d.RepoKey != tt.repo || tt.d.Body != tt.body {
			t.Errorf("doc %+v, want title %q agent %q repo %q", tt.d, tt.title, tt.agent, tt.repo)
		}
	}
	if got := msgRefs(searchDocs(t, h, m2.SentAt)); !slices.Contains(got, "msg:"+m2.ID) {
		t.Fatalf("since is inclusive: %v", got)
	}
	if got, _ := h.ib.SearchDocs(bg, time.Time{}, 1); len(got) != 1 || got[0].Ref != "msg:"+m1.ID {
		t.Fatalf("limit: %v", msgRefs(got))
	}
}

func TestSearchHeldMessagesStayHidden(t *testing.T) {
	h := newHarness(t)
	h.abc()
	if err := h.ib.SetPolicy(bg, Policy{Block: []string{"alpha"}}, user); err != nil {
		t.Fatal(err)
	}
	held, hd := h.send(claude(keyA, "alpha"), to(keyB), "secret plan one")
	dropped, dd := h.send(claude(keyA, "alpha"), to(keyB), "secret plan two")
	h.wantState(hd.ID, Held)
	h.wantState(dd.ID, Held)

	if got := searchDocs(t, h, time.Time{}); len(got) != 0 {
		t.Fatalf("held messages searchable: %v", msgRefs(got))
	}
	for _, id := range []string{held.ID, dropped.ID} {
		if _, err := h.ib.SearchShow(bg, "msg:"+id, search.ShowOptions{}); !errors.Is(err, search.ErrNotFound) {
			t.Fatalf("show held %s: %v", id, err)
		}
	}

	h.clk.advance(time.Hour)
	if err := h.ib.Dismiss(bg, dd.ID, user); err != nil {
		t.Fatal(err)
	}
	if err := h.ib.Release(bg, hd.ID, user); err != nil {
		t.Fatal(err)
	}
	docs := searchDocs(t, h, time.Time{})
	if got := msgRefs(docs); !slices.Equal(got, []string{"msg:" + held.ID}) {
		t.Fatalf("after release/dismiss: %v", got)
	}
	// The released message comes back at its release, so a pull that had
	// already passed its SentAt still sees it.
	if !docs[0].Updated.After(held.SentAt) || !docs[0].At.Equal(held.SentAt) {
		t.Fatalf("released doc %+v", docs[0])
	}
	if got := msgRefs(searchDocs(t, h, held.SentAt.Add(time.Minute))); !slices.Equal(got, []string{"msg:" + held.ID}) {
		t.Fatalf("pull after SentAt: %v", got)
	}
	if _, err := h.ib.SearchShow(bg, "msg:"+dropped.ID, search.ShowOptions{}); !errors.Is(err, search.ErrNotFound) {
		t.Fatalf("show dismissed: %v", err)
	}
}

func TestSearchShowMessage(t *testing.T) {
	h := newHarness(t)
	h.abc()
	m, _ := h.send(claude(keyA, "alpha"), to(keyB), "please look at the board card")
	// SendOptions.Ref is SendWith's (A2); set it on the folded message here.
	h.ib.impl.mu.Lock()
	h.ib.impl.msgs[m.ID].Ref = "task:T1"
	h.ib.impl.mu.Unlock()
	h.deliverEngage(keyB, "turn-1")
	ds := h.ib.For(keyB, true, 10)
	rm, _, err := h.ib.Reply(bg, claude(keyB, "beta"), ds[0].ID, "on it")
	if err != nil {
		t.Fatal(err)
	}

	sh, err := h.ib.SearchShow(bg, "msg:"+m.ID, search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	for _, s := range []string{"from alpha (" + keyA + ", claude)", "trust: untrusted", "about: task:T1", "to " + keyB + ": answered", "text:\nplease look at the board card"} {
		if !strings.Contains(sh.Text, s) {
			t.Errorf("text lacks %q:\n%s", s, sh.Text)
		}
	}
	if want := []string{"msg:" + rm.ID, "task:T1"}; !slices.Equal(sh.Related, want) {
		t.Fatalf("related %v, want %v", sh.Related, want)
	}
	reply, err := h.ib.SearchShow(bg, "msg:"+rm.ID, search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if !slices.Equal(reply.Related, []string{"msg:" + m.ID}) || reply.Title != "beta -> alpha" {
		t.Fatalf("reply %+v", reply)
	}
	for _, ref := range []string{"msg:nope", "task:T1"} {
		if _, err := h.ib.SearchShow(bg, ref, search.ShowOptions{}); !errors.Is(err, search.ErrNotFound) {
			t.Errorf("%s: %v", ref, err)
		}
	}
	if _, err := h.ib.SearchShow(bg, "bad", search.ShowOptions{}); !errors.Is(err, search.ErrBadRef) {
		t.Errorf("bad ref: %v", err)
	}
}
