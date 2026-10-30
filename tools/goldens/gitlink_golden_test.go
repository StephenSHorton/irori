// Golden exporter for crates/capture's gitlink module (capture.md §2.5,
// §3 "Linking", "Committer", "IsCommitCommand").
//
// Injected into I:internal/capture/gitlink by tools/goldens/export.sh with
// `go test -overlay`; runs only when IRORI_GOLDEN_OUT is set and writes
// $IRORI_GOLDEN_OUT/capture/gitlink.json: IsCommitCommand and
// splitCommands over the Go tables plus extras, ParseReflog chunks,
// parseRawZ outputs, underRoot pairs, and Classify and Committer over
// generated candidate sets (fixed seed).
package gitlink

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math/rand/v2"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestGoldenGitlink(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	v := map[string]any{}

	cmds := []string{
		`git commit -m "fix"`, `git merge --ff-only feat`, `git cherry-pick abc123`, `git revert HEAD`,
		`git rebase main`, `git am < patch.mbox`, `git pull --rebase`, `gh pr merge 12 --squash`,
		`gh pr merge --squash --delete-branch`, `gh -R owner/repo pr merge 12`, `gh pr view 12`, `gh pr create --fill`,
		`git -C C:\work\repo commit -m x`, `git -C "C:\Program Files\x" commit -m x`,
		`git -c user.name=x -c core.autocrlf=false commit -am x`, `git --no-pager --git-dir=.git --work-tree . commit -m x`,
		`git --git-dir .git log`, `"C:\Program Files\Git\cmd\git.exe" commit -m x`, `/usr/bin/git commit -m x`,
		`GIT.EXE commit -m x`, `cd repo && git add -A && git commit -m "x"`, "go test ./...; git commit -m x",
		"git status\ngit commit -m x", "git status\r\ngit commit -m x\r\n", `make || git commit -m x`,
		`git commit -m x 2>&1 | tail -5`, `(cd sub; git commit -m x)`, `{ git commit -m x; }`,
		`if true; then git merge feat; fi`, `echo $(git commit -m x)`, "echo `git commit -m x`",
		`GIT_AUTHOR_DATE=now git commit -m x`, `env FOO=1 git commit -m x`, `env -u FOO git commit -m x`,
		`sudo -u root git commit -m x`, `sudo -E -u root -- git commit -m x`, `timeout 60 git commit -m x`,
		`timeout -s KILL -k 5 60s git commit -m x`, `timeout --signal=KILL 60 git pull`, `nice -n 5 git commit -m x`,
		`nice git commit -m x`, `timeout 60 git status`, `git commit -m --abort`, `git commit -m "--quit" -F -h`,
		`git commit -- --help`, `git rebase -q --abort`, `sudo git pull`, "git commit \\\n  -m x",
		`bash -c "git commit -m x"`, `bash -lc 'cd r && git commit -m x'`, `eval git commit -m x`,
		`git log --oneline`, `git status`, `git diff HEAD`, `git merge-base main feat`, `git commit-tree HEAD^{tree} -m x`,
		`git rebase --abort`, `git merge --abort`, `git cherry-pick --quit`, `git commit --help`, `git`, `git -C`,
		`echo "git commit -m x"`, `echo git commit`, `grep -r 'git commit' .`, `# git commit -m x`, `ls # && git commit`,
		`mygit commit`, `gitk commit`, `git log --grep="commit"`, ``, `   `, "cat <<EOF\ngit commit -m x\nEOF",
		`sh -c "sh -c \"sh -c 'sh -c \\\"sh -c git\\\\ commit\\\"'\""`, `eval eval eval eval eval git commit`,
		`eval eval eval eval eval eval git commit`, `x=1 y=2 git pull`, `1x=2 git pull`, `_A1=x git am`,
		`zsh -xc 'git commit'`, `dash --c 'git commit'`, `bash -c`, `xargs -n 1 git commit`, `time -f %e git merge`,
		`command git commit`, `builtin git commit`, `nohup git pull &`, `exec git rebase main`, `! git commit`,
		`while true; do git pull; done`, `until false; do git am; done`, `git -c x commit`, `git --config-env a=b commit`,
		`git --super-prefix p commit`, `git commit -C HEAD`, `git commit --fixup abc --abort`,
		`git commit -m "a\"b" --abort`, "git commit 'it''s' --quit", `GH pr merge`, `gh pr merge`, `gh pr`,
		`gh --repo o/r pr merge`, `gh -R`, `git.exe pull`, `C:\Git\git.EXE pull`, `git pull\`,
		"git \\\r\npull", `git "pull"`, `git p"ul"l`, `$(git pull)`, `a$(b) git pull`, `git commit -x`,
		`git revert -m 1 HEAD`, `git merge -s ours x`, `git rebase --onto main x`, `git rebase --exec "x" main`,
	}
	type cc struct {
		Cmd    string
		Commit bool
	}
	var ccs []cc
	for _, c := range cmds {
		ccs = append(ccs, cc{c, IsCommitCommand(c)})
	}
	v["isCommitCommand"] = ccs

	var splits []map[string]any
	for _, in := range []string{`a b && c`, `a "b c" 'd e'`, `a "x \"y\" \$z"`, `a\ b c`, `a "" b`, `a 'unterminated`,
		`a#b c`, "a\n\n;;b", `a "b\` + "\n" + `c"`, "a \\\r\nb", `a\`, `"x`, `a $(b c) d`, "a `b` c", `a$b`, `a $ (b)`,
		"a # c\nd", `é "ü x"`, `a\"b`} {
		splits = append(splits, map[string]any{"in": in, "out": splitCommands(in)})
	}
	v["splitCommands"] = splits

	shaA, shaB, shaC := strings.Repeat("a", 40), strings.Repeat("b", 40), strings.Repeat("c", 40)
	zero40 := strings.Repeat("0", 40)
	chunks := []string{
		shaA + " " + shaB + " Jo Coder <jo@example.invalid> 1700000000 +0130\tcommit: add things\n",
		zero40 + " " + shaA + " A <a@x> 1700000001 -0700\tcommit (initial): first\n",
		shaA + " " + shaB + " A <a@x> 10 +0000\r\n" + shaB + " " + shaC + "  <b@x> 11 +0000\tcommit: x\r\n",
		strings.ToUpper(shaC) + " " + shaA + " A <a@x> 12 +0000\tcommit: a\tb\n",
		strings.Repeat("0", 64) + " " + strings.Repeat("ab", 32) + " A <a@x> 13 +0000\tcommit (initial): x\n",
		"garbage\n\n" + shaA[:7] + " " + shaB + " A <a@x> 1 +0000\tshort old\n" +
			shaA + " " + shaB[:39] + " A <a@x> 1 +0000\tshort new\n" +
			shaA + " " + strings.Repeat("g", 40) + " A <a@x> 1 +0000\tnot hex\n" +
			shaA + " " + strings.Repeat("ab", 32) + " A <a@x> 1 +0000\tmixed lengths\n" +
			shaA + " " + shaB + " A a@x 1 +0000\tno brackets\n" +
			shaA + " " + shaB + " A <a@x> soon +0000\tbad time\n" +
			shaA + " " + shaB + " A <a@x> 1 0000\tbad zone\n" +
			shaA + " " + shaB + " A <a@x> 1 +0099\tbad minutes\n" +
			shaA + " " + shaB + " A <a@x> 1\tno zone\n" +
			shaA + " " + shaB + " A <a@x> 14 +0000\tgood\n",
		shaA + " " + shaB + " A <a@x> 15 +0000\tcommit: one\n" + shaB + " " + shaC + " A <a@x> 16 +00",
		shaA + " " + shaB, "",
		shaA + " " + shaB + " A <a@x> 1700000000 -0730\tx\n",
		shaA + " " + shaB + " A <a@x> 1 +-500\tsigned hour\n",
		shaA + " " + shaB + " A <a@x> 1 +00-5\tsigned minute\n",
		shaA + " " + shaB + " A <a> b <a@x> 1 +0000\tbrackets in name\n",
		shaA + " " + shaB + " A <a@x> +17 +0000\tplus time\n",
		shaA + " " + shaB + " Ünï Cödé <ü@x> 17 +1400\tcommit: é\n",
		shaA + "  " + shaB + " A <a@x> 18 +0000\tdouble space\n",
		shaA + " " + shaB + " A <a@x> 19  +0000 \tspaced fields\n",
	}
	var refl []map[string]any
	for _, c := range chunks {
		es, n := ParseReflog([]byte(c))
		if es == nil {
			es = []ReflogEntry{}
		}
		refl = append(refl, map[string]any{"data": c, "entries": es, "consumed": n})
	}
	v["parseReflog"] = refl

	blob := "ce013625030ba8dba906f756967f9e9ca394464a"
	var raws []map[string]any
	for _, in := range []string{"", ":000000 100644 " + zero40 + " " + blob + " A\x00a.txt\x00",
		":100644 120000 " + blob + " " + blob + " T\x00l\x00", ":100644 100644 " + blob + " " + blob + " M100\x00a\x00",
		":100644 100644 " + blob + " " + blob + " M\x00a\nb\x00", ":100644 100644 " + blob + " " + blob + " M",
		"diff --git a b\x00", ":100644 " + blob + " M\x00a\x00", "\n:100644 100644 " + strings.ToUpper(blob) + " " + strings.ToUpper(blob) + " D\x00x/é.go\x00\n",
		":100644 100644 " + blob + " " + blob + " M\x00a\x00:100644 100644 " + blob + " " + blob + " A\x00b\x00"} {
		fs, err := parseRawZ(in)
		e := ""
		if err != nil {
			e = err.Error()
		}
		raws = append(raws, map[string]any{"in": in, "files": fs, "err": e})
	}
	v["parseRawZ"] = raws

	var under []map[string]any
	for _, p := range [][2]string{{`C:\a\b`, `C:\a\b`}, {`c:/a/b/c`, `C:\a\b`}, {`C:\A\B\C\`, `c:/a/b/`}, {`C:\a\bc`, `C:\a\b`},
		{`C:\a`, `C:\a\b`}, {`D:\a\b`, `C:\a\b`}, {`C:\x`, `C:\`}, {`c:/`, `C:\`}, {`/c/a/b`, `C:\a\b`}, {`/d/a/b`, `C:\a\b`},
		{`\\host\share\repo\x`, `\\HOST\share\repo`}, {"/home/u/repo", "/home/u/repo"}, {"/home/u/repo/x", "/home/u/repo/"},
		{"/home/u/Repo", "/home/u/repo"}, {"/x", "/"}, {"", `C:\a`}, {`C:\a\b\c`, "/c/a/b"}, {`C:\A\b`, "/c/a/b"},
		{`D:\a\b`, "/c/a/b"}, {"/c/a/b/c", "/c/a/b"}, {`C:`, `c:\`}, {`/c`, `C:\`}, {`C:\Ä\x`, `c:\ä`}, {`C:\a//`, `C:\a`}} {
		under = append(under, map[string]any{"dir": p[0], "root": p[1], "under": underRoot(p[0], p[1]), "same": sameRoot(p[0], p[1])})
	}
	v["underRoot"] = under

	// Classify and Committer over generated cases.
	rng := rand.New(rand.NewPCG(20261029, 1))
	roots := []string{`C:\r`, `c:/R`, `C:\r2`, "", `C:\r\`}
	paths := []string{"a.go", "b.go", "dir/c.go", "A.go"}
	blobs := []string{blob, strings.Repeat("1", 40), strings.Repeat("2", 40), strings.ToUpper(strings.Repeat("3", 40))}
	base := time.Unix(1_800_000_000, 0)
	var cls []map[string]any
	for n := 0; n < 300; n++ {
		var cands []EditRef
		for i := 0; i < rng.IntN(7); i++ {
			e := EditRef{
				ID:        fmt.Sprintf("e-%02d", i),
				Root:      roots[rng.IntN(len(roots))],
				Path:      paths[rng.IntN(len(paths))],
				Heuristic: rng.IntN(4) == 0,
				At:        base.Add(time.Duration(rng.IntN(5)) * time.Second),
			}
			switch rng.IntN(4) {
			case 0:
				e.AfterBlob = blobs[rng.IntN(len(blobs))]
			case 1:
				e.AfterBlobLF = blobs[rng.IntN(len(blobs))]
				e.AfterBlob = strings.Repeat("9", 40)
			case 2:
				e.AfterBlob = strings.ToUpper(blobs[rng.IntN(len(blobs))])
			}
			cands = append(cands, e)
		}
		var files []FileChange
		for i := 0; i < 1+rng.IntN(3); i++ {
			st := []string{"A", "M", "D", "T"}[rng.IntN(4)]
			b := blobs[rng.IntN(len(blobs))]
			if st == "D" || rng.IntN(8) == 0 {
				b = zero40
			}
			files = append(files, FileChange{Path: paths[rng.IntN(len(paths))], Blob: b, Status: st})
		}
		c := Commit{SHA: shaA, At: base.Add(time.Duration(rng.IntN(30)-10) * time.Second), Files: files}
		root := roots[rng.IntN(len(roots))]
		var ccmds []CommitCmd
		for i := 0; i < rng.IntN(4); i++ {
			at := base.Add(time.Duration(rng.IntN(60)-30) * time.Second)
			cmd := CommitCmd{Key: fmt.Sprintf("dev/s%d", i), TurnID: fmt.Sprintf("t%d", i),
				Cwd: []string{`C:\r`, `c:/r/sub`, `/c/r`, `C:\other`, ""}[rng.IntN(5)], At: at}
			switch rng.IntN(3) {
			case 0:
				cmd.Start = at.Add(-time.Duration(rng.IntN(40)) * time.Second)
			case 1:
				cmd.Start = at.Add(time.Duration(rng.IntN(5)) * time.Second)
			}
			ccmds = append(ccmds, cmd)
		}
		cls = append(cls, map[string]any{
			"commit": c, "root": root, "cands": cands, "cmds": ccmds,
			"links": Classify(c, root, cands), "committer": Committer(c, root, ccmds),
		})
	}
	v["classify"] = cls

	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "capture", "gitlink.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
