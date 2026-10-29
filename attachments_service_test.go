package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/coord"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/mesh"
)

// commentWith attaches a file called name holding data to a comment the
// person makes on a new card, and returns its hash.
func commentWith(t *testing.T, c *core.Core, name string, data []byte) string {
	t.Helper()
	ctx := context.Background()
	src := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(src, data, 0o600); err != nil {
		t.Fatal(err)
	}
	by := c.UserSender().Key
	refs, err := c.AttachFiles(ctx, by, []string{src})
	if err != nil {
		t.Fatal(err)
	}
	tk, err := c.Coord().CreateTask(ctx, by, coord.NewTask{Title: "files"})
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := c.Coord().AddCommentWith(ctx, by, tk.ID, "here", false, refs); err != nil {
		t.Fatal(err)
	}
	return refs[0].SHA256
}

func TestAttachmentsServiceSaveOpen(t *testing.T) {
	c := startCore(t, pairOpts(t, mesh.NewFakeNet()))
	downloads := t.TempDir()
	var opened []string
	s := newAttachmentsService(c)
	s.downloads = func() (string, error) { return downloads, nil }
	s.openDir = t.TempDir()
	s.openFile = func(p string) error { opened = append(opened, p); return nil }

	notes := commentWith(t, c, "notes.txt", []byte("hello\n"))
	info, err := s.Info(notes)
	if err != nil || info.Name != "notes.txt" || !info.Here || info.URL != core.AttachmentRoute+notes || info.Size != 6 || !info.Openable {
		t.Fatalf("info = %+v, %v", info, err)
	}
	p, err := s.Save(notes)
	if err != nil || filepath.Dir(p) != downloads {
		t.Fatalf("save = %s, %v", p, err)
	}
	if runtime.GOOS == "windows" {
		if b, err := os.ReadFile(p + ":Zone.Identifier"); err != nil || !strings.Contains(string(b), "ZoneId=3") {
			t.Fatalf("saved file's Mark of the Web = %q, %v", b, err)
		}
	}
	os.WriteFile(p, []byte("changed"), 0o600)
	p2, err := s.Save(notes)
	if err != nil || filepath.Base(p2) != "notes (2).txt" {
		t.Fatalf("save over a changed file = %s, %v", p2, err)
	}
	if err := s.Open(notes); err != nil || len(opened) != 1 || filepath.Base(opened[0]) != "notes.txt" {
		t.Fatalf("open = %v, %v", err, opened)
	}
	if b, _ := os.ReadFile(opened[0]); string(b) != "hello\n" {
		t.Fatalf("opened copy = %q", b)
	}

	// Only viewable types open: nothing unlisted by name (programs, macro
	// documents, help files, disk images, web pages).
	for _, name := range []string{"setup.exe", "report.docm", "guide.chm", "disk.iso", "page.html", "x.svg", "noext"} {
		sha := commentWith(t, c, name, []byte("not really "+name))
		if info, _ := s.Info(sha); info.Openable {
			t.Fatalf("%s is openable", name)
		}
		if err := s.Open(sha); err != errNotOpenable {
			t.Fatalf("open %s = %v", name, err)
		}
	}
	// Nor programs and scripts by their bytes.
	for _, sha := range []string{
		commentWith(t, c, "readme.txt", []byte("MZ\x90\x00binary")),
		commentWith(t, c, "run.txt", []byte("#!/bin/sh\necho hi\n")),
	} {
		if err := s.Open(sha); err != errNotOpenable {
			t.Fatalf("open %s = %v", sha, err)
		}
	}
	if len(opened) != 1 {
		t.Fatalf("opened %v", opened)
	}
	if runtime.GOOS == "windows" {
		if _, err := os.Stat(opened[0] + ":Zone.Identifier"); err != nil {
			t.Fatalf("opened copy has no Mark of the Web: %v", err)
		}
	}
	if _, err := s.Info("00" + notes[2:]); err == nil {
		t.Fatal("info for a hash no comment carries")
	}
}

// The middleware serves only its route and hands every other path to the
// app's assets.
func TestAttachmentRouteMiddleware(t *testing.T) {
	c := startCore(t, pairOpts(t, mesh.NewFakeNet()))
	sha := commentWith(t, c, "a.txt", []byte("a"))
	assets := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.Write([]byte("asset")) })
	h := attachmentRoute(c)(assets)
	for path, want := range map[string]string{"/": "asset", "/index.html": "asset", core.AttachmentRoute + sha: "a"} {
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, path, nil))
		if rec.Body.String() != want {
			t.Fatalf("%s = %d %q", path, rec.Code, rec.Body)
		}
	}
}

// The same bytes on two comments under two names: Open checks and opens
// the earliest comment's name every time, so a later comment can't swap a
// script in under a name that was checked as text.
func TestAttachmentsServiceOpenUsesOneStableName(t *testing.T) {
	c := startCore(t, pairOpts(t, mesh.NewFakeNet()))
	s := newAttachmentsService(c)
	s.downloads = func() (string, error) { return t.TempDir(), nil }
	s.openDir = t.TempDir()
	var opened []string
	s.openFile = func(p string) error { opened = append(opened, p); return nil }

	page := []byte("<html><script>alert(1)</script></html>")
	sha := commentWith(t, c, "notes.txt", page)
	if again := commentWith(t, c, "notes.hta", page); again != sha {
		t.Fatalf("the same bytes hashed %s and %s", sha, again)
	}
	for range 30 {
		if err := s.Open(sha); err != nil {
			t.Fatalf("open = %v", err)
		}
		if info, _ := s.Info(sha); info.Name != "notes.txt" {
			t.Fatalf("info name = %q", info.Name)
		}
	}
	for _, p := range opened {
		if filepath.Base(p) != "notes.txt" {
			t.Fatalf("opened %s", p)
		}
	}

	// The other way round: the earliest name is a script, so it never opens.
	script := []byte("<script>x()</script>")
	hta := commentWith(t, c, "run.hta", script)
	commentWith(t, c, "run.txt", script)
	for range 30 {
		if err := s.Open(hta); err != errNotOpenable {
			t.Fatalf("open = %v", err)
		}
	}
}

// The window's canOpen (frontend/src/lib/attachments.ts) mirrors
// openableExts, so it never shows an Open that Open refuses.
func TestOpenableExtsMatchTheWindow(t *testing.T) {
	src, err := os.ReadFile(filepath.Join("frontend", "src", "lib", "attachments.ts"))
	if err != nil {
		t.Fatal(err)
	}
	_, list, ok := strings.Cut(string(src), "const OpenableExts = new Set([")
	list, _, ok2 := strings.Cut(list, "])")
	if !ok || !ok2 {
		t.Fatal("no OpenableExts in attachments.ts")
	}
	got := map[string]bool{}
	for _, f := range strings.Split(list, ",") {
		if f = strings.Trim(strings.TrimSpace(f), `"`); f != "" {
			got[f] = true
		}
	}
	if len(got) != len(openableExts) {
		t.Fatalf("window has %d types, Open %d", len(got), len(openableExts))
	}
	for ext := range openableExts {
		if !got[ext] {
			t.Fatalf("the window can't open %s", ext)
		}
	}
}
