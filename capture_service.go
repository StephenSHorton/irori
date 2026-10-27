package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/StephenSHorton/irori/internal/capture"
	"github.com/StephenSHorton/irori/internal/core"
	"github.com/StephenSHorton/irori/internal/decision"
	"github.com/StephenSHorton/irori/internal/inbox"
	"github.com/StephenSHorton/irori/internal/ledger"
	"github.com/StephenSHorton/irori/internal/presence"
	"github.com/wailsapp/wails/v3/pkg/application"
)

// EventCaptureChanged carries a CaptureChange when capture appends rows or a
// live turn grows. Bursts are coalesced (at most captureEmitEvery apart), so
// the UI re-reads what it shows rather than applying each change.
const EventCaptureChanged = "capture:changed"

// captureEmitEvery bounds capture:changed. An open turn reports on every
// tailer pass, and a backfill appends thousands of rows.
const captureEmitEvery = 300 * time.Millisecond

// decisionList is how many decisions the Decisions tab gets (decision.List's max).
const decisionList = 500

func init() {
	application.RegisterEvent[CaptureChange](EventCaptureChanged)
}

// CaptureChange is the capture:changed payload: what moved since the last
// event. Kinds are capture.ChangeKind values (turn, edit, commit, decision,
// pr, open); Keys are agent keys and RepoKeys repo keys, each deduplicated.
type CaptureChange struct {
	Kinds    []string `json:"kinds"`
	Keys     []string `json:"keys"`
	RepoKeys []string `json:"repoKeys"`
}

// Fidelity values for a roster card (design §11).
const (
	FidelityMod     = "mod"     // a Claude session with the Irori mod attached
	FidelityPassive = "passive" // a Claude session captured from its transcript only
	FidelityMCP     = "mcp"     // a Grok member: only what it reports through Irori tools
)

// CaptureRepo is one repo capture has seen.
type CaptureRepo struct {
	RepoKey string   `json:"repoKey"`
	Roots   []string `json:"roots"` // worktree roots seen, sorted
	Live    bool     `json:"live"`  // a live agent is working in it
	// LastCommit is the newest commit capture has seen in it (zero if none).
	LastCommit time.Time `json:"lastCommit"`
}

// CaptureService is the person's view of what agents did: timelines,
// commits and their links, edits, why, decisions and the PR ledger.
// Decisions recorded here are by the person at this device (trust "user").
// JS binding name: CaptureService.*
type CaptureService struct {
	cap    *capture.Capture
	agents func() []presence.AgentCard
	user   func() inbox.Sender
	// reach reports an agent's open wake paths (inbox.Reachable).
	reach func(agent string) (mod, channel bool)
}

func newCaptureService(c *core.Core) *CaptureService {
	return &CaptureService{cap: c.Capture(), agents: c.Agents, user: c.UserSender, reach: c.Inbox().Reachable}
}

// Timeline returns a page of one agent's history, newest first. before is
// RFC 3339 ("" = newest); page on with the last entry's at while More is
// set. limit counts turns (default 20, max 100). Irori messages aren't in
// it: the UI merges them from the inbox.
func (s *CaptureService) Timeline(key, before string, limit int, subagents bool) (capture.Timeline, error) {
	q := capture.TimelineQuery{Key: strings.TrimSpace(key), Limit: limit, Subagents: subagents}
	if b := strings.TrimSpace(before); b != "" {
		t, err := time.Parse(time.RFC3339Nano, b)
		if err != nil {
			return capture.Timeline{}, fmt.Errorf("before must be an RFC 3339 time: %w", err)
		}
		q.Before = t
	}
	tl := s.cap.Timeline(q)
	tl.Entries = nonNil(tl.Entries)
	return tl, nil
}

// Commits lists a repo's commits, newest first, with their links; "" lists
// every repo's. limit default 50, max 500.
func (s *CaptureService) Commits(repoKey string, limit int) []capture.CommitView {
	return nonNil(s.cap.Commits(strings.TrimSpace(repoKey), limit))
}

// Commit returns one commit by sha or a unique prefix of 7 or more hex
// digits, or nil when capture hasn't seen it.
func (s *CaptureService) Commit(sha string) *capture.CommitView {
	v, ok := s.cap.Commit(strings.TrimSpace(sha))
	if !ok {
		return nil
	}
	return &v
}

// Repos lists the repos capture has seen: newest commit first, then live
// ones, then by key.
func (s *CaptureService) Repos() []CaptureRepo {
	live := map[string]bool{}
	for _, a := range s.agents() {
		if a.RepoKey != "" {
			live[a.RepoKey] = true
		}
	}
	out := []CaptureRepo{}
	for k, roots := range s.cap.Repos() {
		rs := slices.Clone(roots)
		sort.Strings(rs)
		r := CaptureRepo{RepoKey: k, Roots: nonNil(rs), Live: live[k]}
		if cs := s.cap.Commits(k, 1); len(cs) > 0 {
			r.LastCommit = cs[0].At
		}
		out = append(out, r)
	}
	// Newest commit first (what the person most likely wants to look at),
	// then live repos, then by key.
	sort.Slice(out, func(i, j int) bool {
		if !out[i].LastCommit.Equal(out[j].LastCommit) {
			return out[i].LastCommit.After(out[j].LastCommit)
		}
		if out[i].Live != out[j].Live {
			return out[i].Live
		}
		return out[i].RepoKey < out[j].RepoKey
	})
	return out
}

// Edits lists captured edits, newest first. Empty arguments mean any; path
// is repo-relative. limit default 50, max 500.
func (s *CaptureService) Edits(repoKey, path, key string, limit int) []capture.EditRow {
	return nonNil(s.cap.Edits(capture.EditQuery{
		RepoKey: strings.TrimSpace(repoKey), Path: strings.TrimSpace(path), Key: strings.TrimSpace(key), Limit: limit,
	}))
}

// Patch returns an edit's unified diff (or a stored summary) by blob id.
func (s *CaptureService) Patch(blobID string) (string, error) {
	b, err := s.cap.Blob(strings.TrimSpace(blobID))
	if err != nil {
		return "", err
	}
	return string(b), nil
}

// Why answers what is behind a path, and a line when line > 0. A relative
// path resolves inside repoKey's worktree.
func (s *CaptureService) Why(path string, line int, repoKey string) (capture.WhyResult, error) {
	repoKey = strings.TrimSpace(repoKey)
	r, err := s.cap.Why(context.Background(), capture.WhyQuery{
		Path: strings.TrimSpace(path), Line: max(line, 0), RepoKey: repoKey, Cwd: s.rootFor(repoKey),
	})
	if err != nil {
		return capture.WhyResult{}, explainNoRepo(err, repoKey)
	}
	r.Edits = nonNil(r.Edits)
	return r, nil
}

// Decisions lists decisions with their anchors re-checked, newest first, at
// most decisionList. all includes superseded ones (the tab asks for them to
// show the supersede chain).
func (s *CaptureService) Decisions(repoKey, path, query string, all bool) []capture.DecisionView {
	return nonNil(s.cap.Decisions(decision.Filter{
		RepoKey: strings.TrimSpace(repoKey), Path: strings.TrimSpace(path), Query: strings.TrimSpace(query), All: all,
		Limit: decisionList,
	}))
}

// RecordDecision records a decision by the person. path (optional) is
// absolute or relative to repoKey's worktree; lines is "" (whole file), "40"
// or "40-52". supersedes lists decision ids it replaces; messageId or turnId
// names what it was marked from.
func (s *CaptureService) RecordDecision(statement, why, path, lines, repoKey string, supersedes []string, messageID, turnID string) (decision.Decision, error) {
	u := s.user()
	repoKey = strings.TrimSpace(repoKey)
	in := capture.DecisionInput{
		By:        decision.Party{Key: u.Key, Name: u.Name, Kind: "user"},
		Trust:     "user",
		Statement: statement,
		Why:       why,
		Path:      strings.TrimSpace(path),
		Lines:     strings.TrimSpace(lines),
		Cwd:       s.rootFor(repoKey),
	}
	for _, id := range supersedes {
		if id = strings.TrimSpace(id); id != "" && !slices.Contains(in.Supersedes, id) {
			in.Supersedes = append(in.Supersedes, id)
		}
	}
	if m, t := strings.TrimSpace(messageID), strings.TrimSpace(turnID); m != "" || t != "" {
		in.Source = &decision.Source{MessageID: m, TurnID: t}
	}
	if in.Path != "" && in.Cwd == "" && !filepath.IsAbs(filepath.FromSlash(in.Path)) {
		return decision.Decision{}, explainNoRepo(capture.ErrNoRepo, repoKey)
	}
	d, err := s.cap.RecordDecision(context.Background(), in)
	if err != nil {
		return decision.Decision{}, explainNoRepo(err, repoKey)
	}
	return d, nil
}

// PRs lists the PR ledger for a repo ("owner/name"; "" = all).
func (s *CaptureService) PRs(repo string) []ledger.Entry {
	return nonNil(s.cap.PRs(strings.TrimSpace(repo)))
}

// Status reports capture's health.
func (s *CaptureService) Status() capture.Status { return s.cap.Status() }

// Fidelity maps each live agent key to how much capture sees of it: mod,
// passive or mcp. It changes with attach streams, which the inbox reports
// through inbox:changed.
func (s *CaptureService) Fidelity() map[string]string {
	out := map[string]string{}
	for _, a := range s.agents() {
		out[a.Key] = s.fidelity(a)
	}
	return out
}

func (s *CaptureService) fidelity(a presence.AgentCard) string {
	if a.Agent == "grok" {
		return FidelityMCP
	}
	if s.reach != nil {
		if mod, _ := s.reach(a.Key); mod {
			return FidelityMod
		}
	}
	return FidelityPassive
}

// rootFor is a worktree root of repoKey: the one holding a live agent's cwd,
// else a seen root that is still on disk, else "". It is a root, never the
// agent's cwd itself: capture resolves a relative path against the cwd
// first, and the person's paths are repo-relative, so an agent working in
// sub/ must not turn README.md into sub/README.md.
func (s *CaptureService) rootFor(repoKey string) string {
	if repoKey == "" {
		return ""
	}
	for _, a := range s.agents() {
		if a.RepoKey == repoKey && a.Cwd != "" && isDir(a.Cwd) {
			if r := worktreeRoot(a.Cwd); r != "" {
				return r
			}
		}
	}
	roots := slices.Clone(s.cap.Repos()[repoKey])
	sort.Strings(roots)
	for _, r := range roots {
		if isDir(r) {
			return r
		}
	}
	return ""
}

// explainNoRepo adds what the person can do about ErrNoRepo.
func explainNoRepo(err error, repoKey string) error {
	if !errors.Is(err, capture.ErrNoRepo) {
		return err
	}
	if repoKey == "" {
		return fmt.Errorf("%w: pick a repo, or give an absolute path", err)
	}
	return fmt.Errorf("%w: no worktree of %s is on this device", err, repoKey)
}

// worktreeRoot is the nearest directory at or above dir holding a .git
// entry (a directory, or the file a linked worktree or submodule has), or "".
func worktreeRoot(dir string) string {
	dir = filepath.Clean(dir)
	for {
		if _, err := os.Stat(filepath.Join(dir, ".git")); err == nil {
			return dir
		}
		up := filepath.Dir(dir)
		if up == dir {
			return ""
		}
		dir = up
	}
}

func isDir(p string) bool {
	fi, err := os.Stat(p)
	return err == nil && fi.IsDir()
}

// captureBatch collects changes between two emits.
type captureBatch struct {
	kinds, keys, repos []string
}

func (b *captureBatch) add(ch capture.Change) {
	addOnce(&b.kinds, string(ch.Kind))
	addOnce(&b.keys, ch.Key)
	addOnce(&b.repos, ch.RepoKey)
}

func (b *captureBatch) empty() bool { return len(b.kinds) == 0 }

// take returns the batch as a payload and resets it.
func (b *captureBatch) take() CaptureChange {
	c := CaptureChange{Kinds: nonNil(b.kinds), Keys: nonNil(b.keys), RepoKeys: nonNil(b.repos)}
	*b = captureBatch{}
	return c
}

// drain adds the changes already waiting on ch without blocking.
func drain(ch <-chan capture.Change, b *captureBatch) {
	for {
		select {
		case c, ok := <-ch:
			if !ok {
				return
			}
			b.add(c)
		default:
			return
		}
	}
}

func addOnce(list *[]string, v string) {
	if v != "" && !slices.Contains(*list, v) {
		*list = append(*list, v)
	}
}

// pump turns capture changes into capture:changed, coalesced so a burst
// becomes one event at most captureEmitEvery after the previous one.
func (s *CaptureService) pump(ctx context.Context, app *application.App) {
	s.run(ctx, s.cap.Subscribe(ctx), func(c CaptureChange) { app.Event.Emit(EventCaptureChanged, c) })
}

func (s *CaptureService) run(ctx context.Context, changes <-chan capture.Change, emit func(CaptureChange)) {
	var batch captureBatch
	var last time.Time
	timer := time.NewTimer(time.Hour)
	timer.Stop()
	pending := false
	for {
		select {
		case <-ctx.Done():
			timer.Stop()
			return
		case ch, ok := <-changes:
			if !ok {
				return
			}
			batch.add(ch)
			if !pending {
				pending = true
				timer.Reset(max(0, captureEmitEvery-time.Since(last)))
			}
		case <-timer.C:
			pending = false
			last = time.Now()
			drain(changes, &batch) // what already arrived rides along
			if !batch.empty() {
				emit(batch.take())
			}
		}
	}
}
