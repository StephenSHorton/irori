package main

import (
	"context"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"github.com/StephenSHorton/irori/internal/blob"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// AttachmentsService shows the window the files agents attach to Board
// comments (card #14): the window loads them from core.AttachmentRoute
// (served by attachmentRoute, inside the app's webview only) and saves or
// opens them through these bindings. Like the route, it reads comment
// files only, never a message's, and fetches a file from the device that
// holds it when it isn't here.
//
// JS binding name: AttachmentsService.*
type AttachmentsService struct {
	core *core.Core
	// downloads is the folder Save writes to (downloadsDir).
	downloads func() (string, error)
	// openFile opens a file with the OS default app (app.Browser.OpenFile
	// once attached; a stub in tests).
	openFile func(string) error
	// openDir is where Open puts a copy to open: outside the data dir.
	openDir string
}

func newAttachmentsService(c *core.Core) *AttachmentsService {
	return &AttachmentsService{core: c, downloads: downloadsDir,
		openDir: filepath.Join(os.TempDir(), "irori-attachments")}
}

// attach sets the OS opener from the app.
func (s *AttachmentsService) attach(app *application.App) { s.openFile = app.Browser.OpenFile }

// AttachmentInfo is one comment file as the window shows it.
type AttachmentInfo struct {
	SHA256 string `json:"sha256"`
	Name   string `json:"name"` // the safe file name
	Size   int64  `json:"size"`
	// Mime is the attaching device's guess (by extension, else content); the
	// route decides by the bytes.
	Mime string `json:"mime"`
	// Device is the name of the device it was attached on.
	Device string `json:"device"`
	// Here: the bytes are on this device, so it shows without a fetch.
	Here bool `json:"here"`
	// URL is where the window loads it: core.AttachmentRoute + SHA256.
	URL string `json:"url"`
	// Openable: Open would open it (by its name; Open checks the bytes
	// too). Anything else is Save only.
	Openable bool `json:"openable"`
}

func (s *AttachmentsService) info(ref blob.FileRef) AttachmentInfo {
	name := blob.SafeName(ref.Name, ref.SHA256)
	return AttachmentInfo{SHA256: ref.SHA256, Name: name, Size: ref.Size,
		Mime: ref.Mime, Device: s.core.DeviceName(ref.Device), Here: s.core.HasFile(ref.SHA256),
		URL: core.AttachmentRoute + ref.SHA256, Openable: openableName(name)}
}

// Info describes a comment's file without fetching it.
func (s *AttachmentsService) Info(sha string) (AttachmentInfo, error) {
	ref, err := s.core.CommentFile(sha)
	if err != nil {
		return AttachmentInfo{}, err
	}
	return s.info(ref), nil
}

// Fetch brings a comment's file to this device (from the device that holds
// it) and describes it. The error says which device to bring online when
// none that holds it is connected.
func (s *AttachmentsService) Fetch(sha string) (AttachmentInfo, error) {
	ctx, cancel := context.WithTimeout(context.Background(), core.FetchTimeout)
	defer cancel()
	ref, _, err := s.core.FetchCommentFile(ctx, sha)
	if err != nil {
		return AttachmentInfo{}, err
	}
	return s.info(ref), nil
}

// Save writes a comment's file into the person's Downloads folder under its
// safe name and returns the path. It never overwrites: a taken name gets
// " (2)", " (3)" ..., unless that file already holds the same bytes.
func (s *AttachmentsService) Save(sha string) (string, error) {
	dir, err := s.downloads()
	if err != nil {
		return "", err
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return "", err
	}
	ctx, cancel := context.WithTimeout(context.Background(), core.FetchTimeout)
	defer cancel()
	return s.core.SaveCommentFile(ctx, sha, dir)
}

// errNotOpenable: Open opens pictures, PDFs, text, plain Office documents
// and media only, never a program, a script or anything else the OS might
// run.
var errNotOpenable = errors.New("Irori opens pictures, PDFs, text, documents and media only; save this one and open it yourself if you trust it")

// Open opens a comment's file with the OS default app, from a copy in a
// temporary folder (fetched first if it isn't here, and marked as from
// elsewhere). Only names in openableExts open, and never bytes that are a
// program or a script: Save the rest.
func (s *AttachmentsService) Open(sha string) error {
	if s.openFile == nil {
		return errors.New("this Irori has no window to open files from")
	}
	// One lookup for the whole call: the name checked is the name written.
	ref, err := s.core.CommentFile(sha)
	if err != nil {
		return err
	}
	if !openableName(blob.SafeName(ref.Name, ref.SHA256)) {
		return errNotOpenable
	}
	ctx, cancel := context.WithTimeout(context.Background(), core.FetchTimeout)
	defer cancel()
	f, err := s.core.OpenFileRef(ctx, ref)
	if err != nil {
		return err
	}
	head := make([]byte, 4)
	n, _ := f.Read(head)
	f.Close()
	if runnableBytes(head[:n]) {
		return errNotOpenable
	}
	path, err := s.core.SaveFileRef(ctx, ref, filepath.Join(s.openDir, ref.SHA256[:16]))
	if err != nil {
		return err
	}
	// The file about to open is the one checked.
	if !openableName(filepath.Base(path)) {
		return errNotOpenable
	}
	return s.openFile(path)
}

// openableExts are the types Open hands to the OS: ones its default apps
// show rather than run. Macro-enabled Office files, archives, disk images,
// web pages, SVGs and everything unlisted are Save only.
var openableExts = map[string]bool{
	".png": true, ".jpg": true, ".jpeg": true, ".gif": true, ".webp": true, ".bmp": true, ".heic": true,
	".pdf": true,
	".txt": true, ".md": true, ".log": true, ".csv": true, ".json": true, ".yaml": true, ".yml": true,
	".docx": true, ".xlsx": true, ".pptx": true,
	".mp4": true, ".mov": true, ".mp3": true, ".wav": true,
}

func openableName(name string) bool {
	return openableExts[strings.ToLower(filepath.Ext(strings.TrimRight(name, ". ")))]
}

// runnableBytes: a PE, ELF or Mach-O binary, or a script with a #! line.
func runnableBytes(b []byte) bool {
	s := string(b)
	return strings.HasPrefix(s, "MZ") || strings.HasPrefix(s, "#!") || strings.HasPrefix(s, "\x7fELF") ||
		s == "\xfe\xed\xfa\xce" || s == "\xfe\xed\xfa\xcf" || s == "\xce\xfa\xed\xfe" || s == "\xcf\xfa\xed\xfe" ||
		s == "\xca\xfe\xba\xbe"
}

// attachmentRoute is the asset-server middleware that serves
// core.AttachmentRoute from the Core; every other path goes on to the
// app's assets.
func attachmentRoute(c *core.Core) application.Middleware {
	h := c.AttachmentHandler()
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if strings.HasPrefix(r.URL.Path, core.AttachmentRoute) {
				h.ServeHTTP(w, r)
				return
			}
			next.ServeHTTP(w, r)
		})
	}
}
