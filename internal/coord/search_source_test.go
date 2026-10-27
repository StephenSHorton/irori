package coord

import (
	"errors"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/search"
)

func TestSearchDocsTasks(t *testing.T) {
	e := newEnv(t)
	t1 := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "Paint the shed", Objective: "two coats", DoneCriteria: "dry", RepoKey: repoA}))
	e.clk.add(time.Minute)
	t2 := must[Task](t)(e.c.CreateTask(ctx, bob, NewTask{Title: "Oil the gate"}))
	e.clk.add(time.Minute)
	must[Task](t)(e.c.ClaimTask(ctx, alice, t1.ID)) // t1 changes after t2

	docs := must[[]search.Doc](t)(e.c.SearchDocs(ctx, time.Time{}, 10))
	if got := docRefs(docs); !slices.Equal(got, []string{"task:" + t2.ID, "task:" + t1.ID}) {
		t.Fatalf("oldest Updated first: %v", got)
	}
	d := docs[1]
	if d.Kind != search.KindTask || d.Title != "Paint the shed" || d.Agent != alice || d.RepoKey != repoA ||
		d.Body != "two coats\ndry" || !d.At.Equal(fixedNow) || !d.Updated.Equal(fixedNow.Add(2*time.Minute)) {
		t.Fatalf("doc %+v", d)
	}

	tests := []struct {
		name  string
		since time.Time
		limit int
		want  []string
	}{
		{"since is inclusive", fixedNow.Add(time.Minute), 10, []string{"task:" + t2.ID, "task:" + t1.ID}},
		{"since skips older", fixedNow.Add(90 * time.Second), 10, []string{"task:" + t1.ID}},
		{"limit", time.Time{}, 1, []string{"task:" + t2.ID}},
		{"zero limit", time.Time{}, 0, nil},
		{"future", fixedNow.Add(time.Hour), 10, nil},
	}
	for _, tt := range tests {
		docs := must[[]search.Doc](t)(e.c.SearchDocs(ctx, tt.since, tt.limit))
		if got := docRefs(docs); !slices.Equal(got, tt.want) {
			t.Errorf("%s: %v, want %v", tt.name, got, tt.want)
		}
	}
}

func TestCommentDoc(t *testing.T) {
	task := Task{ID: "T1", Title: "Paint the shed", RepoKey: repoA}
	tests := []struct {
		by, agent string
	}{
		{alice, alice},
		{"user:dev-a", ""},
	}
	for _, tt := range tests {
		d := commentDoc(Comment{ID: "C1", TaskID: "T1", By: tt.by, Text: "use blue", At: fixedNow}, task)
		if d.Ref != "comment:C1" || d.Kind != search.KindComment || d.Title != "comment on Paint the shed" ||
			d.Body != "use blue" || d.Agent != tt.agent || d.RepoKey != repoA || !d.Updated.Equal(fixedNow) {
			t.Errorf("by %s: %+v", tt.by, d)
		}
	}
}

func TestSearchShowTask(t *testing.T) {
	e := newEnv(t)
	task := must[Task](t)(e.c.CreateTask(ctx, alice, NewTask{Title: "Paint the shed", Objective: "two coats", RepoKey: repoA}))
	sh := must[search.Shown](t)(e.c.SearchShow(ctx, "task:"+task.ID, search.ShowOptions{}))
	if sh.Ref != "task:"+task.ID || sh.Kind != search.KindTask || sh.Title != "Paint the shed" || sh.More {
		t.Fatalf("shown %+v", sh)
	}
	for _, s := range []string{"state: submitted", "holders: nobody", "objective:\ntwo coats", "repo: " + repoA} {
		if !strings.Contains(sh.Text, s) {
			t.Errorf("text lacks %q:\n%s", s, sh.Text)
		}
	}
	page := must[search.Shown](t)(e.c.SearchShow(ctx, "task:"+task.ID, search.ShowOptions{Limit: 10}))
	if !page.More || page.Next != len(page.Text) || len(page.Text) > 10 {
		t.Fatalf("page %+v", page)
	}

	errTests := []struct {
		ref  string
		want error
	}{
		{"task:nope", search.ErrNotFound},
		{"comment:nope", search.ErrNotFound},
		{"msg:x", search.ErrNotFound},
		{"nope", search.ErrBadRef},
	}
	for _, tt := range errTests {
		if _, err := e.c.SearchShow(ctx, tt.ref, search.ShowOptions{}); !errors.Is(err, tt.want) {
			t.Errorf("%s: %v, want %v", tt.ref, err, tt.want)
		}
	}
}

func docRefs(docs []search.Doc) []string {
	var out []string
	for _, d := range docs {
		out = append(out, d.Ref)
	}
	return out
}
