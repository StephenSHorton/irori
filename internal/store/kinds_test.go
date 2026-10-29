package store

import (
	"go/ast"
	"go/parser"
	"go/token"
	"io/fs"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"testing"
)

// logKindDirs are the packages that write log rows.
var logKindDirs = []string{"store", "inbox", "coord", "capture", "core", "identity"}

var dottedKind = regexp.MustCompile(`^[a-z]+(\.[a-z]+)+$`)

// TestEveryKindIsClassified finds every Kind* string constant in the
// packages that write rows and requires the registry to say whether it
// replicates, so a new kind can't ship unclassified (v0.4 §5.1).
func TestEveryKindIsClassified(t *testing.T) {
	found := map[string]string{}
	for _, dir := range logKindDirs {
		root := filepath.Join("..", dir)
		err := filepath.WalkDir(root, func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if d.IsDir() {
				if path != root {
					return filepath.SkipDir // subpackages write no rows
				}
				return nil
			}
			if !strings.HasSuffix(path, ".go") || strings.HasSuffix(path, "_test.go") {
				return nil
			}
			f, err := parser.ParseFile(token.NewFileSet(), path, nil, 0)
			if err != nil {
				return err
			}
			for _, decl := range f.Decls {
				gd, ok := decl.(*ast.GenDecl)
				if !ok || gd.Tok != token.CONST {
					continue
				}
				for _, spec := range gd.Specs {
					vs := spec.(*ast.ValueSpec)
					for i, name := range vs.Names {
						if !strings.HasPrefix(name.Name, "Kind") || i >= len(vs.Values) {
							continue
						}
						lit, ok := vs.Values[i].(*ast.BasicLit)
						if !ok || lit.Kind != token.STRING {
							continue
						}
						v, _ := strconv.Unquote(lit.Value)
						if dottedKind.MatchString(v) {
							found[v] = path + ": " + name.Name
						}
					}
				}
			}
			return nil
		})
		if err != nil {
			t.Fatal(err)
		}
	}
	if len(found) < 40 {
		t.Fatalf("found only %d kinds; is the scan looking in the right place?", len(found))
	}
	for kind, where := range found {
		if KindSide(kind) == Unclassified {
			t.Errorf("%s (%s) is not in kindSides: say whether it replicates or stays on the device", kind, where)
		}
	}
	// The Board column (v0.5 §2.2): every replicated kind says whether a
	// board log accepts it, so a new kind can't ship unclassified for
	// boards either.
	for kind, side := range kindSides {
		if side == Replicated && KindBoard(kind) == BoardUnclassified {
			t.Errorf("%s is not in kindBoards: say whether a board log accepts it (v0.5 §2.2)", kind)
		}
	}
	for kind := range kindBoards {
		if KindSide(kind) == Unclassified {
			t.Errorf("%s is in kindBoards but not in kindSides", kind)
		}
	}
	for _, local := range []string{KindPolicySet, KindMemberJoined} {
		if !IsLocal(local) {
			t.Errorf("%s must be device-local", local)
		}
	}
	for _, rep := range []string{KindAgentSeen, KindLocalMoved, KindPersonDeviceAdded, "message.sent", "task.created", "capture.turn"} {
		if IsLocal(rep) {
			t.Errorf("%s must replicate", rep)
		}
	}
}
