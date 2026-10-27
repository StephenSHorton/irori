package repo

// Skeleton stubs. The repo work package replaces this file.

type resolver struct{}

func newResolver() *Resolver          { return &Resolver{impl: &resolver{}} }
func (r *Resolver) key(string) string { return "" }
func normalizeRemote(string) string   { return "" }
