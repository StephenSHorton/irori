package gitlink

import (
	"context"
	"strings"
	"testing"
)

func TestIsCommitCommand(t *testing.T) {
	tests := []struct {
		cmd  string
		want bool
	}{
		// Each commit-making subcommand.
		{`git commit -m "fix"`, true},
		{`git merge --ff-only feat`, true},
		{`git cherry-pick abc123`, true},
		{`git revert HEAD`, true},
		{`git rebase main`, true},
		{`git am < patch.mbox`, true},
		{`git pull --rebase`, true},
		{`gh pr merge 12 --squash`, true},
		{`gh pr merge --squash --delete-branch`, true},
		{`gh -R owner/repo pr merge 12`, true},
		{`gh pr view 12`, false},
		{`gh pr create --fill`, false},

		// Global options before the subcommand.
		{`git -C C:\work\repo commit -m x`, true},
		{`git -C "C:\Program Files\x" commit -m x`, true},
		{`git -c user.name=x -c core.autocrlf=false commit -am x`, true},
		{`git --no-pager --git-dir=.git --work-tree . commit -m x`, true},
		{`git --git-dir .git log`, false},
		{`"C:\Program Files\Git\cmd\git.exe" commit -m x`, true},
		{`/usr/bin/git commit -m x`, true},
		{`GIT.EXE commit -m x`, true},

		// Compound commands.
		{`cd repo && git add -A && git commit -m "x"`, true},
		{"go test ./...; git commit -m x", true},
		{"git status\ngit commit -m x", true},
		{"git status\r\ngit commit -m x\r\n", true},
		{`make || git commit -m x`, true},
		{`git commit -m x 2>&1 | tail -5`, true},
		{`(cd sub; git commit -m x)`, true},
		{`{ git commit -m x; }`, true},
		{`if true; then git merge feat; fi`, true},
		{`echo $(git commit -m x)`, true},
		{"echo `git commit -m x`", true},
		{`GIT_AUTHOR_DATE=now git commit -m x`, true},
		{`env FOO=1 git commit -m x`, true},
		{`sudo git pull`, true},
		{"git commit \\\n  -m x", true},
		{`bash -c "git commit -m x"`, true},
		{`bash -lc 'cd r && git commit -m x'`, true},
		{`eval git commit -m x`, true},

		// Not commit commands.
		{`git log --oneline`, false},
		{`git status`, false},
		{`git diff HEAD`, false},
		{`git merge-base main feat`, false},
		{`git commit-tree HEAD^{tree} -m x`, false}, // plumbing: no HEAD move by itself
		{`git rebase --abort`, false},
		{`git merge --abort`, false},
		{`git cherry-pick --quit`, false},
		{`git commit --help`, false},
		{`git`, false},
		{`git -C`, false},
		{`echo "git commit -m x"`, false},
		{`echo git commit`, false},
		{`grep -r 'git commit' .`, false},
		{`# git commit -m x`, false},
		{`ls # && git commit`, false},
		{`mygit commit`, false},
		{`gitk commit`, false},
		{`git log --grep="commit"`, false},
		{``, false},
		{`   `, false},

		// Accepted false positives: text that reads like a command line.
		{"cat <<EOF\ngit commit -m x\nEOF", true},
	}
	for _, tt := range tests {
		if got := IsCommitCommand(tt.cmd); got != tt.want {
			t.Errorf("IsCommitCommand(%q) = %v, want %v", tt.cmd, got, tt.want)
		}
	}
}

func TestSplitCommands(t *testing.T) {
	tests := []struct {
		in   string
		want string // commands joined by " | ", words by ","
	}{
		{`a b && c`, "a,b | c"},
		{`a "b c" 'd e'`, "a,b c,d e"},
		{`a "x \"y\" \$z"`, `a,x "y" $z`},
		{`a\ b c`, "a b,c"},
		{`a "" b`, "a,,b"},
		{`a 'unterminated`, "a,unterminated"},
		{`a#b c`, "a#b,c"},
		{"a\n\n;;b", "a | b"},
	}
	for _, tt := range tests {
		var cmds []string
		for _, w := range splitCommands(tt.in) {
			cmds = append(cmds, strings.Join(w, ","))
		}
		if got := strings.Join(cmds, " | "); got != tt.want {
			t.Errorf("splitCommands(%q) = %q, want %q", tt.in, got, tt.want)
		}
	}
}

func TestExec(t *testing.T) {
	r := newRepo(t, "false")
	r.write("a.txt", "a\n")
	sha := r.commit("one", 1700000000)

	// A GIT_DIR inherited from a hook must not redirect the run.
	t.Setenv("GIT_DIR", t.TempDir())
	out, err := Exec.Run(context.Background(), r.root, "rev-parse", "HEAD")
	if err != nil || out != sha+"\n" {
		t.Fatalf("Run = %q, %v (stdout should come back untrimmed)", out, err)
	}

	_, err = Exec.Run(context.Background(), r.root, "rev-parse", "--verify", "nope^{commit}")
	if err == nil || !strings.Contains(err.Error(), "rev-parse") {
		t.Fatalf("error %v", err)
	}

	c, cancel := context.WithCancel(context.Background())
	cancel()
	if _, err := Exec.Run(c, r.root, "status"); err == nil {
		t.Fatal("canceled context: want error")
	}
}

func TestGitEnv(t *testing.T) {
	env := []string{"PATH=x", "GIT_DIR=/r", "GIT_WORK_TREE=/w", "GIT_AUTHOR_NAME=a", "HOME=h"}
	got := strings.Join(gitEnv(env), " ")
	if got != "PATH=x GIT_AUTHOR_NAME=a HOME=h" {
		t.Fatalf("gitEnv = %s", got)
	}
}
