// Golden exporter for the app shell (app/src/shell, app/src/cli.rs;
// architecture.md §4, §6.2).
//
// Injected into the Go repo's root package (package main) by
// tools/goldens/export.sh with `go test -overlay`; it runs only when
// IRORI_GOLDEN_OUT is set. It writes shell/shell.json:
//
//   - instanceIDs: instanceID() for IRORI_HOME values (unset, case,
//     trailing and mixed separators, relative and dotted paths);
//   - statusLines, heldLines, pauseLabels, versionLines: the tray texts;
//   - startedAtLogin and wantsConsole over argument and role tables.
package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestGoldenShell(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	type kv struct {
		In  string `json:"in"`
		Out string `json:"out"`
	}
	homes := []string{
		"",
		`C:\Users\a\AppData\Local\Temp\irori-copy`,
		`c:\users\a\appdata\local\temp\IRORI-COPY\`,
		`C:/Users/a/AppData/Local/Temp/irori-copy`,
		`C:\Users\a\AppData\Local\Temp\.\x\..\irori-copy`,
		`/Users/a/irori-copy`,
		`relative\copy`,
		`É\Ünïcode`,
	}
	var ids []kv
	for _, h := range homes {
		t.Setenv("IRORI_HOME", h)
		ids = append(ids, kv{h, instanceID()})
	}
	type status struct {
		Status string      `json:"status"`
		Counts InboxCounts `json:"counts"`
		Decide int         `json:"decide"`
		Out    string      `json:"out"`
	}
	var statuses []status
	for _, s := range []status{
		{Status: "No agents running"},
		{Status: "1 agent (0 busy)", Counts: InboxCounts{Unread: 1}},
		{Status: "3 agents (2 busy)", Counts: InboxCounts{Unread: 12, Held: 2, Paused: true}, Decide: 4},
		{Status: "Irori (dev build)", Decide: 1},
		{Status: "Irori v0.6.0", Counts: InboxCounts{Held: 5}},
	} {
		s.Out = statusLine(s.Status, s.Counts, s.Decide)
		statuses = append(statuses, s)
	}
	type num struct {
		In  int    `json:"in"`
		Out string `json:"out"`
	}
	var held []num
	for _, n := range []int{1, 2, 10} {
		held = append(held, num{n, heldLine(n)})
	}
	var pause, versions []kv
	for _, d := range []string{"", "Stephen's PC", "mac <mini>"} {
		pause = append(pause, kv{d, pauseLabel(d)})
	}
	for _, v := range []string{"", "dev", "0.6.0", "1.2.3-rc.1"} {
		versions = append(versions, kv{v, versionLine(v)})
	}
	type args struct {
		Args []string `json:"args"`
		Out  bool     `json:"out"`
	}
	var startup []args
	for _, a := range [][]string{{}, {"--startup"}, {"-startup"}, {"x", "--startup"}, {"--startup=true"}} {
		startup = append(startup, args{a, startedAtLogin(a)})
	}
	type role struct {
		Cmd string `json:"cmd"`
		Out bool   `json:"out"`
	}
	var console []role
	for _, c := range []string{"", "gui", "mcp", "serve", "attach", "events", "pair", "node", "version", "bogus"} {
		console = append(console, role{c, wantsConsole(c)})
	}
	out := map[string]any{
		"instanceIDs":    ids,
		"statusLines":    statuses,
		"heldLines":      held,
		"pauseLabels":    pause,
		"versionLines":   versions,
		"startedAtLogin": startup,
		"wantsConsole":   console,
	}
	b, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(dir, "shell"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "shell", "shell.json"), append(b, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
