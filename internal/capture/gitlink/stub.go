package gitlink

import (
	"context"
	"errors"
)

// stub: replaced by the gitlink work package.
var errUnimplemented = errors.New("gitlink: not implemented yet")

type execGit struct{}

func (execGit) Run(ctx context.Context, dir string, args ...string) (string, error) {
	return "", errUnimplemented
}
func gitDir(root string) (string, error)           { return "", errUnimplemented }
func parseReflog(data []byte) ([]ReflogEntry, int) { return nil, 0 }
func zeroSHA(s string) bool                        { return false }
func newCommits(ctx context.Context, g Git, root string, e ReflogEntry, seen func(string) bool) ([]string, int, error) {
	return nil, 0, errUnimplemented
}
func describe(ctx context.Context, g Git, root, sha string) (Commit, error) {
	return Commit{}, errUnimplemented
}
func classify(c Commit, root string, cands []EditRef) []FileLink   { return nil }
func isCommitCommand(command string) bool                          { return false }
func committer(c Commit, root string, cmds []CommitCmd) *CommitCmd { return nil }
