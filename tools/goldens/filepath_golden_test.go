// Golden exporter for crates/gocompat filepath (capture.md §7.1, identity.md
// §7.3 trap 4).
//
// Injected into GOROOT's path/filepath test package by
// tools/goldens/export.sh with `go test -overlay`, so it can read Go's own
// test tables. Runs only when IRORI_GOLDEN_OUT is set, and only on Windows:
//
//   - "windows": filepath's answers on this Windows build over every input
//     in Go's tables (unix-shaped and Windows-shaped).
//   - "unix": the expected values Go's tables state for Unix (cleantests,
//     nonwincleantests, jointests, nonwinjointests, reltests, isabstests),
//     which a Windows run cannot compute.
package filepath_test

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

func TestGoldenFilepath(t *testing.T) {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" || runtime.GOOS != "windows" {
		t.Skip("IRORI_GOLDEN_OUT not set, or not Windows")
	}
	type pair struct {
		In  string `json:"in"`
		Out string `json:"out"`
	}
	type join struct {
		Elem []string `json:"elem"`
		Out  string   `json:"out"`
	}
	type rel struct {
		Base string `json:"base"`
		Targ string `json:"targ"`
		Out  string `json:"out"`
		Err  bool   `json:"err,omitempty"`
	}
	type abs struct {
		In  string `json:"in"`
		Abs bool   `json:"abs"`
	}
	var clean, vol []pair
	var joins []join
	var rels []rel
	var abss []abs
	cleanIns := append(append(append([]PathTest{}, cleantests...), nonwincleantests...), wincleantests...)
	// postClean ranges over Go's whole lazybuf, stale bytes included.
	for _, p := range []string{`z/../ab:/../c`, `a/../c:`, `ab:`, `\a\..\??\c:\x`, `z/../\??/c`, `C:\Users\pat\AppData\Local\irori\..\Irori\.`, `\\host\share\..\x`, `//?/C:/a/../b`} {
		cleanIns = append(cleanIns, PathTest{p, ""})
	}
	for _, c := range cleanIns {
		clean = append(clean, pair{c.path, filepath.Clean(c.path)})
		vol = append(vol, pair{c.path, filepath.VolumeName(c.path)})
	}
	for _, c := range volumenametests {
		vol = append(vol, pair{c.path, filepath.VolumeName(c.path)})
	}
	for _, c := range append(append(append([]JoinTest{}, jointests...), nonwinjointests...), winjointests...) {
		joins = append(joins, join{c.elem, filepath.Join(c.elem...)})
	}
	relIns := append(append([]RelTests{}, reltests...), winreltests...)
	for _, c := range relIns {
		out, err := filepath.Rel(c.root, c.path)
		rels = append(rels, rel{c.root, c.path, out, err != nil})
	}
	for _, c := range append(append([]IsAbsTest{}, isabstests...), winisabstests...) {
		abss = append(abss, abs{c.path, filepath.IsAbs(c.path)})
	}
	var uclean []pair
	for _, c := range append(append([]PathTest{}, cleantests...), nonwincleantests...) {
		uclean = append(uclean, pair{c.path, c.result})
	}
	var ujoin []join
	for _, c := range append(append([]JoinTest{}, jointests...), nonwinjointests...) {
		ujoin = append(ujoin, join{c.elem, c.path})
	}
	var urel []rel
	for _, c := range reltests {
		urel = append(urel, rel{c.root, c.path, c.want, c.want == "err"})
	}
	var uabs []abs
	for _, c := range isabstests {
		uabs = append(uabs, abs{c.path, c.isAbs})
	}
	out := map[string]any{
		"windows": map[string]any{"clean": clean, "volumeName": vol, "join": joins, "rel": rels, "isAbs": abss},
		"unix":    map[string]any{"clean": uclean, "join": ujoin, "rel": urel, "isAbs": uabs},
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(out); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(dir, "gocompat", "filepath.json")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
