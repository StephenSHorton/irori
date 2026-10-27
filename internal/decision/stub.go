package decision

import "errors"

// stub: replaced by the decision work package.
var errUnimplemented = errors.New("decision: not implemented yet")

type view struct{}

func validate(d *Decision) error            { return errUnimplemented }
func parseLines(s string) (int, int, error) { return 0, 0, errUnimplemented }
func makeAnchor(root, path string, start, end int, rk, sha string) (Anchor, error) {
	return Anchor{}, errUnimplemented
}
func check(a Anchor, root string) Status                   { return Status{State: Unknown} }
func newView() *View                                       { return &View{impl: &view{}} }
func (v *view) add(d Decision)                             {}
func (v *view) get(id string) (Decision, bool)             { return Decision{}, false }
func (v *view) canSupersede(by Party, target string) error { return errUnimplemented }
func (v *view) list(f Filter) []Listed                     { return nil }
