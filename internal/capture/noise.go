package capture

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"

	"github.com/StephenSHorton/irori/internal/capture/transcript"
)

// Heuristic (Bash/PowerShell bashEditDiff) edits are mostly build output: a
// `dotnet build` reports every .dll it wrote. Before a heuristic edit is
// logged it must name a file inside a known repo that git doesn't ignore and
// whose extension isn't a binary or build artifact, and it must add or remove
// a line (a shell call that only touched a file reports +0-0); at most maxShellEdits of
// one shell call's edits survive. When git can't say which repo owns a file
// (see noiseRoot) the edit is kept. Exact edits (Edit, Write, NotebookEdit)
// are never filtered.
const (
	// maxShellEdits caps the heuristic edits kept per shell call.
	maxShellEdits = 50
	// ignoreCacheMax bounds the check-ignore cache (root+path entries); it
	// is cleared when full.
	ignoreCacheMax = 50000
	// ignoreArgBytes bounds the paths passed to one check-ignore run, well
	// under Windows' 32K command line.
	ignoreArgBytes = 8000
	// noiseTurnsMax bounds the per-turn dropped-path sets kept for open
	// turns; they are cleared when full.
	noiseTurnsMax = 4096
)

// artifactExts are extensions of binaries and build artifacts.
var artifactExts = map[string]bool{}

func init() {
	for _, e := range strings.Fields(`
		.dll .exe .pdb .obj .o .a .lib .so .dylib .exp .ilk .idb .ipdb .iobj .pch .nupkg .snupkg
		.class .jar .war .pyc .pyo .wasm .node .bin .dat .cache .tlog
		.png .jpg .jpeg .gif .webp .ico .bmp .tga .dds .psd .tif .tiff .exr .hdr .ktx
		.mp3 .mp4 .wav .ogg .webm .mov .avi .flac
		.zip .7z .gz .tgz .tar .bz2 .xz .rar .zst .cab .msi
		.ttf .otf .woff .woff2
		.vpk .vmdl_c .vmat_c .vtex_c .vsnd_c .vpcf_c .vmap_c .vanmgrph_c .vagrp_c .vsvg_c .vxml_c .shader_c .vjs_c .vcss_c .sbproj_c
		.rel .dol .iso .rvz .sav
	`) {
		artifactExts[e] = true
	}
}

// artifactDirs are path segments whose contents are never anyone's edit.
var artifactDirs = map[string]bool{".git": true, "node_modules": true}

// Dropped counts heuristic edits capture discarded as noise.
type Dropped struct {
	Artifact    int `json:"artifact"`    // binary or build-artifact extension, or under .git/node_modules
	OutsideRepo int `json:"outsideRepo"` // not inside a known git worktree
	Ignored     int `json:"ignored"`     // git check-ignore says the path is ignored
	OverCap     int `json:"overCap"`     // past maxShellEdits in one shell call
	Unchanged   int `json:"unchanged"`   // no lines added or removed (a shell call touched the file)
}

// isArtifact reports a path capture never keeps a heuristic edit for.
func isArtifact(path string) bool {
	p := strings.ToLower(filepath.ToSlash(path))
	if artifactExts[pathExt(p)] {
		return true
	}
	for _, seg := range strings.Split(p, "/") {
		if artifactDirs[seg] {
			return true
		}
	}
	return false
}

// pathExt is the extension of a forward-slash path ("" when none).
func pathExt(p string) string {
	base := p[strings.LastIndexByte(p, '/')+1:]
	if i := strings.LastIndexByte(base, '.'); i > 0 {
		return base[i:]
	}
	return ""
}

// ignoreCache remembers check-ignore answers per root and path.
type ignoreCache struct {
	mu sync.Mutex
	m  map[string]bool
}

func ignoreKey(root, rel string) string { return normRoot(root) + "|" + strings.ToLower(rel) }

func (ic *ignoreCache) get(root, rel string) (ignored, ok bool) {
	ic.mu.Lock()
	defer ic.mu.Unlock()
	ignored, ok = ic.m[ignoreKey(root, rel)]
	return ignored, ok
}

func (ic *ignoreCache) put(root, rel string, ignored bool) {
	ic.mu.Lock()
	defer ic.mu.Unlock()
	if ic.m == nil || len(ic.m) >= ignoreCacheMax {
		ic.m = map[string]bool{}
	}
	ic.m[ignoreKey(root, rel)] = ignored
}

// ignored asks git which of rels (repo-relative, forward slashes) root
// ignores, consulting and filling the cache. Tracked files are never
// reported (no --no-index). A failed run answers "not ignored" for its
// batch and caches nothing: losing a real edit is worse than keeping noise.
func (c *capture) ignored(ctx context.Context, root string, rels []string) map[string]bool {
	out := map[string]bool{}
	var ask []string
	seen := map[string]bool{}
	for _, rel := range rels {
		k := strings.ToLower(rel)
		if seen[k] {
			continue
		}
		seen[k] = true
		if ig, ok := c.ignores.get(root, rel); ok {
			if ig {
				out[k] = true
			}
			continue
		}
		ask = append(ask, rel)
	}
	for len(ask) > 0 {
		n, size := 0, 0
		for n < len(ask) && (n == 0 || size+len(ask[n])+1 <= ignoreArgBytes) {
			size += len(ask[n]) + 1
			n++
		}
		batch := ask[:n]
		ask = ask[n:]
		hits, ok := c.checkIgnore(ctx, root, batch)
		if !ok {
			continue
		}
		for _, rel := range batch {
			ig := hits[strings.ToLower(rel)]
			c.ignores.put(root, rel, ig)
			if ig {
				out[strings.ToLower(rel)] = true
			}
		}
	}
	return out
}

// checkIgnore runs one `git check-ignore` over batch (-z needs --stdin,
// which gitlink.Git can't feed; a name git still quotes misses the match
// and is kept). Exit 1 means none is ignored; any other failure reports !ok.
func (c *capture) checkIgnore(ctx context.Context, root string, batch []string) (map[string]bool, bool) {
	cctx, cancel := context.WithTimeout(ctx, gitTimeout)
	defer cancel()
	args := append([]string{"-c", "core.quotePath=false", "check-ignore", "--"}, batch...)
	stdout, err := c.git.Run(cctx, root, args...)
	if err != nil {
		var ee *exec.ExitError
		if errors.As(err, &ee) && ee.ExitCode() == 1 && cctx.Err() == nil {
			return map[string]bool{}, true
		}
		return nil, false
	}
	hits := map[string]bool{}
	for _, p := range strings.Split(stdout, "\n") {
		if p = strings.TrimRight(p, "\r"); p != "" {
			hits[strings.ToLower(filepath.ToSlash(p))] = true
		}
	}
	return hits, true
}

// heuristicCand is one heuristic edit under consideration.
type heuristicCand struct {
	idx       int
	root, rel string
}

// dropNoise returns the indexes of items' heuristic edits to discard, counts
// them in c.dropped, and remembers each dropped path against its turn so the
// turn's EditPaths lose it too.
func (c *capture) dropNoise(ctx context.Context, f *tfile, items []transcript.Item) map[int]bool {
	var drop map[int]bool
	var stats Dropped
	byRoot := map[string][]heuristicCand{}
	var order []string // roots in first-seen order
	mark := func(i int) {
		if drop == nil {
			drop = map[int]bool{}
		}
		drop[i] = true
	}
	for i, it := range items {
		e := it.Edit
		if e == nil || e.Fidelity != transcript.Heuristic {
			continue
		}
		abs := editAbs(e)
		if isArtifact(abs) {
			stats.Artifact++
			mark(i)
			continue
		}
		if e.Adds == 0 && e.Dels == 0 {
			stats.Unchanged++
			mark(i)
			continue
		}
		root, unsure := c.noiseRoot(ctx, e.Cwd, abs)
		if unsure {
			continue
		}
		rel, ok := relTo(root, abs)
		if !ok {
			stats.OutsideRepo++
			mark(i)
			continue
		}
		n := normRoot(root)
		if _, ok := byRoot[n]; !ok {
			order = append(order, n)
		}
		byRoot[n] = append(byRoot[n], heuristicCand{idx: i, root: root, rel: rel})
	}
	for _, n := range order {
		cands := byRoot[n]
		rels := make([]string, len(cands))
		for j, cd := range cands {
			rels[j] = cd.rel
		}
		ig := c.ignored(ctx, cands[0].root, rels)
		for _, cd := range cands {
			if ig[strings.ToLower(cd.rel)] {
				stats.Ignored++
				mark(cd.idx)
			}
		}
	}
	// Cap what survived, per shell call, in transcript order.
	perCall := map[string]int{}
	for i, it := range items {
		e := it.Edit
		if e == nil || e.Fidelity != transcript.Heuristic || drop[i] {
			continue
		}
		perCall[e.ToolUseID]++
		if perCall[e.ToolUseID] > maxShellEdits {
			stats.OverCap++
			mark(i)
		}
	}
	if drop == nil {
		return nil
	}
	c.mu.Lock()
	c.dropped.Artifact += stats.Artifact
	c.dropped.OutsideRepo += stats.OutsideRepo
	c.dropped.Ignored += stats.Ignored
	c.dropped.OverCap += stats.OverCap
	c.dropped.Unchanged += stats.Unchanged
	c.mu.Unlock()
	return drop
}

// noiseRoot is the worktree root that judges a heuristic edit of abs ("",
// !unsure: no repo holds it). unsure reports an edit whose owner can't be
// known, which is kept unfiltered (losing a real edit is worse than keeping
// noise): a .git sits above it but rev-parse failed, or it lies inside a
// session cwd that is gone. A gone cwd was most likely an agent worktree,
// since deleted, whose own repo can't be asked; the nearest existing
// ancestor is the parent repo, which ignores its worktrees (.claude/).
func (c *capture) noiseRoot(ctx context.Context, cwd, abs string) (root string, unsure bool) {
	if !filepath.IsAbs(abs) || !underGit(abs) {
		return "", false
	}
	if cwd != "" && filepath.IsAbs(cwd) && !dirExists(cwd) {
		if _, in := relTo(cwd, abs); in {
			return "", true
		}
	}
	root = c.roots.forFile(ctx, abs)
	return root, root == ""
}

// underGit reports a .git entry (a repo's directory or a worktree's file)
// in some directory above path, existing or not.
func underGit(path string) bool {
	for d := filepath.Dir(path); ; {
		if _, err := os.Lstat(filepath.Join(d, ".git")); err == nil {
			return true
		}
		up := filepath.Dir(d)
		if up == d {
			return false
		}
		d = up
	}
}

// notePath records that a turn dropped (noisy) or kept an edit for path.
// Kept wins: a path some edit of the turn logged stays in EditPaths. Only
// turns that dropped something are tracked; onTurn forgets the turn.
func (c *capture) notePath(key string, e *transcript.Edit, noisy bool) {
	tk := turnKey(key, e.AgentID, e.TurnID)
	pk := editPathKey(e.Path)
	c.mu.Lock()
	defer c.mu.Unlock()
	s := c.noisePaths[tk]
	if s == nil {
		if !noisy {
			return // the turn has no noise yet; its logged edits seed the set below
		}
		if len(c.noisePaths) >= noiseTurnsMax {
			c.noisePaths = map[string]map[string]bool{}
		}
		s = map[string]bool{}
		for _, ed := range c.editsByTurn[tk] {
			s[editPathKey(rowAbs(ed))] = false
		}
		c.noisePaths[tk] = s
	}
	if noisy {
		if _, ok := s[pk]; !ok {
			s[pk] = true
		}
		return
	}
	s[pk] = false
}

// stripNoise removes dropped heuristic paths from a turn's EditPaths; c.mu
// is held (read or write).
func (c *capture) stripNoise(key string, t transcript.Turn) []string {
	s := c.noisePaths[turnKey(key, t.AgentID, t.ID)]
	if len(s) == 0 {
		return t.EditPaths
	}
	var out []string
	for _, p := range t.EditPaths {
		if !s[editPathKey(p)] {
			out = append(out, p)
		}
	}
	return out
}

// editAbs is an edit's absolute, cleaned path (as onEdit computes it).
func editAbs(e *transcript.Edit) string {
	abs := e.Path
	if !filepath.IsAbs(abs) && e.Cwd != "" {
		abs = filepath.Join(e.Cwd, abs)
	}
	return filepath.Clean(filepath.FromSlash(abs))
}

// rowAbs is a logged edit's absolute path.
func rowAbs(e *EditRow) string {
	if e.Root != "" {
		return filepath.Join(e.Root, filepath.FromSlash(e.Path))
	}
	return e.Path
}

// editPathKey compares an edit's path with the turn's EditPaths entries.
func editPathKey(p string) string {
	return strings.ToLower(filepath.ToSlash(filepath.Clean(filepath.FromSlash(p))))
}
