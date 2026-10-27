package inbox

import (
	"errors"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

func (h *harness) docs(since time.Time) map[string]search.Doc {
	h.t.Helper()
	ds, err := h.ib.SearchDocs(bg, since, 0)
	if err != nil {
		h.t.Fatal(err)
	}
	out := map[string]search.Doc{}
	for i, d := range ds {
		if i > 0 && d.Updated.Before(ds[i-1].Updated) {
			h.t.Fatalf("docs not oldest first: %v", ds)
		}
		out[d.Ref] = d
	}
	return out
}

func TestSearchVisibility(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	t0 := h.clk.now()
	plain, _ := h.send(claude(keyA, "alpha"), to("beta"), "parser plan")
	h.clk.advance(time.Minute)
	held, hd := h.send(grok(keyG, "grok"), to("beta"), "held words") // bypass receiver
	dropped, dd := h.send(grok(keyG, "grok"), to("gamma"), "dismissed words")
	if hd.State != Held || dd.State != Held {
		t.Fatal("not held")
	}
	if err := h.ib.Dismiss(bg, dd.ID, user); err != nil {
		t.Fatal(err)
	}
	ref := func(m Message) string { return search.Ref(search.KindMsg, m.ID) }

	tests := []struct {
		name string
		m    Message
		want bool
	}{
		{"never held", plain, true},
		{"still held", held, false},
		{"dismissed while held", dropped, false},
	}
	got := h.docs(time.Time{})
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			_, in := got[ref(tc.m)]
			_, err := h.ib.SearchShow(bg, ref(tc.m), search.ShowOptions{})
			if in != tc.want || (err == nil) != tc.want {
				t.Fatalf("in docs %v, show err %v; want visible %v", in, err, tc.want)
			}
			if !tc.want && !errors.Is(err, search.ErrNotFound) {
				t.Fatalf("err %v", err)
			}
		})
	}
	if d := got[ref(plain)]; !d.Updated.Equal(plain.SentAt) || !d.At.Equal(t0) || d.Agent != keyA || d.Body != "parser plan" || d.Title != "alpha -> beta" {
		t.Fatalf("doc %+v", d)
	}

	// Released: found from then on, with Updated = the release time.
	h.clk.advance(time.Hour)
	rel := h.clk.now()
	if err := h.ib.Release(bg, hd.ID, user); err != nil {
		t.Fatal(err)
	}
	later := h.docs(rel)
	d, ok := later[ref(held)]
	if !ok || !d.Updated.Equal(rel) || !d.At.Equal(held.SentAt) || len(later) != 1 {
		t.Fatalf("after release %+v", later)
	}
	// Survives replay.
	h.reopen()
	if again := h.docs(rel); len(again) != 1 || !again[ref(held)].Updated.Equal(rel) {
		t.Fatalf("after replay %+v", again)
	}
	if _, ok := h.docs(time.Time{})[ref(dropped)]; ok {
		t.Fatal("dismissed message found after replay")
	}
}

func TestSearchDocFields(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ib.SetReachable(keyA, PathMod, true)
	const keyD = "dev1/dddddddd-4444-4444-8444-444444444444" // the PR repo's desk
	h.ros.put(card(keyD, "delta", "remote:github.com/acme/app", BypassMode))
	h.ib.SetReachable(keyD, PathMod, true)
	tests := []struct {
		name               string
		from               Sender
		addr               Address
		reason             string
		title, agent, repo string
	}{
		{"agent to agent", claude(keyB, "beta"), to("gamma"), "", "beta -> gamma", keyB, ""},
		{"with a reason", claude(keyB, "beta"), to("gamma"), "review", "beta -> gamma: review", keyB, ""},
		{"to a repo", claude(keyC, "gamma"), toRepo("repo1"), "", "gamma -> repo repo1", keyC, "repo1"},
		{"a person: the recipient's agent", person(), to("gamma"), "", "Stephen -> gamma", keyC, ""},
		{"Irori: the recipient's agent", SystemSender(), to("gamma"), "", "Irori -> gamma", keyC, ""},
		{"to a PR", claude(keyB, "beta"), Address{Kind: ToPR, Value: "acme/app#7"}, "", "beta -> pr:acme/app#7", keyB, "remote:github.com/acme/app"},
	}
	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			m, _, err := h.ib.SendWith(bg, tc.from, tc.addr, "body "+tc.name, SendOptions{Reason: tc.reason})
			if err != nil {
				t.Fatal(err)
			}
			d, ok := h.docs(time.Time{})[search.Ref(search.KindMsg, m.ID)]
			if !ok {
				t.Fatal("not found")
			}
			if d.Kind != search.KindMsg || d.Title != tc.title || d.Agent != tc.agent || d.RepoKey != tc.repo || d.Body != "body "+tc.name {
				t.Fatalf("doc %+v", d)
			}
		})
	}
	if ks := h.ib.SearchKinds(); !slices.Equal(ks, []search.Kind{search.KindMsg}) {
		t.Fatal(ks)
	}
	ds, _ := h.ib.SearchDocs(bg, time.Time{}, 2)
	if len(ds) != 2 {
		t.Fatalf("limit: %d docs", len(ds))
	}
}

func TestSearchShow(t *testing.T) {
	h := newHarness(t)
	h.abc()
	h.ros.put(grokCard(keyG, "grok", ""))
	orig := h.sendWith(claude(keyA, "alpha"), to("beta"), "can you take the lexer?", SendOptions{Ref: "task:01LEX", Reason: "hand-off"})
	_, r1, err := h.ib.Reply(bg, claude(keyB, "beta"), orig.ID, "yes")
	if err != nil {
		t.Fatal(err)
	}
	// A held reply is not offered as related.
	h.wake(keyA)
	if _, _, err := h.ib.Send(bg, grok(keyG, "grok"), Address{Kind: ToReply, Value: orig.MessageID}, "me too", ""); err != nil {
		t.Fatal(err)
	}

	ref := search.Ref(search.KindMsg, orig.MessageID)
	s, err := h.ib.SearchShow(bg, "  MSG:"+orig.MessageID+" ", search.ShowOptions{})
	if err != nil {
		t.Fatal(err)
	}
	if s.Ref != ref || s.Kind != search.KindMsg || s.Title != "alpha -> beta: hand-off" || s.More {
		t.Fatalf("shown %+v", s)
	}
	for _, want := range []string{"from: alpha (" + keyA + ") trust=untrusted", "about: task:01LEX", orig.ID + " to beta (" + keyB + "): answered", "can you take the lexer?"} {
		if !strings.Contains(s.Text, want) {
			t.Fatalf("text lacks %q:\n%s", want, s.Text)
		}
	}
	if want := []string{"task:01LEX", search.Ref(search.KindMsg, r1.MessageID)}; !slices.Equal(s.Related, want) {
		t.Fatalf("related %v, want %v", s.Related, want)
	}
	back, err := h.ib.SearchShow(bg, search.Ref(search.KindMsg, r1.MessageID), search.ShowOptions{})
	if err != nil || !slices.Equal(back.Related, []string{ref}) {
		t.Fatalf("reply related %v %v", back.Related, err)
	}

	// Paging.
	page, _ := h.ib.SearchShow(bg, ref, search.ShowOptions{Limit: 20})
	if !page.More || page.Next != len(page.Text) {
		t.Fatalf("page %+v", page)
	}
	rest, _ := h.ib.SearchShow(bg, ref, search.ShowOptions{Offset: page.Next})
	if !strings.HasSuffix(rest.Text, "can you take the lexer?\n") || page.Text+rest.Text != s.Text {
		t.Fatal("pages don't add up")
	}

	errs := []struct {
		ref  string
		want error
	}{
		{"msg:msg_nope", search.ErrNotFound},
		{"task:01LEX", search.ErrNotFound},
		{"nonsense", search.ErrBadRef},
	}
	for _, tc := range errs {
		if _, err := h.ib.SearchShow(bg, tc.ref, search.ShowOptions{}); !errors.Is(err, tc.want) {
			t.Errorf("%s: %v, want %v", tc.ref, err, tc.want)
		}
	}
}
