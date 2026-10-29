package coord

import (
	"errors"
	"testing"
)

func TestParseLink(t *testing.T) {
	tests := []struct {
		in    string
		kind  LinkKind
		ref   string
		label string
		bad   bool
	}{
		{in: "https://linear.app/acme/issue/abc-123/fix-the-thing", kind: LinkURL, label: "Linear ABC-123"},
		{in: "https://linear.app/acme/project/x", kind: LinkURL, label: "Linear"},
		{in: "https://github.com/StephenSHorton/irori/issues/57", kind: LinkURL, label: "GitHub #57"},
		{in: "https://github.com/StephenSHorton/irori/pull/12/files", kind: LinkURL, label: "GitHub #12"},
		{in: "https://github.com/StephenSHorton/irori", kind: LinkURL, label: "GitHub"},
		{in: "https://www.notion.so/acme/Board-design-0123456789abcdef0123456789abcdef", kind: LinkURL, label: "Notion Board design"},
		{in: "https://www.notion.so/0123456789abcdef0123456789abcdef", kind: LinkURL, label: "Notion"},
		{in: "https://acme.notion.site/Plan-0123456789abcdef0123456789abcdef", kind: LinkURL, label: "Notion Plan"},
		{in: "https://www.example.com/a/b", kind: LinkURL, label: "example.com"},
		{in: "ABCDEF1234", kind: LinkCommit, ref: "abcdef1234", label: "commit abcdef1"},
		{in: `docs\design\board.md`, kind: LinkPath, ref: "docs/design/board.md", label: ".../design/board.md"},
		{in: "README.md", kind: LinkPath, label: "README.md"},
		{in: "C:/work/notes.txt", kind: LinkPath, label: ".../work/notes.txt"},
		{in: "ftp://example.com/x", bad: true},
		{in: "javascript:alert(1)", bad: true},
		{in: "mailto:x@example.com", bad: true},
		{in: "data:text/html,<b>x</b>", bad: true},
		{in: "https:example.com", bad: true},
		{in: "https://github.com@evil.com/x", bad: true},
		{in: "https://user:pw@example.com/", bad: true},
		{in: "https://", bad: true},
		{in: "../secrets", bad: true},
		{in: "", bad: true},
		{in: "a\nb", bad: true},
	}
	for _, tc := range tests {
		l, err := ParseLink(tc.in)
		if tc.bad {
			if !errors.Is(err, ErrInvalid) {
				t.Errorf("ParseLink(%q) = %+v, %v; want ErrInvalid", tc.in, l, err)
			}
			continue
		}
		ref := tc.ref
		if ref == "" {
			ref = tc.in
		}
		if err != nil || l.Kind != tc.kind || l.Ref != ref || l.Label != tc.label {
			t.Errorf("ParseLink(%q) = %+v, %v; want {%s %s %s}", tc.in, l, err, tc.kind, ref, tc.label)
		}
	}
}
