package mcpserver

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"slices"

	"github.com/StephenSHorton/irori/internal/tools"
)

// fallbackJSON is a copy of what GET /v1/tools?client=grok answers, used when
// the Core isn't reachable at start so the tools still exist (and answer
// NotRunningText). Keep it in sync with internal/tools; TestFallbackMatchesTools
// checks it once the tools package serves specs.
//
//go:embed fallback_tools.json
var fallbackJSON []byte

// FallbackSpecs returns the embedded grok tool specs.
func FallbackSpecs() []tools.Spec {
	specs, err := parseSpecs(fallbackJSON)
	if err != nil {
		panic(fmt.Sprintf("mcpserver: bad fallback_tools.json: %v", err))
	}
	return specs
}

// parseSpecs decodes and checks a spec list.
func parseSpecs(data []byte) ([]tools.Spec, error) {
	var specs []tools.Spec
	if err := json.Unmarshal(data, &specs); err != nil {
		return nil, err
	}
	for _, s := range specs {
		if err := checkSpec(s); err != nil {
			return nil, err
		}
	}
	return specs, nil
}

// checkSpec rejects a spec the SDK would panic on or a client would refuse.
func checkSpec(s tools.Spec) error {
	if !validToolName(s.Name) {
		return fmt.Errorf("tool name %q", s.Name)
	}
	var schema map[string]any
	if err := json.Unmarshal(s.InputSchema, &schema); err != nil {
		return fmt.Errorf("tool %s: input schema: %v", s.Name, err)
	}
	if schema["type"] != "object" {
		return fmt.Errorf("tool %s: input schema type %v, want object", s.Name, schema["type"])
	}
	return nil
}

// validToolName allows what every client accepts once prefixed with irori_:
// lowercase letters, digits and _ (the go-sdk and Claude Code allow more).
func validToolName(name string) bool {
	if name == "" || len(name) > 40 {
		return false
	}
	for _, r := range name {
		if !(r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '_') {
			return false
		}
	}
	return true
}

// forGrok keeps the specs a Grok client gets, valid ones only, and makes sure
// join is there (irori mcp implements it itself).
func forGrok(specs []tools.Spec, log func(msg string, args ...any)) []tools.Spec {
	var out []tools.Spec
	seen := map[string]bool{}
	for _, s := range specs {
		if len(s.Clients) > 0 && !slices.Contains(s.Clients, "grok") {
			continue
		}
		if err := checkSpec(s); err != nil {
			log("skipping tool spec", "error", err)
			continue
		}
		if seen[s.Name] {
			continue
		}
		seen[s.Name] = true
		out = append(out, s)
	}
	if !seen[toolJoin] {
		for _, s := range FallbackSpecs() {
			if s.Name == toolJoin {
				out = append(out, s)
			}
		}
	}
	return out
}
