package notify

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"log"
	"os"
	"path/filepath"
	"sync"
)

// SettingsFile is Irori's app settings file in its data folder. It holds
// app-level preferences Irori keeps itself (launch at login stays the OS
// registration); each one is a top-level key, and keys this package doesn't
// know are kept as they are.
const SettingsFile = "settings.json"

// settingsKey is the notifications object's key in SettingsFile.
const settingsKey = "notifications"

// fileMu serializes read-modify-write of the settings file in this process.
var fileMu sync.Mutex

// load reads the saved Options, falling back to DefaultOptions for a
// missing or unreadable file and filling unset fields from the defaults.
func load(dir string) Options {
	o := DefaultOptions()
	if dir == "" {
		return o
	}
	fileMu.Lock()
	defer fileMu.Unlock()
	all, err := readAll(dir)
	if err != nil {
		log.Printf("irori: notifications: %v; using the defaults", err)
		return o
	}
	raw, ok := all[settingsKey]
	if !ok {
		return o
	}
	// Decoding over the defaults keeps a field the file leaves out.
	if err := json.Unmarshal(raw, &o); err != nil {
		log.Printf("irori: notifications: %s: %v; using the defaults", SettingsFile, err)
		return DefaultOptions()
	}
	return o.normalized()
}

// save writes o under settingsKey, keeping the file's other keys.
func save(dir string, o Options) error {
	if dir == "" {
		return nil
	}
	fileMu.Lock()
	defer fileMu.Unlock()
	all, err := readAll(dir)
	if err != nil {
		all = map[string]json.RawMessage{} // a broken file is replaced
	}
	raw, err := json.Marshal(o)
	if err != nil {
		return err
	}
	all[settingsKey] = raw
	out, err := json.MarshalIndent(all, "", "  ")
	if err != nil {
		return err
	}
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, SettingsFile+".*")
	if err != nil {
		return err
	}
	_, werr := tmp.Write(append(out, '\n'))
	cerr := tmp.Close()
	if werr == nil {
		werr = cerr
	}
	if werr == nil {
		werr = os.Rename(tmp.Name(), filepath.Join(dir, SettingsFile))
	}
	if werr != nil {
		os.Remove(tmp.Name())
		return fmt.Errorf("save %s: %w", SettingsFile, werr)
	}
	return nil
}

func readAll(dir string) (map[string]json.RawMessage, error) {
	b, err := os.ReadFile(filepath.Join(dir, SettingsFile))
	if errors.Is(err, fs.ErrNotExist) {
		return map[string]json.RawMessage{}, nil
	}
	if err != nil {
		return nil, err
	}
	all := map[string]json.RawMessage{}
	if err := json.Unmarshal(b, &all); err != nil {
		return nil, fmt.Errorf("%s: %w", SettingsFile, err)
	}
	return all, nil
}
