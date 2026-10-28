package main

import (
	"slices"
	"strings"
	"testing"
)

// node install registers `irori serve --headless` at logon with the exe
// path quoted, and shows the command it runs. Nothing here runs schtasks.
func TestNodeInstallArgs(t *testing.T) {
	exe := `C:\Users\me\AppData\Local\Programs\Irori\irori.exe`
	got := nodeInstallArgs(exe)
	want := []string{"schtasks", "/Create", "/TN", "Irori node", "/SC", "ONLOGON", "/RL", "LIMITED", "/F",
		"/TR", `"C:\Users\me\AppData\Local\Programs\Irori\irori.exe" serve --headless`}
	if !slices.Equal(got, want) {
		t.Fatalf("install args\n got %q\nwant %q", got, want)
	}
	shown := strings.Join(quoteArgs(got), " ")
	if !strings.Contains(shown, `/TN "Irori node"`) || !strings.Contains(shown, `/TR "\"C:\Users\me\AppData\Local\Programs\Irori\irori.exe\" serve --headless"`) {
		t.Fatalf("shown command %s", shown)
	}
	if got := nodeUninstallArgs(); !slices.Equal(got, []string{"schtasks", "/Delete", "/TN", "Irori node", "/F"}) {
		t.Fatalf("uninstall args %q", got)
	}
	if err := runNode([]string{"start"}); err == nil || !strings.Contains(err.Error(), "usage") {
		t.Fatalf("bad verb: %v", err)
	}
}
