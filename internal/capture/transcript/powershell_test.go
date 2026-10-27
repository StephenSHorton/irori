package transcript

import "testing"

func TestPowerShell(t *testing.T) {
	cmd := "git add .; if ($?) { git commit -m @'\nfix\n'@ }"
	items, p := feed(t, mainSrc(),
		prompt("u1", "p1", 0, "go", nil),
		assistant("a1", 1, toolUse("t1", "PowerShell", obj{"command": cmd, "description": "commit"})),
		result("r1", 2, "t1", false, obj{"stdout": "[main abc1234] fix\n", "stderr": "", "interrupted": false, "isImage": false}),
	)
	bs := bashes(items)
	if len(bs) != 1 {
		t.Fatalf("items = %v", kinds(items))
	}
	b := bs[0]
	if b.Tool != "PowerShell" || b.Command != cmd || b.Stdout != "[main abc1234] fix\n" || b.TurnID != "p1" || b.Cwd != fxCwd {
		t.Fatalf("run = %+v", b)
	}
	if tc := p.Open().Tools[0]; tc.Name != "PowerShell" || tc.Target != "git add .; if ($?) { git commit -m @'" || !tc.Done {
		t.Fatalf("tool = %+v", tc)
	}

	// Bash runs say so too.
	items, _ = feed(t, mainSrc(),
		prompt("u1", "p1", 0, "go", nil),
		assistant("a1", 1, toolUse("t1", "Bash", obj{"command": "ls"})),
		result("r1", 2, "t1", false, obj{"stdout": "", "stderr": ""}),
	)
	if b := bashes(items); len(b) != 1 || b[0].Tool != "Bash" {
		t.Fatalf("bash = %+v", b)
	}

	// A PowerShell run's bashEditDiff makes heuristic edits like Bash's.
	items, _ = feed(t, mainSrc(),
		prompt("u1", "p1", 0, "go", nil),
		assistant("a1", 1, toolUse("t1", "PowerShell", obj{"command": "gofmt -w ."})),
		result("r1", 2, "t1", false, obj{"stdout": "", "stderr": "", "bashEditDiff": obj{"files": []any{}, "changedFiles": []string{`C:\src\demo\a.go`}}}),
	)
	if es := edits(items); len(es) != 1 || es[0].Tool != "PowerShell" || es[0].Fidelity != Heuristic {
		t.Fatalf("edits = %v", kinds(items))
	}
}

func TestPowerShellTarget(t *testing.T) {
	if got := Target("PowerShell", []byte(`{"command":"\n  Get-ChildItem\r\nls"}`), ""); got != "Get-ChildItem" {
		t.Fatalf("target = %q", got)
	}
}
