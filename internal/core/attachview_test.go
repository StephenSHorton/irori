package core

import (
	"bytes"
	"context"
	"errors"
	"image"
	"image/png"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/coord"
)

// pngBytes is a small PNG.
func pngBytes(t *testing.T) []byte {
	t.Helper()
	var b bytes.Buffer
	if err := png.Encode(&b, image.NewRGBA(image.Rect(0, 0, 4, 3))); err != nil {
		t.Fatal(err)
	}
	return b.Bytes()
}

// commentFile writes data to a file called name and attaches it to a new
// comment on a new card from h's agent; it returns the hash.
func commentFile(t *testing.T, h *hcore, name string, data []byte) string {
	t.Helper()
	src := filepath.Join(t.TempDir(), name)
	if err := os.WriteFile(src, data, 0o600); err != nil {
		t.Fatal(err)
	}
	tk, err := h.c.Coord().CreateTask(context.Background(), h.agent(), coord.NewTask{Title: "card for " + name})
	if err != nil {
		t.Fatal(err)
	}
	out := h.tool("board_comment", map[string]any{"task_id": tk.ID, "text": "attached " + name, "attach": []string{src}})
	m := shaRe.FindStringSubmatch(out)
	if m == nil {
		t.Fatalf("board_comment = %s", out)
	}
	return m[1]
}

func getRoute(h *hcore, sha string) *httptest.ResponseRecorder {
	rec := httptest.NewRecorder()
	h.c.AttachmentHandler().ServeHTTP(rec, httptest.NewRequest(http.MethodGet, AttachmentRoute+sha, nil))
	return rec
}

// The window's route serves a comment's image inline by its bytes, a
// non-image as a download, and nothing for a hash no comment carries (a
// message's file included).
func TestAttachmentRouteServesCommentFiles(t *testing.T) {
	h := newHCore(t, nil, "pc", "aaaaaaaa-1111-4111-8111-00000000000a")
	img := pngBytes(t)

	// Named .txt, but the bytes are a PNG: the bytes decide.
	sha := commentFile(t, h, "shot.txt", img)
	rec := getRoute(h, sha)
	if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != "image/png" || !bytes.Equal(rec.Body.Bytes(), img) {
		t.Fatalf("image: %d %q (%d bytes)", rec.Code, rec.Header().Get("Content-Type"), rec.Body.Len())
	}
	if rec.Header().Get("X-Content-Type-Options") != "nosniff" || !strings.HasPrefix(rec.Header().Get("Content-Disposition"), "inline") {
		t.Fatalf("image headers: %v", rec.Header())
	}

	// HTML named .png is still a download.
	page := commentFile(t, h, "evil.png", []byte("<html><script>alert(1)</script></html>"))
	rec = getRoute(h, page)
	if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != "application/octet-stream" ||
		rec.Header().Get("Content-Disposition") != `attachment; filename=evil.png` || rec.Header().Get("X-Content-Type-Options") != "nosniff" {
		t.Fatalf("non-image: %d %v", rec.Code, rec.Header())
	}

	// Unknown, malformed, a path below a hash, and a file stored here that
	// no comment carries: 404.
	loose := filepath.Join(t.TempDir(), "loose.png")
	os.WriteFile(loose, pngBytes(t)[:40], 0o600)
	refs, err := h.c.AttachFiles(context.Background(), h.agent(), []string{loose})
	if err != nil || !h.c.HasFile(refs[0].SHA256) {
		t.Fatalf("attach = %v, %v", refs, err)
	}
	m := []string{"", refs[0].SHA256}
	for _, s := range []string{strings.Repeat("ab", 32), "nothex", sha + "/x", m[1]} {
		if rec := getRoute(h, s); rec.Code != http.StatusNotFound {
			t.Fatalf("%s: %d %s", s, rec.Code, rec.Body)
		}
	}
	if _, err := h.c.SaveCommentFile(context.Background(), m[1], t.TempDir()); !errors.Is(err, ErrNoAttachment) {
		t.Fatalf("saving a file no comment carries = %v", err)
	}

	// A POST is refused.
	rec = httptest.NewRecorder()
	h.c.AttachmentHandler().ServeHTTP(rec, httptest.NewRequest(http.MethodPost, AttachmentRoute+sha, nil))
	if rec.Code != http.StatusMethodNotAllowed {
		t.Fatalf("POST = %d", rec.Code)
	}

	// Save never overwrites, never writes into the data dir, and lands on
	// the same file for the same bytes.
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "evil.png"), []byte("mine"), 0o600)
	p, err := h.c.SaveCommentFile(context.Background(), page, dir)
	if err != nil || filepath.Base(p) != "evil (2).png" {
		t.Fatalf("save = %s, %v", p, err)
	}
	if b, _ := os.ReadFile(filepath.Join(dir, "evil.png")); string(b) != "mine" {
		t.Fatal("an existing file was overwritten")
	}
	if again, err := h.c.SaveCommentFile(context.Background(), page, dir); err != nil || again != p {
		t.Fatalf("save again = %s, %v", again, err)
	}
	if _, err := h.c.SaveCommentFile(context.Background(), page, filepath.Join(h.data, "out")); err == nil {
		t.Fatal("saved into the data dir")
	}
}

// The route fetches a comment's file from the device that attached it,
// and says which device to bring back when none that holds it is
// connected.
func TestAttachmentRouteFetchesAndReportsAnOfflineHolder(t *testing.T) {
	net, pc, mac, _ := meshTrio(t)
	img := pngBytes(t)
	first := commentFile(t, mac, "screen.png", img)
	second := commentFile(t, mac, "later.log", []byte("build ok\n"))
	wait(t, "the comments reach the pc", func() bool {
		_, err1 := pc.c.CommentFile(first)
		_, err2 := pc.c.CommentFile(second)
		return err1 == nil && err2 == nil
	})
	// A message's file is the message's readers', never the window's
	// (other bytes: the same bytes on a comment would be the comment's).
	msg := filepath.Join(t.TempDir(), "for-mac.png")
	os.WriteFile(msg, append(slices.Clone(img), 0), 0o600)
	m := shaRe.FindStringSubmatch(pc.tool("send", map[string]any{"to": mac.agent(), "text": "x", "attach": []string{msg}}))
	if m == nil {
		t.Fatal("send named no file")
	}
	if rec := getRoute(pc, m[1]); rec.Code != http.StatusNotFound {
		t.Fatalf("a message's file through the route: %d", rec.Code)
	}
	if pc.c.HasFile(first) {
		t.Fatal("the pc has the file before asking")
	}
	rec := getRoute(pc, first)
	if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != "image/png" || !bytes.Equal(rec.Body.Bytes(), img) {
		t.Fatalf("fetched image: %d %q %s", rec.Code, rec.Header().Get("Content-Type"), rec.Body)
	}
	if !pc.c.HasFile(first) {
		t.Fatal("the fetched file wasn't kept")
	}

	// The mac goes: the file already here still shows; the other one says
	// it is on the mac.
	label := pc.c.DeviceName(mac.id())
	mac.stop()
	net.Isolate(mac.id())
	if rec := getRoute(pc, first); rec.Code != http.StatusOK {
		t.Fatalf("a local file with the mac away: %d", rec.Code)
	}
	wait(t, "the pc reports the mac offline", func() bool {
		rec := getRoute(pc, second)
		if rec.Code == http.StatusOK {
			t.Fatal("served a file nobody connected holds")
		}
		return rec.Code == http.StatusServiceUnavailable
	})
	rec = getRoute(pc, second)
	if body := rec.Body.String(); !strings.Contains(body, label) || !strings.Contains(body, "isn't connected") || rec.Header().Get("Cache-Control") != "no-store" {
		t.Fatalf("offline: %q %v", body, rec.Header())
	}
	var off *FileOfflineError
	if _, _, err := pc.c.FetchCommentFile(context.Background(), second); !errors.As(err, &off) || !strings.Contains(off.On, label) {
		t.Fatalf("fetch with the mac away = %v", err)
	}
}

// The same bytes on two comments answer with the earliest comment's name
// every time; the route serves GET only; a file over MaxPreview is
// refused without a fetch.
func TestAttachmentRouteStableNameAndPreviewCap(t *testing.T) {
	h := newHCore(t, nil, "pc", "aaaaaaaa-1111-4111-8111-00000000000a")
	ctx := context.Background()
	data := []byte("same bytes\n")
	sha := commentFile(t, h, "first.txt", data)
	if again := commentFile(t, h, "second.hta", data); again != sha {
		t.Fatalf("hashes %s and %s", sha, again)
	}
	for range 50 {
		if ref, err := h.c.CommentFile(sha); err != nil || ref.Name != "first.txt" {
			t.Fatalf("comment file = %+v, %v", ref, err)
		}
	}
	if rec := getRoute(h, sha); !strings.Contains(rec.Header().Get("Content-Disposition"), "first.txt") {
		t.Fatalf("route disposition = %q", rec.Header().Get("Content-Disposition"))
	}

	rec := httptest.NewRecorder()
	h.c.AttachmentHandler().ServeHTTP(rec, httptest.NewRequest(http.MethodHead, AttachmentRoute+sha, nil))
	if rec.Code != http.StatusMethodNotAllowed || rec.Header().Get("Allow") != "GET" {
		t.Fatalf("HEAD = %d %v", rec.Code, rec.Header())
	}

	// A big file on another device: refused before anything is asked for.
	tk, err := h.c.Coord().CreateTask(ctx, h.agent(), coord.NewTask{Title: "big"})
	if err != nil {
		t.Fatal(err)
	}
	big := blob.FileRef{SHA256: strings.Repeat("cd", 32), Name: "huge.png", Size: MaxPreview + 1, Mime: "image/png",
		Device: "bbbbbbbb-2222-4222-8222-00000000000b"}
	if _, _, err := h.c.Coord().AddCommentWith(ctx, h.agent(), tk.ID, "big", false, []blob.FileRef{big}); err != nil {
		t.Fatal(err)
	}
	rec = getRoute(h, big.SHA256)
	if rec.Code != http.StatusRequestEntityTooLarge || !strings.Contains(rec.Body.String(), "save it") {
		t.Fatalf("big = %d %s", rec.Code, rec.Body)
	}
}
