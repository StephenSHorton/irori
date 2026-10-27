package capture

import (
	"path/filepath"
	"testing"
)

func TestCommandDir(t *testing.T) {
	cwd := `C:\Users\me`
	for _, tc := range []struct{ cmd, want string }{
		{`git commit -m x`, cwd},
		{`git -C C:/Users/me/repo commit -m x`, `C:/Users/me/repo`},
		{`git -C "C:\Users\me\my repo" add . && git -C "C:\Users\me\my repo" commit -m x`, `C:\Users\me\my repo`},
		{`git -c user.name=a -C C:/r commit`, `C:/r`},
		{`git -C sub commit -m x`, filepath.Join(cwd, "sub")},
		{`cd C:/r && git commit -m x`, `C:/r`},
		{`Set-Location -Path 'C:\r'; git commit -m x`, `C:\r`},
		{`echo cd later`, cwd},
	} {
		if got := commandDir(tc.cmd, cwd); got != tc.want {
			t.Errorf("commandDir(%q) = %q, want %q", tc.cmd, got, tc.want)
		}
	}
}
