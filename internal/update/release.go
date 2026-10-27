package update

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"
)

type ghAsset struct {
	Name string `json:"name"`
	URL  string `json:"browser_download_url"`
}

type ghRelease struct {
	TagName     string    `json:"tag_name"`
	Body        string    `json:"body"`
	PublishedAt string    `json:"published_at"`
	Assets      []ghAsset `json:"assets"`
}

// latestRelease fetches the newest published, non-prerelease release
// (GitHub's /releases/latest never returns drafts or prereleases). It returns
// nil when the repo has no releases yet.
func (s *UpdateService) latestRelease(ctx context.Context) (*ghRelease, error) {
	url := fmt.Sprintf("%s/repos/%s/releases/latest", s.apiBase, s.repo)
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "application/vnd.github+json")
	resp, err := s.client.Do(req)
	if err != nil {
		return nil, err
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode == http.StatusNotFound {
		return nil, nil
	}
	if resp.StatusCode != http.StatusOK {
		b, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return nil, fmt.Errorf("github releases: %s: %s", resp.Status, strings.TrimSpace(string(b)))
	}
	var rel ghRelease
	if err := json.NewDecoder(resp.Body).Decode(&rel); err != nil {
		return nil, err
	}
	return &rel, nil
}

// findUpdate returns the Windows installer update the latest release offers,
// or nil when the running version is current (or a dev build, which never
// touches the network). The release must carry SHA256SUMS with a line for the
// installer: an unverifiable installer is never offered.
func (s *UpdateService) findUpdate() (*UpdateInfo, error) {
	if isDev(s.current) {
		return nil, nil
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	rel, err := s.latestRelease(ctx)
	if err != nil || rel == nil {
		return nil, err
	}
	if !isNewer(rel.TagName, s.current) {
		return nil, nil
	}

	var installer ghAsset
	var sumsURL string
	for _, a := range rel.Assets {
		if isInstallerAsset(a.Name) {
			installer = a
		}
		if isChecksumAsset(a.Name) {
			sumsURL = a.URL
		}
	}
	if installer.URL == "" {
		return nil, fmt.Errorf("release %s has no windows installer asset", rel.TagName)
	}
	if sumsURL == "" {
		return nil, fmt.Errorf("release %s has no SHA256SUMS asset", rel.TagName)
	}
	sum, err := s.fetchChecksum(ctx, sumsURL, installer.Name)
	if err != nil {
		return nil, err
	}

	return &UpdateInfo{
		Version:     strings.TrimPrefix(rel.TagName, "v"),
		Notes:       rel.Body,
		AssetURL:    installer.URL,
		AssetName:   installer.Name,
		SHA256:      sum,
		PublishedAt: rel.PublishedAt,
	}, nil
}

// isInstallerAsset reports whether a release asset is the Windows NSIS
// installer. release.yml names it irori-<version>-windows-<arch>-installer.exe;
// TestReleaseWorkflowAssetNames keeps the two in step.
func isInstallerAsset(name string) bool {
	n := strings.ToLower(name)
	return strings.HasSuffix(n, "-installer.exe") && strings.Contains(n, "windows")
}

// isChecksumAsset reports whether a release asset is the SHA256SUMS file.
func isChecksumAsset(name string) bool { return strings.EqualFold(name, "SHA256SUMS") }

// fetchChecksum reads a SHA256SUMS file ("<hex>  <name>" lines, an optional
// '*' before the name) and returns the hash listed for name.
func (s *UpdateService) fetchChecksum(ctx context.Context, url, name string) (string, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return "", err
	}
	resp, err := s.client.Do(req)
	if err != nil {
		return "", err
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("download SHA256SUMS: %s", resp.Status)
	}
	b, err := io.ReadAll(io.LimitReader(resp.Body, 1<<20))
	if err != nil {
		return "", err
	}
	for line := range strings.SplitSeq(string(b), "\n") {
		fields := strings.Fields(line)
		if len(fields) == 2 && filepath.Base(strings.TrimPrefix(fields[1], "*")) == name {
			return strings.ToLower(fields[0]), nil
		}
	}
	return "", fmt.Errorf("SHA256SUMS has no line for %s", name)
}

// downloadVerified downloads the installer into the update folder and checks
// its SHA256. It returns the installer path; a mismatch deletes the file.
func (s *UpdateService) downloadVerified(info UpdateInfo) (string, error) {
	if info.AssetName == "" || info.AssetURL == "" {
		return "", errors.New("update has no installer asset")
	}
	if info.SHA256 == "" {
		return "", errors.New("update has no checksum")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()

	if err := os.MkdirAll(s.dir, 0o755); err != nil {
		return "", err
	}
	// The asset name comes from the network: keep only its base name.
	dst := filepath.Join(s.dir, filepath.Base(info.AssetName))
	// Installers from earlier updates have done their job; drop them.
	if old, err := os.ReadDir(s.dir); err == nil {
		for _, e := range old {
			if p := filepath.Join(s.dir, e.Name()); p != dst && !e.IsDir() {
				_ = os.Remove(p)
			}
		}
	}
	if err := s.download(ctx, info.AssetURL, dst); err != nil {
		_ = os.Remove(dst)
		return "", err
	}
	got, err := sha256File(dst)
	if err != nil {
		return "", err
	}
	if !strings.EqualFold(got, info.SHA256) {
		_ = os.Remove(dst)
		return "", fmt.Errorf("update checksum mismatch: got %s, want %s", got, info.SHA256)
	}
	return dst, nil
}

func (s *UpdateService) download(ctx context.Context, url, dst string) (err error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return err
	}
	// The API client's 30 s timeout would cut a large download short; the
	// context bounds it instead.
	client := *s.client
	client.Timeout = 0
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer func() { _ = resp.Body.Close() }()
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("download %s: %s", url, resp.Status)
	}
	f, err := os.Create(dst)
	if err != nil {
		return err
	}
	defer func() {
		if cerr := f.Close(); cerr != nil && err == nil {
			err = cerr
		}
	}()
	_, err = io.Copy(f, resp.Body)
	return err
}

func sha256File(p string) (string, error) {
	f, err := os.Open(p)
	if err != nil {
		return "", err
	}
	defer func() { _ = f.Close() }()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}
