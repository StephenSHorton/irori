package main

import (
	"context"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

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

// fetchTimeout bounds a fetch from another device for one binding call.
const fetchTimeout = 2 * time.Minute

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
}

func (s *AttachmentsService) info(ref blob.FileRef) AttachmentInfo {
	return AttachmentInfo{SHA256: ref.SHA256, Name: blob.SafeName(ref.Name, ref.SHA256), Size: ref.Size,
		Mime: ref.Mime, Device: s.core.DeviceName(ref.Device), Here: s.core.HasFile(ref.SHA256),
		URL: core.AttachmentRoute + ref.SHA256}
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
	ctx, cancel := context.WithTimeout(context.Background(), fetchTimeout)
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
	ctx, cancel := context.WithTimeout(context.Background(), fetchTimeout)
	defer cancel()
	return s.core.SaveCommentFile(ctx, sha, dir)
}

// errRunnable: Open won't launch a program or a script.
var errRunnable = errors.New("Irori doesn't open programs or scripts; save it and open it yourself if you trust it")

// Open opens a comment's file with the OS default app, from a copy in a
// temporary folder (fetched first if it isn't here). Programs and scripts,
// by name or by their bytes, are refused: Save them instead.
func (s *AttachmentsService) Open(sha string) error {
	if s.openFile == nil {
		return errors.New("this Irori has no window to open files from")
	}
	ref, err := s.core.CommentFile(sha)
	if err != nil {
		return err
	}
	if runnableName(blob.SafeName(ref.Name, ref.SHA256)) {
		return errRunnable
	}
	ctx, cancel := context.WithTimeout(context.Background(), fetchTimeout)
	defer cancel()
	ref, f, err := s.core.OpenCommentFile(ctx, sha)
	if err != nil {
		return err
	}
	head := make([]byte, 4)
	n, _ := f.Read(head)
	f.Close()
	if runnableBytes(head[:n]) {
		return errRunnable
	}
	path, err := s.core.SaveCommentFile(ctx, ref.SHA256, filepath.Join(s.openDir, ref.SHA256[:16]))
	if err != nil {
		return err
	}
	return s.openFile(path)
}

// runnableExts are names the OS would run rather than show.
var runnableExts = map[string]bool{
	".exe": true, ".com": true, ".bat": true, ".cmd": true, ".ps1": true, ".psm1": true, ".vbs": true,
	".vbe": true, ".js": true, ".jse": true, ".wsf": true, ".wsh": true, ".msi": true, ".msp": true,
	".scr": true, ".pif": true, ".cpl": true, ".lnk": true, ".url": true, ".hta": true, ".jar": true,
	".reg": true, ".dll": true, ".appref-ms": true, ".application": true, ".msc": true, ".inf": true,
	".sh": true, ".command": true, ".app": true, ".pkg": true, ".dmg": true, ".scpt": true,
	".terminal": true, ".workflow": true, ".action": true, ".py": true, ".pl": true, ".rb": true,
}

func runnableName(name string) bool {
	return runnableExts[strings.ToLower(filepath.Ext(strings.TrimRight(name, ". ")))]
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
