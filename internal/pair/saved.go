package pair

import (
	"context"
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/StephenSHorton/irori/internal/identity"
	"github.com/StephenSHorton/irori/internal/mesh"
	"github.com/StephenSHorton/irori/internal/store"
)

// RowsFile keeps the person rows a pairing handed this device (B) until
// they are stored. They can't be inserted at once (a device's rows are
// gapless, and A's cert rows sit among A's other rows), so B learns them
// (Chain.Learn), which lasts only for the process: without the file, a B
// that stopped before its first sync would hold a person with no certs and
// never start the net layer. The rows are signed and public; no secret is
// in the file.
const RowsFile = "pair-rows.json"

func saveRows(dir string, rows []mesh.Row) error {
	if dir == "" {
		return nil
	}
	raw, err := json.Marshal(rows)
	if err != nil {
		return err
	}
	path := filepath.Join(dir, RowsFile)
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, raw, 0o600); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// LearnSaved folds the saved pairing rows that the store doesn't hold yet
// into ch. The Core calls it at Start, once the store is open. When every
// saved row is held, the file is removed.
func LearnSaved(ctx context.Context, dir string, st *store.Store, ch *identity.Chain) error {
	path := filepath.Join(dir, RowsFile)
	raw, err := os.ReadFile(path)
	if errors.Is(err, fs.ErrNotExist) {
		return nil
	}
	if err != nil {
		return err
	}
	var rows []mesh.Row
	if err := json.Unmarshal(raw, &rows); err != nil {
		return os.Remove(path) // unreadable: the sync will bring the rows
	}
	vec, err := st.Vector(ctx)
	if err != nil {
		return err
	}
	var left []store.Event
	for _, r := range rows {
		if r.S > vec[r.D] {
			left = append(left, r.Event())
		}
	}
	if len(left) == 0 {
		return os.Remove(path)
	}
	learnAll(ch, left)
	return nil
}
