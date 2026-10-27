package capture

import (
	"os"
	"path/filepath"
	"testing"
)

func TestCommandDir(t *testing.T) {
	cwd := `C:\Users\me`
	home, _ := os.UserHomeDir()
	for _, tc := range []struct{ cmd, want string }{
		{`git commit -m x`, cwd},
		{`git -C C:/Users/me/repo commit -m x`, `C:/Users/me/repo`},
		{`git -C "C:\Users\me\my repo" add . && git -C "C:\Users\me\my repo" commit -m x`, `C:\Users\me\my repo`},
		{`git -c user.name=a -C C:/r commit`, `C:/r`},
		{`git --no-pager -C C:/r commit`, `C:/r`},
		{`git -C sub commit -m x`, filepath.Join(cwd, "sub")},
		{`cd C:/r && git commit -m x`, `C:/r`},
		{`Set-Location -Path 'C:\r'; git commit -m x`, `C:\r`},
		{`echo cd later`, cwd},
		// A -C inside the message is text, not an option.
		{`git add foo.go && git commit -m 'docs: use git -C C:/elsewhere'`, cwd},
		{`git commit -m "cd C:/elsewhere && git -C C:/x commit"`, cwd},
		// -C after a cd is relative to where the cd went.
		{`cd sub && git -C .. commit`, cwd},
		{`cd ~/projects/repo && git commit -m x`, filepath.Join(home, "projects", "repo")},
		{`cd $HOME/projects/repo && git commit`, filepath.Join(home, "projects", "repo")},
		{`cd /c/Users/me/repo && git commit`, `C:/Users/me/repo`},
		{`git -C my\ repo commit`, filepath.Join(cwd, "my repo")},
		{`git -C C:/a log && git -C C:/b commit`, `C:/b`},
	} {
		if got := commandDir(tc.cmd, cwd); filepath.Clean(got) != filepath.Clean(tc.want) {
			t.Errorf("commandDir(%q) = %q, want %q", tc.cmd, got, tc.want)
		}
	}
}
