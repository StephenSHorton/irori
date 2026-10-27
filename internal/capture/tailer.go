package capture

import (
	"bufio"
	"bytes"
	"context"
	"errors"
	"io"
	"log"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/capture/transcript"
	"github.com/StephenSHorton/irori/internal/fswatch"
	"github.com/StephenSHorton/irori/internal/presence"
)

const (
	// saveEvery is how many bytes a file pass reads between cursor saves.
	saveEvery = 4 << 20
	// readBuf is the line reader's buffer; longer lines are assembled.
	readBuf = 1 << 20
	// maxLine drops a pathological line (it is consumed, never decoded).
	maxLine = 64 << 20
)

var compactBoundary = []byte(`"subtype":"compact_boundary"`)

// tfile is one transcript file being tailed. Owned by the run goroutine;
// size and offset are read under c.mu for Status.
type tfile struct {
	path      string
	src       transcript.Source
	key       string
	parser    *transcript.Parser
	offset    int64 // bytes fed to the parser
	openStart int64 // offset of the line that opened the open turn, -1 when none
	saved     int64 // the cursor last handed to the cursor file
	size      int64
	lastGrow  time.Time
	// quietAt postpones the next quiet check of an open turn that stayed
	// open when it was due (its live card was busy).
	quietAt time.Time
}

// cursor is where a restart resumes: the open turn's first line, so the
// turn is rebuilt whole (rows past it dedupe by id), else the end of what
// was read.
func (f *tfile) cursor() int64 {
	if f.openStart >= 0 {
		return f.openStart
	}
	return f.offset
}

func (c *capture) run(ctx context.Context) error {
	c.loadCursors()
	if err := c.runEvents(ctx); !errors.Is(err, fswatch.ErrUnsupported) {
		return err
	}
	return c.runPolling(ctx)
}

// runPolling is run where no file watcher exists: every pass on a ticker.
func (c *capture) runPolling(ctx context.Context) error {
	c.tailPass(ctx)
	c.repoPass(ctx)
	scan := time.NewTicker(c.opts.ScanInterval)
	defer scan.Stop()
	repos := time.NewTicker(c.opts.RepoInterval)
	defer repos.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil
		case <-scan.C:
			c.tailPass(ctx)
		case <-repos.C:
			c.repoPass(ctx)
		}
	}
}

func (c *capture) loadCursors() {
	if c.cursors != nil {
		return
	}
	cf, err := loadCursors(c.opts.DataDir)
	c.setErr(err)
	c.cursors = cf
}

func (c *capture) saveCursors() {
	if err := c.cursors.save(c.opts.DataDir); err != nil {
		c.setErr(err)
	}
}

// liveCards maps session ids of live roster cards to their cards.
func (c *capture) liveCards() map[string]presence.AgentCard {
	out := map[string]presence.AgentCard{}
	if c.roster == nil {
		return out
	}
	for _, card := range c.roster.Agents() {
		if card.Status != "gone" && card.SessionID != "" {
			out[card.SessionID] = card
		}
	}
	return out
}

type found struct {
	path string
	src  transcript.Source
	size int64
	mod  time.Time
}

// listFiles finds projects/<slug>/<sid>.jsonl and
// projects/<slug>/<sid>/subagents/**/agent-<id>.jsonl.
func (c *capture) listFiles() []found {
	projects := filepath.Join(c.opts.ClaudeDir, "projects")
	slugs, err := os.ReadDir(projects)
	if err != nil {
		return nil
	}
	var out []found
	add := func(p string, fi os.FileInfo) {
		if src, ok := transcript.SourceFor(projects, p); ok {
			out = append(out, found{path: p, src: src, size: fi.Size(), mod: fi.ModTime()})
		}
	}
	for _, s := range slugs {
		if !s.IsDir() {
			continue
		}
		dir := filepath.Join(projects, s.Name())
		ents, err := os.ReadDir(dir)
		if err != nil {
			continue
		}
		for _, e := range ents {
			p := filepath.Join(dir, e.Name())
			if !e.IsDir() {
				if strings.HasSuffix(e.Name(), ".jsonl") {
					if fi, err := e.Info(); err == nil {
						add(p, fi)
					}
				}
				continue
			}
			sub := filepath.Join(p, "subagents")
			if !dirExists(sub) {
				continue
			}
			_ = filepath.WalkDir(sub, func(q string, d os.DirEntry, err error) error {
				if err != nil || d.IsDir() || !strings.HasSuffix(d.Name(), ".jsonl") {
					return nil
				}
				if fi, err := d.Info(); err == nil {
					add(q, fi)
				}
				return nil
			})
		}
	}
	return out
}

// tailPass reads every transcript's new lines once.
func (c *capture) tailPass(ctx context.Context) {
	c.loadCursors()
	live := c.liveCards()
	files := c.listFiles()
	// Live sessions first, then newest first.
	sort.SliceStable(files, func(i, j int) bool {
		_, li := live[files[i].src.SessionID]
		_, lj := live[files[j].src.SessionID]
		if li != lj {
			return li
		}
		return files[i].mod.After(files[j].mod)
	})
	for _, fd := range files {
		if ctx.Err() != nil {
			return
		}
		card, isLive := live[fd.src.SessionID]
		c.tailFile(ctx, fd, isLive, card)
	}
	c.endTail(ctx, true)
}

// tailPaths reads the new lines of the named transcripts only (paths the
// watcher reported); anything that isn't a transcript is ignored.
func (c *capture) tailPaths(ctx context.Context, paths []string) {
	c.loadCursors()
	projects := filepath.Join(c.opts.ClaudeDir, "projects")
	live := c.liveCards()
	for _, p := range paths {
		if ctx.Err() != nil {
			return
		}
		src, ok := transcript.SourceFor(projects, p)
		if !ok {
			continue
		}
		fi, err := os.Stat(p)
		if err != nil || fi.IsDir() {
			continue
		}
		card, isLive := live[src.SessionID]
		c.tailFile(ctx, found{path: p, src: src, size: fi.Size(), mod: fi.ModTime()}, isLive, card)
	}
	c.endTail(ctx, false)
}

// endTail saves cursors, relinks and refreshes Status after a tail; full
// marks a pass over every file (the first one ends the backfill).
func (c *capture) endTail(ctx context.Context, full bool) {
	c.saveCursors()
	c.relink(ctx)

	c.mu.Lock()
	var backlog int64
	for _, f := range c.files {
		if f.size > f.offset {
			backlog += f.size - f.offset
		}
	}
	c.backlog = backlog
	c.lastScan = c.now()
	if full && c.firstPass {
		c.firstPass = false
		c.backfilling = false
	}
	c.mu.Unlock()
}

// track starts tailing a file, choosing where to begin.
func (c *capture) track(fd found, isLive bool) *tfile {
	key := c.opts.DeviceID + "/" + fd.src.SessionID
	f := &tfile{
		path: fd.path, src: fd.src, key: key, parser: transcript.NewParser(fd.src),
		openStart: -1, lastGrow: fd.mod,
	}
	if off, ok := c.cursors.Files[fd.path]; ok {
		f.offset = off
		if off > fd.size {
			f.offset = 0
		}
	} else {
		f.offset = c.startOffset(fd, isLive)
		c.cursors.setFile(fd.path, f.offset)
	}
	f.saved = f.offset
	c.mu.Lock()
	c.files[fd.path] = f
	c.mu.Unlock()
	return f
}

// startOffset applies the backfill rules (design §2) to a file seen for the
// first time.
func (c *capture) startOffset(fd found, isLive bool) int64 {
	max := c.opts.BackfillMaxBytes
	if fd.size <= max && c.now().Sub(fd.mod) <= c.opts.BackfillWindow {
		return 0
	}
	if isLive && fd.size > max {
		if off, err := boundaryStart(fd.path, fd.size, max); err == nil {
			return off
		}
	}
	return fd.size
}

// boundaryStart scans the last max bytes of a file for its last
// compact_boundary row and returns that row's offset, else the first line
// start in that window.
func boundaryStart(path string, size, max int64) (int64, error) {
	fh, err := os.Open(path)
	if err != nil {
		return 0, err
	}
	defer fh.Close()
	low := size - max
	if low < 0 {
		low = 0
	}
	first := int64(-1)
	best := int64(-1)
	sr := io.NewSectionReader(fh, low, size-low)
	_, err = eachLine(sr, low, func(start int64, line []byte) bool {
		if low > 0 && start == low {
			// The window starts mid-line unless the byte before is '\n'.
			var prev [1]byte
			if _, err := fh.ReadAt(prev[:], low-1); err != nil || prev[0] != '\n' {
				return true
			}
		}
		if first < 0 {
			first = start
		}
		if bytes.Contains(line, compactBoundary) {
			best = start
		}
		return true
	})
	if err != nil {
		return 0, err
	}
	switch {
	case best >= 0:
		return best, nil
	case first >= 0:
		return first, nil
	}
	return size, nil
}

// eachLine reads complete lines from r (whose first byte is at offset base)
// and calls fn with each line's offset and bytes (newline included). It
// stops when fn returns false or at a trailing partial line, and returns the
// bytes consumed. Lines over maxLine are consumed without a call.
func eachLine(r io.Reader, base int64, fn func(start int64, line []byte) bool) (int64, error) {
	br := bufio.NewReaderSize(r, readBuf)
	var consumed, pending int64 // pending: bytes of the current line so far
	var long []byte
	over := false
	for {
		chunk, err := br.ReadSlice('\n')
		pending += int64(len(chunk))
		switch {
		case err == nil:
			var line []byte
			switch {
			case over:
			case long != nil:
				line = append(long, chunk...)
			default:
				line = chunk
			}
			start := base + consumed
			consumed += pending
			pending, long, over = 0, nil, false
			if line != nil && !fn(start, line) {
				return consumed, nil
			}
		case errors.Is(err, bufio.ErrBufferFull):
			if over {
				continue
			}
			if pending > maxLine {
				over, long = true, nil
				continue
			}
			long = append(long, chunk...)
		case errors.Is(err, io.EOF):
			return consumed, nil
		default:
			return consumed, err
		}
	}
}

// tailFile reads one file's new lines into rows.
func (c *capture) tailFile(ctx context.Context, fd found, isLive bool, card presence.AgentCard) {
	c.mu.RLock()
	f := c.files[fd.path]
	c.mu.RUnlock()
	if f == nil {
		f = c.track(fd, isLive)
	}
	if fd.size < f.offset {
		// Shrunk: start over.
		f.offset, f.openStart = 0, -1
		f.parser = transcript.NewParser(f.src)
	}
	c.mu.Lock()
	f.size = fd.size
	c.mu.Unlock()
	grew := false
	if fd.size > f.offset {
		grew = c.readFile(ctx, f, fd.size)
	}
	now := c.now()
	if grew {
		f.lastGrow = now
	}
	if f.parser.Open() != nil && now.Sub(f.lastGrow) >= c.opts.QuietAfter && !(isLive && card.Status == "busy") {
		at := f.parser.LastActivity()
		if at.IsZero() {
			at = now
		}
		if err := c.handle(ctx, f, f.parser.Close(at, "quiet")); err != nil {
			c.setErr(err)
		}
		f.openStart = -1
	}
	c.cursors.setFile(f.path, f.cursor())
	f.saved = f.cursor()

	ok := openKey(f.key, f.src.AgentID)
	open := f.parser.Open()
	c.mu.Lock()
	if open != nil {
		c.open[ok] = open
	} else {
		delete(c.open, ok)
	}
	c.mu.Unlock()
	if grew && open != nil {
		c.notify(Change{Kind: ChangedOpenTurn, Key: f.key})
	}
}

// readFile feeds [offset, size) to the parser up to the last newline and
// reports whether anything was read. On a failed append the parser is reset
// to the last saved cursor, so the next pass reads those lines again.
func (c *capture) readFile(ctx context.Context, f *tfile, size int64) bool {
	fh, err := os.Open(f.path)
	if err != nil {
		return false
	}
	defer fh.Close()
	start := f.offset
	sr := io.NewSectionReader(fh, f.offset, size-f.offset)
	var failed error
	sinceSave := int64(0)
	_, err = eachLine(sr, f.offset, func(at int64, line []byte) bool {
		if ctx.Err() != nil {
			failed = ctx.Err()
			return false
		}
		end := at + int64(len(line))
		if !transcript.SkipLine(line) {
			hadOpen := f.openStart >= 0
			items := f.parser.Line(line)
			closed := false
			for _, it := range items {
				if it.Turn != nil {
					closed = true
				}
			}
			if !hadOpen || closed {
				if f.parser.Open() != nil {
					f.openStart = at
				} else {
					f.openStart = -1
				}
			}
			if err := c.handle(ctx, f, items); err != nil {
				failed = err
				return false
			}
		}
		sinceSave += end - f.offset
		f.offset = end
		if sinceSave >= saveEvery {
			sinceSave = 0
			c.cursors.setFile(f.path, f.cursor())
			f.saved = f.cursor()
			c.saveCursors()
		}
		return true
	})
	if err != nil && failed == nil {
		failed = err
	}
	if failed != nil {
		if ctx.Err() == nil {
			c.setErr(failed)
			log.Printf("capture: %s: %v", filepath.Base(f.path), failed)
		}
		// Re-read from the last saved cursor next time; ids dedupe.
		f.parser = transcript.NewParser(f.src)
		f.offset, f.openStart = f.saved, -1
		return false
	}
	return f.offset > start
}
