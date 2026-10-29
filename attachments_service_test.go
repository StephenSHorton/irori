package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
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
	if err != nil || info.Name != "notes.txt" || !info.Here || info.URL != core.AttachmentRoute+notes || info.Size != 6 {
		t.Fatalf("info = %+v, %v", info, err)
	}
	p, err := s.Save(notes)
	if err != nil || filepath.Dir(p) != downloads {
		t.Fatalf("save = %s, %v", p, err)
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

	// Programs and scripts, by name or by bytes, are never opened.
	for _, sha := range []string{
		commentWith(t, c, "setup.exe", []byte("not really")),
		commentWith(t, c, "readme.txt", []byte("MZ\x90\x00binary")),
		commentWith(t, c, "run.txt", []byte("#!/bin/sh\necho hi\n")),
	} {
		if err := s.Open(sha); err != errRunnable {
			t.Fatalf("open %s = %v", sha, err)
		}
	}
	if len(opened) != 1 {
		t.Fatalf("opened %v", opened)
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
