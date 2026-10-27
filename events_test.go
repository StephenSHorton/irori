package main

import (
	"bytes"
	"encoding/json"
	"slices"
	"strings"
	"testing"
	"time"

	"github.com/StephenSHorton/irori/internal/hlc"
	"github.com/StephenSHorton/irori/internal/store"
)

// at is the HLC for a wall time (logical 0).
func at(t time.Time) hlc.Timestamp { return hlc.Timestamp(uint64(t.UnixMilli()) << 16) }

func TestFormatEvent(t *testing.T) {
	when := time.Date(2026, 10, 27, 9, 41, 5, 0, time.UTC)
	long := strings.Repeat("x", 300)
	for _, tc := range []struct {
		name string
		ev   store.Event
		want string
	}{
		{
			"status with priority keys first",
			store.Event{DeviceID: "dev1", Seq: 3, Actor: "irori", Kind: "agent.status",
				Payload: json.RawMessage(`{"status":"busy","name":"fox","key":"dev1/s1","zeta":1}`)},
			"10-27 09:41:05 agent.status     key=dev1/s1 name=fox status=busy zeta=1",
		},
		{
			"other device, actor, nested fields left out",
			store.Event{DeviceID: "dev2", Seq: 1, Actor: "user", Kind: "agent.gone",
				Payload: json.RawMessage(`{"key":"dev2/s9","reason":"pid gone","card":{"a":1},"tags":["x"],"ok":true}`)},
			`10-27 09:41:05 agent.gone       [dev2] key=dev2/s9 reason="pid gone" ok=true by=user`,
		},
		{
			"empty payload",
			store.Event{DeviceID: "dev1", Seq: 1, Actor: "irori", Kind: "device.started", Payload: json.RawMessage(`{}`)},
			"10-27 09:41:05 device.started",
		},
		{
			"long kind and non-object payload",
			store.Event{DeviceID: "dev1", Seq: 1, Kind: "capture.commit.seen.long", Payload: json.RawMessage(`"hi\nthere"`)},
			`10-27 09:41:05 capture.commit.seen.long "hi\nthere"`,
		},
		{
			"long value is capped",
			store.Event{DeviceID: "dev1", Seq: 1, Kind: "x", Payload: json.RawMessage(`{"name":"` + long + `"}`)},
			"10-27 09:41:05 x                name=" + strings.Repeat("x", 120),
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			tc.ev.HLC = at(when)
			if got := formatEvent(tc.ev, "dev1", time.UTC); got != tc.want {
				t.Fatalf("got  %q\nwant %q", got, tc.want)
			}
		})
	}
}

func TestSummarizeCap(t *testing.T) {
	m := map[string]string{}
	for _, k := range []string{"a", "b", "c", "d", "e", "f", "g"} {
		m[k] = strings.Repeat(k, 40)
	}
	raw, _ := json.Marshal(m)
	s := summarize(raw)
	if r := []rune(s); len(r) != maxSummary || !strings.HasSuffix(s, "…") {
		t.Fatalf("summary is %d runes (%q), want %d ending in …", len(r), s, maxSummary)
	}
}

func TestEventPrinter(t *testing.T) {
	evs := []store.Event{
		{DeviceID: "dev1", Seq: 4, Kind: "agent.seen", Payload: json.RawMessage(`{"key":"dev1/s1"}`)},
		{DeviceID: "dev2", Seq: 90, Kind: "agent.seen", Payload: json.RawMessage(`{}`)},
		{DeviceID: "dev1", Seq: 2, Kind: "agent.seen", Payload: json.RawMessage(`{}`)},
	}
	for _, tc := range []struct {
		name  string
		json  bool
		lines func([]string) bool
	}{
		{"text", false, func(l []string) bool {
			return len(l) == 4 && strings.Contains(l[0], "key=dev1/s1") && strings.Contains(l[1], "[dev2]") && strings.HasPrefix(l[3], "--")
		}},
		{"json", true, func(l []string) bool {
			if len(l) != 4 || l[3] != `{"resync":true}` {
				return false
			}
			var ev store.Event
			return json.Unmarshal([]byte(l[0]), &ev) == nil && ev.Seq == 4 && ev.DeviceID == "dev1"
		}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			var buf bytes.Buffer
			p := &eventPrinter{w: &buf, json: tc.json, self: "dev1"}
			for _, ev := range evs {
				if err := p.event(ev); err != nil {
					t.Fatal(err)
				}
			}
			if err := p.resync(); err != nil {
				t.Fatal(err)
			}
			if p.lastSelf != 4 {
				t.Fatalf("lastSelf = %d, want 4 (this device's newest)", p.lastSelf)
			}
			lines := strings.Split(strings.TrimSuffix(buf.String(), "\n"), "\n")
			if !tc.lines(lines) {
				t.Fatalf("output:\n%s", buf.String())
			}
		})
	}
}

func TestSplitKinds(t *testing.T) {
	if got, want := splitKinds("agent., device.started,,"), []string{"agent.", "device.started"}; !slices.Equal(got, want) {
		t.Fatalf("splitKinds = %q, want %q", got, want)
	}
	if got := splitKinds(""); got != nil {
		t.Fatalf("splitKinds(\"\") = %q, want nil", got)
	}
}
