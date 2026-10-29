package home

// Keep this PC awake while it's the server (card #38): idle system sleep
// is held off while the server runs and the setting is on. The display may
// still turn off, and a lid close or the Start menu's Sleep still sleeps it.

// SetAwake changes "Keep this PC awake while it's the server" (Settings →
// Devices → Home server, GUI only).
func (s *Server) SetAwake(on bool) error {
	s.mu.Lock()
	set := s.set
	set.Awake = &on
	if err := saveSettings(s.cfg.DataDir, set); err != nil {
		s.mu.Unlock()
		return err
	}
	s.set = set
	s.mu.Unlock()
	s.Kick()
	return nil
}

// syncAwake takes or releases the hold to match the setting while the
// server runs (stop releases it). A hold that can't be taken leaves the
// server running, with the reason in Status, and is tried again at the
// next check.
func (s *Server) syncAwake() {
	s.mu.Lock()
	r, want := s.run, s.set.AwakeOn()
	if r == nil || want == (r.awake != nil) {
		s.mu.Unlock()
		return
	}
	if !want {
		hold := r.awake
		r.awake, r.awakeErr = nil, ""
		s.mu.Unlock()
		hold.Release()
		return
	}
	s.mu.Unlock()
	hold, err := s.cfg.Awake.Acquire()
	s.mu.Lock()
	defer s.mu.Unlock()
	if err != nil {
		if r.awakeErr == "" {
			s.cfg.Logf("home: can't keep this PC awake: %v", err)
		}
		r.awakeErr = err.Error()
		return
	}
	if s.run != r {
		hold.Release() // stopped meanwhile
		return
	}
	r.awake, r.awakeErr = hold, ""
	s.cfg.Logf("home: keeping this PC awake while it's the server")
}

// awakeStatus is the Devices line for the hold; s.mu is held.
func awakeStatus(r *running) string {
	switch {
	case r.awake != nil:
		return "Keeping this PC awake"
	case r.awakeErr != "":
		return "Can't keep this PC awake: " + r.awakeErr
	}
	return ""
}
