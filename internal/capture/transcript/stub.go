package transcript

import (
	"encoding/json"
	"time"
)

// stub: replaced by the transcript work package.
type parser struct{}

func sourceFor(projectsDir, path string) (Source, bool)            { return Source{}, false }
func skipLine(line []byte) bool                                    { return false }
func newParser(src Source) *Parser                                 { return &Parser{impl: &parser{}} }
func (p *parser) line(line []byte) []Item                          { return nil }
func (p *parser) close(at time.Time, reason string) []Item         { return nil }
func (p *parser) open() *Turn                                      { return nil }
func (p *parser) lastActivity() time.Time                          { return time.Time{} }
func target(name string, input json.RawMessage, cwd string) string { return "" }
func unifiedDiff(path string, hunks []Hunk) (string, int, int)     { return "", 0, 0 }
