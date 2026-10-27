package capture

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
)

// cursorFileName lives in DataDir. It is local bookkeeping and never
// replicates (design §2).
const cursorFileName = "capture-cursors.json"

// cursorFile is the persisted byte offsets. An entry's presence means its
// file (or reflog) has been seen, so backfill and "start at the end" rules
// run once.
type cursorFile struct {
	Files   map[string]int64 `json:"files"`   // transcript path -> offset to resume at
	Reflogs map[string]int64 `json:"reflogs"` // normRoot -> logs/HEAD offset
	dirty   bool
}

func loadCursors(dataDir string) (*cursorFile, error) {
	cf := &cursorFile{Files: map[string]int64{}, Reflogs: map[string]int64{}}
	b, err := os.ReadFile(filepath.Join(dataDir, cursorFileName))
	if errors.Is(err, fs.ErrNotExist) {
		return cf, nil
	}
	if err != nil {
		return cf, err
	}
	if err := json.Unmarshal(b, cf); err != nil {
		// A damaged cursor file means re-reading; ids keep that safe.
		cf = &cursorFile{Files: map[string]int64{}, Reflogs: map[string]int64{}}
		return cf, nil
	}
	if cf.Files == nil {
		cf.Files = map[string]int64{}
	}
	if cf.Reflogs == nil {
		cf.Reflogs = map[string]int64{}
	}
	return cf, nil
}

func (cf *cursorFile) setFile(path string, off int64) {
	if v, ok := cf.Files[path]; !ok || v != off {
		cf.Files[path] = off
		cf.dirty = true
	}
}

func (cf *cursorFile) setReflog(root string, off int64) {
	if v, ok := cf.Reflogs[root]; !ok || v != off {
		cf.Reflogs[root] = off
		cf.dirty = true
	}
}

// save writes the file atomically (temp file, then rename) when it changed.
func (cf *cursorFile) save(dataDir string) error {
	if !cf.dirty {
		return nil
	}
	b, err := json.Marshal(cf)
	if err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dataDir, cursorFileName+".*.tmp")
	if err != nil {
		return err
	}
	name := tmp.Name()
	if _, err := tmp.Write(b); err != nil {
		tmp.Close()
		os.Remove(name)
		return err
	}
	if err := tmp.Sync(); err != nil {
		tmp.Close()
		os.Remove(name)
		return err
	}
	if err := tmp.Close(); err != nil {
		os.Remove(name)
		return err
	}
	if err := os.Rename(name, filepath.Join(dataDir, cursorFileName)); err != nil {
		os.Remove(name)
		return err
	}
	cf.dirty = false
	return nil
}
