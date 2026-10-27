package scrub

// stub: replaced by the scrub work package.
type scrubber struct{}

func newScrubber(exact []string) *Scrubber  { return &Scrubber{impl: &scrubber{}} }
func (s *scrubber) setExact(exact []string) {}
func (s *scrubber) scrub(in string) string  { return in }
func skipFile(path string) bool             { return false }
