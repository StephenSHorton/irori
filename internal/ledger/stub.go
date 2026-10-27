package ledger

import "time"

// stub: replaced by the ledger work package.
type ledger struct{}

func key(repo string, n int) string                          { return "" }
func repoKey(repo string) string                             { return "" }
func repoFromKey(k string) string                            { return "" }
func newLedger() *Ledger                                     { return &Ledger{impl: &ledger{}} }
func (l *ledger) add(e Entry) bool                           { return false }
func (l *ledger) owners(key string, now time.Time) []Entry   { return nil }
func (l *ledger) forRepo(repo string, now time.Time) []Entry { return nil }
func (l *ledger) forSession(s string, now time.Time) []Entry { return nil }
func extractRefs(text string, o RefOptions) []Ref            { return nil }
func parseTarget(s string) (Ref, bool)                       { return Ref{}, false }
func fromBash(b BashResult) []BashRef                        { return nil }
