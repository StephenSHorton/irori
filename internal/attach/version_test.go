package attach

import "testing"

func TestModAtLeast(t *testing.T) {
	tests := []struct {
		v    string
		want bool
	}{
		{"", false},
		{"junk", false},
		{".4.0", false},
		{"0.2.0-test", false},
		{"0.3.0", false},
		{"0.3.9", false},
		{"0.3.99999", false},
		{"0.4.0", true},
		{"0.4", true},
		{"0.4.0-rc1", true},
		{"0.4.1", true},
		{"0.10.0", true},
		{"1.0", true},
		{"1", true},
		{"v0.4.0", true},
		{" 0.4.0 ", true},
		{"0.4-rc1.9", true},
		{"0-rc.9.9", false},
	}
	for _, tc := range tests {
		if got := ModAtLeast(tc.v, AppendSince); got != tc.want {
			t.Errorf("ModAtLeast(%q, %q) = %v, want %v", tc.v, AppendSince, got, tc.want)
		}
	}
	if !ModAtLeast("0.0.1", "") || ModAtLeast("", "") {
		t.Error("empty min")
	}
}
