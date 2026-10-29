package blob

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/StephenSHorton/irori/internal/scrub"
)

// ReceivedDir is where attachment_get puts files by default, inside the
// data dir: <dataDir>/received/<session>/<name>. It is the one part of the
// data dir an attach may read, so a received file can be passed on.
const ReceivedDir = "received"

// ErrNeverLeaves is CheckAttachPath's refusal; the error text says which
// rule the resolved path hit.
var ErrNeverLeaves = errors.New("blob: this file never leaves the device")

// credentialDirs are folders whose files never leave the device, matched
// on any path element (lowercased).
var credentialDirs = []string{".ssh", ".gnupg", ".aws", ".azure", ".kube", ".docker", ".password-store", "keychains"}

// credentialNames are files that never leave, besides scrub.SkipFile's
// (.env*, private keys, .netrc, credentials*).
var credentialNames = []string{".git-credentials", ".credentials.json", ".pgpass", "irori.json", "device.key", "person.json"}

// credentialSuffixes are extensions of key and password stores.
var credentialSuffixes = []string{".keychain", ".keychain-db", ".kdbx", ".ppk", ".gpg"}

// CheckAttachPath resolves path to its canonical form, following symlinks,
// and applies the never-leave rules to the resolved path (§6.6): anything
// inside dataDir (except ReceivedDir), .env files, private keys and
// credential stores are refused with the reason. A relative path is taken
// against cwd; with no cwd it must be absolute. It returns the resolved
// path of a regular file.
func CheckAttachPath(path, cwd, dataDir string) (string, error) {
	path = strings.TrimSpace(path)
	if path == "" {
		return "", errors.New("blob: give a file path")
	}
	if !filepath.IsAbs(path) {
		if cwd == "" || !filepath.IsAbs(cwd) {
			return "", fmt.Errorf("blob: %q is relative and your working directory is unknown: give an absolute path", path)
		}
		path = filepath.Join(cwd, path)
	}
	// The name as given is checked too, so a link named .env is refused
	// whatever it points at.
	if why := neverLeaves(path); why != "" {
		return "", fmt.Errorf("%w: %s", ErrNeverLeaves, why)
	}
	abs, err := filepath.Abs(path)
	if err != nil {
		return "", err
	}
	real, err := filepath.EvalSymlinks(abs)
	if err != nil {
		return "", fmt.Errorf("blob: %w", err)
	}
	if real, err = filepath.Abs(real); err != nil {
		return "", err
	}
	if why := neverLeaves(real); why != "" {
		return "", fmt.Errorf("%w: it resolves to %s, %s", ErrNeverLeaves, real, why)
	}
	if dataDir != "" {
		data := dataDir
		if d, err := filepath.EvalSymlinks(dataDir); err == nil {
			data = d
		}
		if data, err = filepath.Abs(data); err != nil {
			return "", err
		}
		if within(real, data) && !within(real, filepath.Join(data, ReceivedDir)) {
			return "", fmt.Errorf("%w: it resolves into Irori's data dir (%s)", ErrNeverLeaves, real)
		}
	}
	fi, err := os.Stat(real)
	if err != nil {
		return "", fmt.Errorf("blob: %w", err)
	}
	if !fi.Mode().IsRegular() {
		return "", fmt.Errorf("%w: %s", ErrNotFile, real)
	}
	return real, nil
}

// neverLeaves names the rule a path hits, or "".
func neverLeaves(p string) string {
	if scrub.SkipFile(p) {
		return "it is a secrets file (.env, a private key or credentials)"
	}
	elems := strings.FieldsFunc(strings.ToLower(p), func(r rune) bool { return r == '/' || r == '\\' })
	for i, e := range elems {
		e = strings.TrimRight(e, ". ")
		for _, d := range credentialDirs {
			if e == d && i < len(elems)-1 {
				return "it is inside a credential store (" + d + ")"
			}
		}
		if i == len(elems)-1 {
			for _, n := range credentialNames {
				if e == n {
					return "it is a credential or key file (" + n + ")"
				}
			}
			for _, s := range credentialSuffixes {
				if strings.HasSuffix(e, s) {
					return "it is a key or password store (" + s + ")"
				}
			}
		}
		// Windows' credential manager and DPAPI master keys.
		if e == "microsoft" && i+1 < len(elems) && (elems[i+1] == "credentials" || elems[i+1] == "protect") {
			return "it is inside the Windows credential store"
		}
	}
	return ""
}

// Within reports whether p is base or inside it, as CheckAttachPath
// compares.
func Within(p, base string) bool { return within(p, base) }

// within reports whether p is base or inside it (case-insensitively on
// Windows and macOS, whose default file systems are).
func within(p, base string) bool {
	if runtime.GOOS == "windows" || runtime.GOOS == "darwin" {
		p, base = strings.ToLower(p), strings.ToLower(base)
	}
	rel, err := filepath.Rel(base, p)
	if err != nil {
		return false
	}
	return rel == "." || rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator))
}

// maxName caps a sanitized name (runes), keeping its extension.
const maxName = 120

// SafeName reduces an attachment name from a row, which another device
// wrote, to a single safe file name (§6.6): no separators, no "..", no
// drive letter or stream suffix, no control, invisible format (bidi
// overrides) or reserved characters, no trailing dots or spaces, and not a
// reserved Windows device name. An
// empty result falls back to "attachment-" and the hash's first 12 hex.
func SafeName(name, sha string) string {
	if i := strings.LastIndexAny(name, `/\`); i >= 0 {
		name = name[i+1:]
	}
	if len(name) >= 2 && name[1] == ':' && name[0] < utf8.RuneSelf && unicode.IsLetter(rune(name[0])) {
		name = name[2:]
	}
	if i := strings.IndexByte(name, ':'); i >= 0 {
		name = name[:i] // an alternate data stream
	}
	var b strings.Builder
	for _, r := range name {
		switch {
		case r < 0x20 || r == 0x7f || unicode.IsControl(r) || r == utf8.RuneError,
			unicode.Is(unicode.Cf, r): // bidi overrides and other invisible format characters
			b.WriteByte('_')
		case strings.ContainsRune(`<>:"/\|?*`, r):
			b.WriteByte('_')
		default:
			b.WriteRune(r)
		}
	}
	name = strings.TrimSpace(strings.TrimRight(b.String(), ". "))
	name = strings.TrimLeft(name, ". ")
	if name == "" {
		id := sha
		if len(id) > 12 {
			id = id[:12]
		}
		return "attachment-" + id
	}
	if reservedName(name) {
		name = "_" + name
	}
	if r := []rune(name); len(r) > maxName {
		ext := []rune(filepath.Ext(name))
		if len(ext) > 16 {
			ext = nil
		}
		name = string(r[:maxName-len(ext)]) + string(ext)
	}
	return name
}

// reservedName: CON, PRN, AUX, NUL, COM0-9 and LPT0-9 (and their
// superscript-digit forms), with or without an extension, in any case.
func reservedName(name string) bool {
	base := strings.ToUpper(name)
	if i := strings.IndexByte(base, '.'); i >= 0 {
		base = base[:i]
	}
	base = strings.TrimRight(base, " ")
	switch base {
	case "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$":
		return true
	}
	if r := []rune(base); len(r) == 4 && (string(r[:3]) == "COM" || string(r[:3]) == "LPT") {
		d := r[3]
		return d >= '0' && d <= '9' || d == '¹' || d == '²' || d == '³'
	}
	return false
}
