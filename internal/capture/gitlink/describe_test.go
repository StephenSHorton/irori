package gitlink

import (
	"context"
	"strconv"
	"strings"
	"testing"
	"time"
	"unicode/utf8"
)

func filesString(fs []FileChange) string {
	var parts []string
	for _, f := range fs {
		parts = append(parts, f.Status+" "+f.Path+" "+f.Blob)
	}
	return strings.Join(parts, "\n")
}

func TestDescribe(t *testing.T) {
	r := newRepo(t, "false")
	c := ctx(t)
	desc := func(sha string) Commit {
		t.Helper()
		got, err := Describe(c, Exec, r.root, sha)
		if err != nil {
			t.Fatalf("Describe(%s): %v", sha, err)
		}
		if got.SHA != sha {
			t.Fatalf("SHA %s, want %s", got.SHA, sha)
		}
		return got
	}

	// Root commit, with a non-ASCII path in a subdirectory and one with a
	// space.
	uni := "docs/café ü/naïve 日本.txt"
	r.write("a.txt", "a\n")
	r.write(uni, "uni\n")
	r.write("with space.txt", "s\n")
	root := r.commit("root commit", 1700000000)
	got := desc(root)
	if len(got.Parents) != 0 || !got.At.Equal(time.Unix(1700000000, 0)) ||
		got.Author != "Test Author" || got.Subject != "root commit" {
		t.Fatalf("root commit %+v", got)
	}
	want := "A a.txt " + r.blob(root, "a.txt") + "\n" +
		"A " + uni + " " + r.blob(root, uni) + "\n" +
		"A with space.txt " + r.blob(root, "with space.txt")
	if s := filesString(got.Files); s != want {
		t.Fatalf("root files:\n%s\nwant:\n%s", s, want)
	}

	// Modify and delete.
	r.write("a.txt", "a2\n")
	r.git("rm", "-q", "with space.txt")
	r.write(uni, "uni2\n")
	second := r.commit("second", 1700000050)
	got = desc(second)
	if strings.Join(got.Parents, ",") != root {
		t.Fatalf("parents %v", got.Parents)
	}
	want = "M a.txt " + r.blob(second, "a.txt") + "\n" +
		"M " + uni + " " + r.blob(second, uni) + "\n" +
		"D with space.txt " + zero40
	if s := filesString(got.Files); s != want {
		t.Fatalf("second files:\n%s\nwant:\n%s", s, want)
	}

	// A merge commit lists the diff against its first parent only.
	r.git("checkout", "-q", "-b", "side")
	r.write("side.txt", "side\n")
	sideTip := r.commit("side work", 1700000100)
	r.git("checkout", "-q", "main")
	r.write("main.txt", "main\n")
	mainTip := r.commit("main work", 1700000150)
	date := "1700000200 +0000"
	gitIn(t, r.root, []string{"GIT_AUTHOR_DATE=" + date, "GIT_COMMITTER_DATE=" + date}, "",
		"merge", "-q", "--no-ff", "-m", "Merge side", "side")
	merge := r.git("rev-parse", "HEAD")
	got = desc(merge)
	if strings.Join(got.Parents, ",") != mainTip+","+sideTip {
		t.Fatalf("merge parents %v", got.Parents)
	}
	if s := filesString(got.Files); s != "A side.txt "+r.blob(merge, "side.txt") {
		t.Fatalf("merge files:\n%s", s)
	}
	if !got.At.Equal(time.Unix(1700000200, 0)) || got.Subject != "Merge side" {
		t.Fatalf("merge commit %+v", got)
	}

	// Subject capped at 200 runes.
	long := strings.Repeat("é", 250)
	r.write("a.txt", "a3\n")
	got = desc(r.commit(long, 1700000300))
	if utf8.RuneCountInString(got.Subject) != 200 || !strings.HasPrefix(long, got.Subject) {
		t.Fatalf("subject has %d runes", utf8.RuneCountInString(got.Subject))
	}

	// An empty commit has no files, as an empty list.
	got = desc(r.commit("empty", 1700000400))
	if got.Files == nil || len(got.Files) != 0 {
		t.Fatalf("empty commit files %#v", got.Files)
	}

	// Errors.
	if _, err := Describe(c, Exec, r.root, "HEAD"); err == nil {
		t.Fatal("Describe(HEAD): want error for a non-sha")
	}
	if _, err := Describe(c, Exec, r.root, shaA); err == nil {
		t.Fatal("Describe(missing): want error")
	}
}

func TestDescribeMaxFiles(t *testing.T) {
	r := newRepo(t, "false")
	for i := 0; i < MaxFiles+5; i++ {
		r.write("f/"+strconv.Itoa(10000+i)+".txt", strconv.Itoa(i)+"\n")
	}
	sha := r.commit("many", 1700000000)
	got, err := Describe(ctx(t), Exec, r.root, sha)
	if err != nil {
		t.Fatal(err)
	}
	if len(got.Files) != MaxFiles || got.FilesOmitted != 5 {
		t.Fatalf("%d files, %d omitted", len(got.Files), got.FilesOmitted)
	}
}

func TestParseRawZ(t *testing.T) {
	blob := "ce013625030ba8dba906f756967f9e9ca394464a"
	tests := []struct {
		name    string
		in      string
		want    string
		wantErr bool
	}{
		{name: "empty", in: ""},
		{name: "add", in: ":000000 100644 " + zero40 + " " + blob + " A\x00a.txt\x00", want: "A a.txt " + blob},
		{name: "type change", in: ":100644 120000 " + blob + " " + blob + " T\x00l\x00", want: "T l " + blob},
		{name: "scored status keeps the letter", in: ":100644 100644 " + blob + " " + blob + " M100\x00a\x00", want: "M a " + blob},
		{name: "path with newline", in: ":100644 100644 " + blob + " " + blob + " M\x00a\nb\x00", want: "M a\nb " + blob},
		{name: "no path", in: ":100644 100644 " + blob + " " + blob + " M", wantErr: true},
		{name: "not raw", in: "diff --git a b\x00", wantErr: true},
		{name: "short meta", in: ":100644 " + blob + " M\x00a\x00", wantErr: true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := parseRawZ(tt.in)
			if (err != nil) != tt.wantErr {
				t.Fatalf("err %v", err)
			}
			if err == nil && filesString(got) != tt.want {
				t.Fatalf("got %q", filesString(got))
			}
		})
	}
}

func TestDescribeFakeErrors(t *testing.T) {
	sha := shaA
	show := "-c log.showSignature=false show -s --no-color --format=%H%x00%P%x00%ct%x00%an%x00%s " + sha + " --"
	tests := []struct {
		name string
		out  string
	}{
		{"too few fields", sha + "\x00\x00123\n"},
		{"bad time", sha + "\x00\x00soon\x00A\x00s\n"},
		{"bad sha", "nope\x00\x00123\x00A\x00s\n"},
	}
	for _, tt := range tests {
		g := &recordGit{answers: map[string]string{show: tt.out}}
		if _, err := Describe(context.Background(), g, "r", sha); err == nil {
			t.Errorf("%s: want error", tt.name)
		}
	}
	// diff-tree failing is an error too.
	g := &recordGit{answers: map[string]string{show: sha + "\x00\x00123\x00A\x00s\n"}}
	if _, err := Describe(context.Background(), g, "r", sha); err == nil {
		t.Error("diff-tree failure: want error")
	}
	if len(g.calls) != 2 || !strings.Contains(g.calls[1], "--root") {
		t.Errorf("root commit diff-tree should pass --root: %v", g.calls)
	}
}
