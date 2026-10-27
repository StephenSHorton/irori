package ledger

import (
	"fmt"
	"reflect"
	"sync"
	"testing"
	"time"
)

const pr = "pr:acme/webapp#1437"

func at(s string) time.Time {
	t, err := time.Parse(time.RFC3339, s)
	if err != nil {
		panic(err)
	}
	return t
}

func entry(k, session string, role Role, when string) Entry {
	return Entry{Key: k, Repo: "acme/webapp", Number: 1437, Session: session, Role: role, Source: "gh", At: at(when)}
}

func sessions(es []Entry) []string {
	out := []string{}
	for _, e := range es {
		out = append(out, e.Session)
	}
	return out
}

// Ported: "owners rank who made a thing over who touched or reviewed it, then earliest".
func TestOwnersRank(t *testing.T) {
	l := New()
	now := at("2026-10-09T13:00:00Z")
	l.Add(entry(pr, "dev/reviewer", Reviewer, "2026-10-09T10:00:00Z"))
	l.Add(entry(pr, "dev/late-toucher", Touched, "2026-10-09T12:00:00Z"))
	l.Add(entry(pr, "dev/early-toucher", Touched, "2026-10-09T11:00:00Z"))
	want := []string{"dev/early-toucher", "dev/late-toucher", "dev/reviewer"}
	if got := sessions(l.Owners(pr, now)); !reflect.DeepEqual(got, want) {
		t.Fatalf("owners = %v, want %v", got, want)
	}
	l.Add(entry(pr, "dev/creator", Author, "2026-10-09T12:30:00Z"))
	if got := l.Owners(pr, now); got[0].Session != "dev/creator" {
		t.Fatalf("first owner = %q, want dev/creator", got[0].Session)
	}
}

func TestOwnersTieBreakOnSession(t *testing.T) {
	l := New()
	when := "2026-10-09T10:00:00Z"
	l.Add(entry(pr, "dev/b", Touched, when))
	l.Add(entry(pr, "dev/c", Touched, when))
	l.Add(entry(pr, "dev/a", Touched, when))
	want := []string{"dev/a", "dev/b", "dev/c"}
	if got := sessions(l.Owners(pr, at(when))); !reflect.DeepEqual(got, want) {
		t.Fatalf("owners = %v, want %v", got, want)
	}
}

func TestAdd(t *testing.T) {
	base := entry(pr, "dev/s", Touched, "2026-10-09T12:00:00Z")
	with := func(f func(*Entry)) Entry { e := base; f(&e); return e }
	tests := []struct {
		name    string
		first   *Entry
		add     Entry
		changed bool
		wantAt  string
		wantURL string
		count   int
	}{
		{name: "new identity", add: base, changed: true, wantAt: "2026-10-09T12:00:00Z", count: 1},
		{name: "empty key ignored", add: with(func(e *Entry) { e.Key = "" }), changed: false, count: 0},
		{name: "empty session ignored", add: with(func(e *Entry) { e.Session = "" }), changed: false, count: 0},
		{name: "unknown role ignored", add: with(func(e *Entry) { e.Role = "poster" }), changed: false, count: 0},
		{name: "empty role ignored", add: with(func(e *Entry) { e.Role = "" }), changed: false, count: 0},
		{name: "same again unchanged", first: &base, add: base, changed: false, wantAt: "2026-10-09T12:00:00Z", count: 1},
		{
			name: "later at keeps earliest", first: &base,
			add:     with(func(e *Entry) { e.At = at("2026-10-09T12:05:00Z") }),
			changed: false, wantAt: "2026-10-09T12:00:00Z", count: 1,
		},
		{
			name: "earlier at wins", first: &base,
			add:     with(func(e *Entry) { e.At = at("2026-10-09T11:00:00Z") }),
			changed: true, wantAt: "2026-10-09T11:00:00Z", count: 1,
		},
		{
			name: "zero at does not win", first: &base,
			add:     with(func(e *Entry) { e.At = time.Time{} }),
			changed: false, wantAt: "2026-10-09T12:00:00Z", count: 1,
		},
		{
			name: "first non-empty url is taken", first: &base,
			add:     with(func(e *Entry) { e.URL = "https://github.com/acme/webapp/pull/1437" }),
			changed: true, wantAt: "2026-10-09T12:00:00Z", wantURL: "https://github.com/acme/webapp/pull/1437", count: 1,
		},
		{
			name:    "a later url does not replace the first",
			first:   ptr(with(func(e *Entry) { e.URL = "https://github.com/acme/webapp/pull/1437" })),
			add:     with(func(e *Entry) { e.URL = "https://github.com/acme/webapp/pull/1437/files" }),
			changed: false, wantAt: "2026-10-09T12:00:00Z", wantURL: "https://github.com/acme/webapp/pull/1437", count: 1,
		},
		{
			name: "same session other role is a new identity", first: &base,
			add:     with(func(e *Entry) { e.Role = Reviewer }),
			changed: true, wantAt: "2026-10-09T12:00:00Z", count: 2,
		},
		{
			name: "same pr other session is a new identity", first: &base,
			add:     with(func(e *Entry) { e.Session = "dev/other" }),
			changed: true, wantAt: "2026-10-09T12:00:00Z", count: 2,
		},
	}
	now := at("2026-10-09T13:00:00Z")
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			l := New()
			if tt.first != nil {
				l.Add(*tt.first)
			}
			if got := l.Add(tt.add); got != tt.changed {
				t.Fatalf("Add = %v, want %v", got, tt.changed)
			}
			got := l.Owners(pr, now)
			if len(got) != tt.count {
				t.Fatalf("owners = %d, want %d: %+v", len(got), tt.count, got)
			}
			if tt.count == 0 {
				return
			}
			for _, e := range got {
				if e.Session == base.Session && e.Role == base.Role || tt.first == nil {
					if !e.At.Equal(at(tt.wantAt)) {
						t.Errorf("at = %v, want %v", e.At, tt.wantAt)
					}
					if e.URL != tt.wantURL {
						t.Errorf("url = %q, want %q", e.URL, tt.wantURL)
					}
				}
			}
		})
	}
}

func ptr(e Entry) *Entry { return &e }

// Ported: "recording the same thing again keeps the earliest time" (with URL
// standing in for tsukumo's label).
func TestAddKeepsEarliestAndFirstURL(t *testing.T) {
	l := New()
	now := at("2026-10-09T13:00:00Z")
	l.Add(entry(pr, "dev/s", Author, "2026-10-09T12:00:00Z"))
	second := entry(pr, "dev/s", Author, "2026-10-09T12:05:00Z")
	second.URL = "https://github.com/acme/webapp/pull/1437"
	second.Source = "gitOperation"
	if !l.Add(second) {
		t.Fatal("adding a url should change the view")
	}
	want := entry(pr, "dev/s", Author, "2026-10-09T12:00:00Z")
	want.URL = second.URL
	if got := l.Owners(pr, now); !reflect.DeepEqual(got, []Entry{want}) {
		t.Fatalf("owners = %+v, want %+v", got, []Entry{want})
	}
}

// Ported: "entries older than 45 days are dropped".
func TestKeep(t *testing.T) {
	now := at("2026-10-09T00:00:00Z")
	tests := []struct {
		name string
		at   time.Time
		keep bool
	}{
		{"old", at("2026-08-01T00:00:00Z"), false},
		{"just past keep", now.Add(-Keep - time.Nanosecond), false},
		{"exactly keep", now.Add(-Keep), true},
		{"recent", now.Add(-time.Hour), true},
		{"future", now.Add(time.Hour), true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			l := New()
			e := entry(pr, "dev/s", Touched, "2026-10-09T00:00:00Z")
			e.At = tt.at
			l.Add(e)
			if got := len(l.Owners(pr, now)) == 1; got != tt.keep {
				t.Errorf("owners kept = %v, want %v", got, tt.keep)
			}
			if got := len(l.ForRepo("", now)) == 1; got != tt.keep {
				t.Errorf("ForRepo kept = %v, want %v", got, tt.keep)
			}
			if got := len(l.ForSession("dev/s", now)) == 1; got != tt.keep {
				t.Errorf("ForSession kept = %v, want %v", got, tt.keep)
			}
			// The row stays: it shows again for an earlier now.
			if len(l.Owners(pr, tt.at)) != 1 {
				t.Error("entry was dropped from the store")
			}
		})
	}
}

func prEntry(repo string, n int, session string, role Role, when string) Entry {
	return Entry{Key: Key(repo, n), Repo: repo, Number: n, Session: session, Role: role, Source: "pr-link", At: at(when)}
}

func keyRoleSessions(es []Entry) []string {
	out := []string{}
	for _, e := range es {
		out = append(out, fmt.Sprintf("%s %s %s", e.Key, e.Role, e.Session))
	}
	return out
}

func TestForRepo(t *testing.T) {
	l := New()
	now := at("2026-10-09T13:00:00Z")
	l.Add(prEntry("acme/webapp", 12, "dev/a", Reviewer, "2026-10-09T10:00:00Z"))
	l.Add(prEntry("acme/webapp", 12, "dev/b", Author, "2026-10-09T11:00:00Z"))
	l.Add(prEntry("Acme/WebApp", 40, "dev/c", Touched, "2026-10-09T09:00:00Z"))
	l.Add(prEntry("acme/storefront", 12, "dev/d", Author, "2026-10-09T09:00:00Z"))
	l.Add(prEntry("acme/webapp", 3, "dev/old", Author, "2026-07-01T00:00:00Z"))

	tests := []struct {
		repo string
		want []string
	}{
		{"acme/webapp", []string{
			"pr:acme/webapp#40 touched dev/c",
			"pr:acme/webapp#12 author dev/b",
			"pr:acme/webapp#12 reviewer dev/a",
		}},
		{"ACME/WEBAPP", []string{
			"pr:acme/webapp#40 touched dev/c",
			"pr:acme/webapp#12 author dev/b",
			"pr:acme/webapp#12 reviewer dev/a",
		}},
		{"", []string{
			"pr:acme/webapp#40 touched dev/c",
			"pr:acme/storefront#12 author dev/d",
			"pr:acme/webapp#12 author dev/b",
			"pr:acme/webapp#12 reviewer dev/a",
		}},
		{"acme/none", []string{}},
	}
	for _, tt := range tests {
		if got := keyRoleSessions(l.ForRepo(tt.repo, now)); !reflect.DeepEqual(got, tt.want) {
			t.Errorf("ForRepo(%q) = %v, want %v", tt.repo, got, tt.want)
		}
	}
}

func TestForSession(t *testing.T) {
	l := New()
	now := at("2026-10-09T13:00:00Z")
	l.Add(prEntry("acme/webapp", 12, "dev/a", Touched, "2026-10-09T10:00:00Z"))
	l.Add(prEntry("acme/webapp", 13, "dev/a", Author, "2026-10-09T12:00:00Z"))
	l.Add(prEntry("acme/webapp", 12, "dev/a", Reviewer, "2026-10-09T10:00:00Z"))
	l.Add(prEntry("acme/webapp", 11, "dev/a", Author, "2026-10-09T10:00:00Z"))
	l.Add(prEntry("acme/webapp", 14, "dev/b", Author, "2026-10-09T12:30:00Z"))
	l.Add(prEntry("acme/webapp", 2, "dev/a", Author, "2026-06-01T00:00:00Z"))
	want := []string{
		"pr:acme/webapp#13 author dev/a",
		"pr:acme/webapp#11 author dev/a",
		"pr:acme/webapp#12 touched dev/a",
		"pr:acme/webapp#12 reviewer dev/a",
	}
	if got := keyRoleSessions(l.ForSession("dev/a", now)); !reflect.DeepEqual(got, want) {
		t.Errorf("ForSession = %v, want %v", got, want)
	}
	if got := l.ForSession("dev/none", now); len(got) != 0 {
		t.Errorf("ForSession(none) = %v", got)
	}
}

func TestViewsAreCopies(t *testing.T) {
	l := New()
	now := at("2026-10-09T13:00:00Z")
	l.Add(entry(pr, "dev/s", Touched, "2026-10-09T12:00:00Z"))
	got := l.Owners(pr, now)
	got[0].Session = "mutated"
	got[0].URL = "x"
	if again := l.Owners(pr, now); again[0].Session != "dev/s" || again[0].URL != "" {
		t.Fatalf("view mutation leaked: %+v", again[0])
	}
}

func TestConcurrent(t *testing.T) {
	l := New()
	now := at("2026-10-09T13:00:00Z")
	var wg sync.WaitGroup
	for g := 0; g < 8; g++ {
		wg.Add(1)
		go func(g int) {
			defer wg.Done()
			for i := 0; i < 200; i++ {
				l.Add(prEntry("acme/webapp", i%10+1, fmt.Sprintf("dev/s%d", g), Touched, "2026-10-09T12:00:00Z"))
				l.Owners(Key("acme/webapp", i%10+1), now)
				l.ForRepo("acme/webapp", now)
				l.ForSession(fmt.Sprintf("dev/s%d", g), now)
			}
		}(g)
	}
	wg.Wait()
	if got := len(l.ForRepo("", now)); got != 80 {
		t.Fatalf("entries = %d, want 80", got)
	}
}
