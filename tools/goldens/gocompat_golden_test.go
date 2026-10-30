// Golden exporter for crates/gocompat (architecture.md §1.2, §6.2).
//
// This file is not part of the Go repo. tools/goldens/export.sh injects it
// into I:internal/hlc with `go test -overlay`, so the Go tree on disk never
// changes. It only runs when IRORI_GOLDEN_OUT is set, and writes:
//
//	$IRORI_GOLDEN_OUT/gocompat/*.json   Go-produced vectors
//	$IRORI_TABLES_OUT                   crates/gocompat/src/text/tables.rs
//
// Byte strings that may hold invalid UTF-8 are written as lowercase hex.
package hlc_test

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"
	"unicode"

	"github.com/StephenSHorton/irori/internal/identity"
)

func goldenDir(t *testing.T) string {
	dir := os.Getenv("IRORI_GOLDEN_OUT")
	if dir == "" {
		t.Skip("IRORI_GOLDEN_OUT not set")
	}
	return dir
}

func writeGolden(t *testing.T, rel string, v any) {
	path := filepath.Join(goldenDir(t), rel)
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	if err := enc.Encode(v); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

func hx(s string) string { return hex.EncodeToString([]byte(s)) }

// stringInputs covers ASCII, every escape class, HTML characters, the line
// separators, and the invalid UTF-8 shapes where Go and Rust decoders differ.
func stringInputs() []string {
	var all []byte
	for b := 0; b < 0x80; b++ {
		all = append(all, byte(b))
	}
	return []string{
		"", "plain", string(all), "a<b>&c", "\u2028\u2029", "x\u2028y",
		"é", "日本語", "😀", "\ufffd", "\u007f", "\u0080\u009f\u00a0\u00ad",
		"\x80", "\xff", "\xc3", "\xc3(", "\xe2\x80", "\xe2\x80\xa8", "\xed\xa0\x80",
		"\xf0\x9f\x98", "\xf0\x9f\x98\x80", "\xf4\x90\x80\x80", "\xc0\xaf", "\xe0\x80\xaf",
		"a\xffb\xfe", "\x00\x01\x1f", "tab\there \"q\" é \x01", "\\", "\"",
		"\u200b\u200e\ufeff", "\U000e0001", "\U0010ffff", "\u0378", "\u00a0",
		"<irori-message from=\"x\">&amp;</irori-message>", "\u0085", "\u2066",
		"\U0001f9ff", "\U0001fae9", "\u0b55", "\U00031350",
	}
}

func TestGoldenValid(t *testing.T) {
	type c struct {
		Hex string `json:"hex"`
		OK  bool   `json:"ok"`
	}
	type deep struct {
		Depth int  `json:"depth"`
		Obj   bool `json:"obj"`
		OK    bool `json:"ok"`
	}
	ins := []string{
		``, ` `, `{}`, `[]`, ` 1 `, `0`, `01`, `-0`, `-`, `-01`, `1.`, `1.5e+3`, `1E5`, `1e`, `1e+`,
		`.5`, `0.0e-0`, `+1`, `1.5E-03`, `"`, `""`, `"\/"`, `"\a"`, `"\u00"`, `"\u12G4"`, `"\uD800"`,
		`"\u0041"`, "\"\x80\"", "\"\xff\xfe\"", "\"\x1f\"", "\"\x7f\"", "\"\t\"", "\"\xed\xa0\x80\"",
		`{"a":1,}`, `[1,]`, `nul`, `null`, `null `, "\ufeff{}", `{} {}`, "\v1", "\f1", "\u00a01",
		"\r\n\t 1", `tru`, `true`, `true1`, `false`, `fals`, `1 2`, `{"a" 1}`, `{1:2}`, `[1 2]`,
		`[[[]]]`, `{"a":{"b":[1,{"c":null}]}}`, `{"a":1 "b":2}`, `[,1]`, `{,}`, `"a"x`, `1x`,
		`[1]]`, `{"a":1}}`, `"\u00e9\n"`, `-1.0e-10`, `00`, `1.e5`, `-.5`, `1e5.5`, "\x00",
		`{"":""}`, `{"a":[]}`, `[true,false,null]`, `123456789012345678901234567890`,
	}
	var cases []c
	for _, s := range ins {
		cases = append(cases, c{hx(s), json.Valid([]byte(s))})
	}
	var deeps []deep
	for _, n := range []int{1, 9999, 10000, 10001, 20000} {
		for _, obj := range []bool{false, true} {
			var b strings.Builder
			for i := 0; i < n; i++ {
				if obj && i > 0 {
					b.WriteString(`{"a":`)
				} else if obj {
					b.WriteString(`{"a":`)
				} else {
					b.WriteString("[")
				}
			}
			if obj {
				b.WriteString("1")
			}
			for i := 0; i < n; i++ {
				if obj {
					b.WriteString("}")
				} else {
					b.WriteString("]")
				}
			}
			deeps = append(deeps, deep{n, obj, json.Valid([]byte(b.String()))})
		}
	}
	writeGolden(t, "gocompat/valid.json", map[string]any{"cases": cases, "deep": deeps})
}

// marshalStruct exercises the struct-level Marshal rules. The Rust test
// builds the same value with serde attributes and compares bytes.
type mInner struct {
	X int    `json:"x"`
	Y string `json:"y,omitempty"`
}

type mStruct struct {
	Name     string            `json:"name"`
	Empty    string            `json:"empty,omitempty"`
	Zero     int               `json:"zero,omitempty"`
	Kept     int               `json:"kept"`
	False    bool              `json:"false,omitempty"`
	True     bool              `json:"true"`
	NilSlice []string          `json:"nilSlice"`
	NilOmit  []string          `json:"nilOmit,omitempty"`
	EmptySl  []string          `json:"emptySlice"`
	NilMap   map[string]int    `json:"nilMap"`
	Map      map[string]int    `json:"map"`
	IntMap   map[int]string    `json:"intMap"`
	Bytes    []byte            `json:"bytes"`
	NilBytes []byte            `json:"nilBytes"`
	EmptyB   []byte            `json:"emptyBytes"`
	Ptr      *mInner           `json:"ptr"`
	PtrOmit  *mInner           `json:"ptrOmit,omitempty"`
	Inner    mInner            `json:"inner"`
	ZeroTime time.Time         `json:"zeroTime,omitempty"`
	OmitZT   time.Time         `json:"omitZeroTime,omitzero"`
	At       time.Time         `json:"at"`
	Local    time.Time         `json:"local"`
	Raw      json.RawMessage   `json:"raw"`
	NilRaw   json.RawMessage   `json:"nilRaw"`
	HTML     string            `json:"html<&>"`
	F64      float64           `json:"f64"`
	F32      float32           `json:"f32"`
	I64      int64             `json:"i64"`
	U64      uint64            `json:"u64"`
	Any      any               `json:"any"`
	AnyMap   map[string]any    `json:"anyMap"`
	Strs     map[string]string `json:"strs"`
}

func marshalValue() mStruct {
	return mStruct{
		Name:    "n<1>",
		Kept:    0,
		True:    true,
		EmptySl: []string{},
		Map:     map[string]int{"b": 2, "a": 1, "B": 3, "é": 4, "": 5, "a\u2028": 6},
		IntMap:  map[int]string{10: "ten", 9: "nine", -1: "neg"},
		Bytes:   []byte("hello\x00\xff"),
		EmptyB:  []byte{},
		Inner:   mInner{X: 7},
		At:      time.Date(2026, 10, 28, 14, 25, 23, 947220900, time.UTC),
		Local:   time.Date(2026, 1, 2, 3, 4, 5, 120000000, time.FixedZone("", -6*3600)),
		Raw:     json.RawMessage(" { \"k\" : [1, 2 ,\"<&>\"] } "),
		HTML:    "a&b",
		F64:     1e21,
		F32:     0.1,
		I64:     math.MinInt64,
		U64:     math.MaxUint64,
		Any:     []any{1.5, "x", nil, true, map[string]any{"z": 1, "a": []any{}}},
		AnyMap:  map[string]any{"hlc": float64(1 << 62)},
		Strs:    map[string]string{"k": "\u2029"},
	}
}

func TestGoldenMarshal(t *testing.T) {
	type sc struct {
		Hex    string `json:"hex"`
		HTML   string `json:"html"`
		NoHTML string `json:"noHtml"`
	}
	var strs []sc
	for _, s := range stringInputs() {
		a, _ := json.Marshal(s)
		var b bytes.Buffer
		enc := json.NewEncoder(&b)
		enc.SetEscapeHTML(false)
		_ = enc.Encode(s)
		strs = append(strs, sc{hx(s), string(a), strings.TrimSuffix(b.String(), "\n")})
	}
	type fc struct {
		Bits string `json:"bits"`
		Out  string `json:"out"`
		Err  bool   `json:"err,omitempty"`
	}
	f64s := []float64{0, math.Copysign(0, -1), 1, -1, 0.1, 0.2 + 0.1, 100, 1e6, 1e20, 1e21, 1.5e21, 1e-6,
		9.99e-7, 1e-7, 1.2345e-7, 123456789.123, 5e-324, math.MaxFloat64, 1<<53 + 1, 1 << 62, 1e100,
		-1.5e-10, 12345678901234567890, 0.000001, 1.0000000000000002, 2.5e-5, 1e-10, -1e21, 3.14159,
		math.NaN(), math.Inf(1), math.Inf(-1)}
	var f64out []fc
	for _, f := range f64s {
		b, err := json.Marshal(f)
		f64out = append(f64out, fc{strconv.FormatUint(math.Float64bits(f), 16), string(b), err != nil})
	}
	f32s := []float32{0, 1, 0.1, 1e21, 1e20, 1e-6, 9.99e-7, 1e-7, 3.4e38, 16777217, 1.17549435e-38,
		1.4e-45, 123.456, float32(math.Inf(1))}
	var f32out []fc
	for _, f := range f32s {
		b, err := json.Marshal(f)
		f32out = append(f32out, fc{strconv.FormatUint(uint64(math.Float32bits(f)), 16), string(b), err != nil})
	}
	v := marshalValue()
	compact, err := json.Marshal(v)
	if err != nil {
		t.Fatal(err)
	}
	indent, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	indentPrefix, _ := json.MarshalIndent(v.Inner, ">", "\t")
	var line bytes.Buffer
	enc := json.NewEncoder(&line)
	enc.SetEscapeHTML(false)
	_ = enc.Encode(v)
	// Indent and Compact over raw bytes.
	raws := []string{` { "a" : [ 1 , { } , [ ] , "x y" ] , "b":{"c":null} } `, `[]`, `"<"`, "{\"a\":\"\u2028\"}"}
	type rc struct {
		In      string `json:"in"`
		Compact string `json:"compact"`
		CompEsc string `json:"compactEscaped"`
		Indent  string `json:"indent"`
	}
	var rcs []rc
	for _, r := range raws {
		var c, i bytes.Buffer
		_ = json.Compact(&c, []byte(r))
		_ = json.Indent(&i, []byte(r), "", "  ")
		esc, _ := json.Marshal(json.RawMessage(r))
		rcs = append(rcs, rc{r, c.String(), string(esc), i.String()})
	}
	_, emptyRawErr := json.Marshal(struct {
		R json.RawMessage `json:"r"`
	}{R: json.RawMessage{}})
	writeGolden(t, "gocompat/marshal.json", map[string]any{
		"strings": strs, "f64": f64out, "f32": f32out,
		"struct": string(compact), "structIndent": string(indent), "innerIndentPrefix": string(indentPrefix),
		"structEncoderNoHtml": line.String(), "raw": rcs, "emptyRawErr": emptyRawErr != nil,
	})
}

func TestGoldenCanonical(t *testing.T) {
	type cc struct {
		Hex string `json:"hex"`
		Out string `json:"out"`
		Err bool   `json:"err,omitempty"`
	}
	ins := []string{
		`{"b":1,"a":{"d":[3,{"z":1,"y":2}],"c":"<&>"}}`,
		`{"n":1.50,"m":-0,"e":1E+2,"big":12345678901234567890123}`,
		"{\"s\":\"\\u003c\\u2028\u2029\\b\\f\\u0001\\u007f\\/\"}",
		`{"a":1,"a":2,"A":3}`,
		"{\"bad\":\"\xff\xc3(\",\"\xfe\":1}",
		`{"sur":"\ud800","pair":"\ud83d\ude00","lone2":"\udc00x","hi_hi":"\ud800\ud800"}`,
		`[]`, `{}`, `null`, `"x"`, `[1,"2",[3]]`, ` { "sp" : true } `,
		`{"é":1,"e":2,"z":3,"Z":4,"":5}`,
	}
	var cases []cc
	for _, s := range ins {
		out, err := identity.Canonical(json.RawMessage(s))
		cases = append(cases, cc{hx(s), string(out), err != nil})
	}
	// identity.md §7.1 ESC: a Go map through Canonical.
	esc, _ := identity.Canonical(map[string]any{
		"big": int64(1 << 62), "m": map[string]int(nil), "n": json.Number("1"), "nil": nil,
		"s": "a<b>&\u2028\u2029\u0001\u0008\u000c\n\t\u007f\"\\é\u2028\u2029",
	})
	// A struct the way identity signs it (Cert at Unix 1, label with '<').
	type cert struct {
		PersonID  string    `json:"personId"`
		DeviceID  string    `json:"deviceId"`
		DevicePub string    `json:"devicePub"`
		Label     string    `json:"label"`
		Role      string    `json:"role"`
		At        time.Time `json:"at"`
	}
	certOut, _ := identity.Canonical(cert{"p", "d", "k", "a<b", "desktop", time.Unix(1, 0).UTC()})
	writeGolden(t, "gocompat/canonical.json", map[string]any{"cases": cases, "esc": string(esc), "cert": string(certOut)})
}

type lInner struct {
	X int    `json:"x"`
	Y string `json:"y"`
}

type lStruct struct {
	Name   string          `json:"name"`
	TaskID string          `json:"taskId"`
	N      int64           `json:"n"`
	U      uint64          `json:"u"`
	I8     int8            `json:"i8"`
	F      float64         `json:"f"`
	F32    float32         `json:"f32"`
	B      bool            `json:"b"`
	S      []string        `json:"s"`
	O      *lInner         `json:"o"`
	V      lInner          `json:"v"`
	Vs     []lInner        `json:"vs"`
	M      map[string]int  `json:"m"`
	A      any             `json:"a"`
	At     time.Time       `json:"at"`
	R      json.RawMessage `json:"r"`
	K      string          `json:"K"`
	KK     string          `json:"k"`
	Sigma  string          `json:"σ"`
}

func TestGoldenLenient(t *testing.T) {
	type lc struct {
		Hex string `json:"hex"`
		OK  bool   `json:"ok"`
		Out string `json:"out,omitempty"`
	}
	ins := []string{
		`{}`, `null`, `[]`, `"x"`, `1`, `true`, ` {"name":"a"} `,
		`{"name":"a","NAME":"b"}`, `{"NAME":"b","name":"a"}`, `{"TASKID":"t1"}`, `{"taskid":"t2","taskId":"t3"}`,
		`{"name":5}`, `{"name":null}`, `{"name":"a","name":null}`, `{"name":5,"name":"ok"}`,
		`{"n":1}`, `{"n":-0}`, `{"n":1.0}`, `{"n":1e2}`, `{"n":9223372036854775807}`, `{"n":9223372036854775808}`,
		`{"n":"1"}`, `{"u":-1}`, `{"u":18446744073709551615}`, `{"u":18446744073709551616}`,
		`{"i8":127}`, `{"i8":128}`, `{"i8":-128}`, `{"f":1e308}`, `{"f":1e309}`, `{"f":-0.0}`, `{"f":1}`,
		`{"f32":3.4e38}`, `{"f32":1e39}`, `{"f32":0.1}`, `{"b":true}`, `{"b":"true"}`, `{"b":1}`, `{"b":null}`,
		`{"s":["a","b"]}`, `{"s":["a",1]}`, `{"s":"x"}`, `{"s":null}`, `{"s":[]}`, `{"s":["a","b"],"s":["c"]}`,
		`{"o":{"x":1}}`, `{"o":{"x":1},"o":null}`, `{"o":{"x":1},"o":{"y":"z"}}`, `{"o":5}`, `{"o":{"x":"bad"}}`,
		`{"v":{"x":1},"v":{"y":"a"}}`, `{"v":null}`, `{"v":{"x":1},"v":null}`, `{"v":[]}`,
		`{"vs":[{"x":1,"y":"a"},{"x":2}],"vs":[{"y":"b"}]}`, `{"vs":[{"x":1}],"vs":[]}`,
		`{"m":{"b":2,"a":1}}`, `{"m":{"a":1},"m":{"b":2}}`, `{"m":{"a":"x"}}`, `{"m":null}`, `{"m":{"a":1,"a":2}}`,
		`{"a":1}`, `{"a":12345678901234567890}`, `{"a":1e400}`, `{"a":{"z":[1,"x",null,{"q":false}]}}`,
		`{"a":"<&>"}`, `{"a":{"k":1,"k":2}}`, `{"a":[1],"a":{"x":1}}`,
		`{"at":"2026-01-02T03:04:05Z"}`, `{"at":"2026-01-02T03:04:05.100Z"}`, `{"at":"2026-01-02T03:04:05+01:00"}`,
		`{"at":null}`, `{"at":5}`, `{"at":"bad"}`, `{"at":"2026-01-02T03:04:05\u002e1Z"}`, `{"at":"2026-01-02t03:04:05Z"}`,
		`{"at":"2026-01-02T03:04:05,5Z"}`, `{"at":"2026-01-02T3:04:05Z"}`,
		`{"r":{ "a" : 1 }}`, `{"r":null}`, `{"r":"s"}`, `{"r":[1, 2]}`,
		`{"K":"upper"}`, `{"k":"lower"}`, `{"\u212a":"kelvin"}`, `{"Σ":"sig"}`, `{"ς":"final"}`, `{"σ":"s"}`,
		"{\"name\":\"\xff\xfe\"}", "{\"name\":\"a\\ud800b\"}", "{\"na\xffme\":\"x\"}", `{"name":"\ud83d\ude00"}`,
		`{"unknown":{"deep":[1,2,{"x":"y"}]},"name":"kept"}`, `{"unknown":5,"n":"bad"}`, `{"name":"a"`, `{"name":"a"}x`,
		`{"Name":"a","taskID":"b","N":3,"U":4,"I8":5,"F":1.5,"B":true}`, `{"nAmE":"mixed"}`, `{"name ":"space"}`,
	}
	var cases []lc
	for _, s := range ins {
		var v lStruct
		err := json.Unmarshal([]byte(s), &v)
		c := lc{Hex: hx(s), OK: err == nil}
		if err == nil {
			out, merr := json.Marshal(v)
			if merr != nil {
				t.Fatal(merr)
			}
			c.Out = string(out)
		}
		cases = append(cases, c)
	}
	writeGolden(t, "gocompat/lenient.json", map[string]any{"cases": cases})
}

func TestGoldenAnyMap(t *testing.T) {
	type ac struct {
		Hex string `json:"hex"`
		OK  bool   `json:"ok"`
		Out string `json:"out,omitempty"`
	}
	ins := []string{
		`{"n":1,"f":1.0,"e":1e2,"big":12345678901234567890,"hlc":4611686018427387905,"neg":-0,"small":0.1}`,
		`{"x":1e21,"y":1e-7,"z":1e-6,"w":123456789012345678}`, `{"bad":1e400}`, `{"b":2,"a":{"d":1,"c":[{"z":1,"y":2}]}}`,
		`{"k":1,"k":2}`, `{"s":"<&>\u2028"}`, `[1,2]`, `null`, `"s"`, `{"t":true,"f":false,"n":null}`,
		"{\"bad\":\"\xff\"}", `{"x":-1.5e-10}`, `{"x":100000000000000000000}`,
	}
	var cases []ac
	for _, s := range ins {
		var m any
		err := json.Unmarshal([]byte(s), &m)
		c := ac{Hex: hx(s), OK: err == nil}
		if err == nil {
			out, _ := json.Marshal(m)
			c.Out = string(out)
		}
		cases = append(cases, c)
	}
	writeGolden(t, "gocompat/anymap.json", map[string]any{"cases": cases})
}

func TestGoldenTime(t *testing.T) {
	type fc struct {
		Secs   int64  `json:"secs"`
		Nanos  int    `json:"nanos"`
		Offset int    `json:"offset"`
		Nano   string `json:"rfc3339nano"`
		JSON   string `json:"json,omitempty"`
		Err    string `json:"err,omitempty"`
	}
	type in struct {
		s, n int64
		o    int
	}
	ins := []in{
		{0, 0, 0}, {1, 0, 0}, {1767323045, 0, 0}, {1767323045, 120000000, 0}, {1767323045, 0, 3600},
		{1767323045, 947220900, 0}, {1767323045, 868679000, 0}, {1767323045, 1, 0}, {1767323045, 999999999, -21600},
		{1767323045, 1234500, 19800}, {-62135596800, 0, 0}, {-62135596801, 0, 0}, {253402300799, 999999999, 0},
		{253402300800, 0, 0}, {-1, 500000000, 0}, {-62167219200, 0, 0}, {1767323045, 0, -1}, {1767323045, 0, 86340},
		{1767323045, 0, 86400}, {1767323045, 0, 45}, {951782400, 0, 0}, {4102444800, 0, -3600}, {-2208988800, 0, 0},
	}
	var fcs []fc
	for _, x := range ins {
		tm := time.Unix(x.s, x.n).In(time.FixedZone("", x.o))
		if x.o == 0 {
			tm = tm.UTC()
		}
		c := fc{Secs: x.s, Nanos: int(x.n), Offset: x.o, Nano: tm.Format(time.RFC3339Nano)}
		if b, err := json.Marshal(tm); err != nil {
			c.Err = err.Error()
		} else {
			c.JSON = string(b)
		}
		fcs = append(fcs, c)
	}
	type pc struct {
		In     string `json:"in"`
		OK     bool   `json:"ok"`
		Secs   int64  `json:"secs"`
		Nanos  int    `json:"nanos"`
		Offset int    `json:"offset"`
		Again  string `json:"again,omitempty"`
	}
	pins := []string{
		"2026-01-02T03:04:05Z", "2026-01-02T03:04:05.100Z", "2026-01-02T03:04:05.9472209Z", "2026-01-02T03:04:05.123456789Z",
		"2026-01-02T03:04:05.1234567891Z", "2026-01-02T03:04:05,5Z", "2026-01-02T03:04:05+01:00", "2026-01-02T03:04:05-06:00",
		"2026-01-02T03:04:05-00:00", "2026-01-02T03:04:05+0700", "2026-01-02t03:04:05Z", "2026-01-02T03:04:05z",
		"2026-01-02T3:04:05Z", "2026-01-02T03:4:05Z", "10000-01-02T03:04:05Z", "0000-01-01T00:00:00Z", "0001-01-01T00:00:00Z",
		"2026-02-29T00:00:00Z", "2024-02-29T00:00:00Z", "2026-13-01T00:00:00Z", "2026-01-32T00:00:00Z", "2026-01-02T24:00:00Z",
		"2026-01-02T23:60:00Z", "2026-01-02T23:59:60Z", "2026-01-02T03:04:05+24:00", "2026-01-02T03:04:05+23:59",
		"2026-01-02T03:04:05+23:60", "2026-01-02T03:04:05+25:00", "2026-01-02T03:04:05", "2026-01-02", "",
		"2026-01-02T03:04:05.Z", "2026-01-02T03:04:05.5", "2026-01-02T03:04:05Z ", " 2026-01-02T03:04:05Z",
		"2026-01-02T03:04:05.000000000Z", "+026-01-02T03:04:05Z", "-026-01-02T03:04:05Z", "2026-1-02T03:04:05Z",
		"2026-01-02T03:04:05.5+01:00", "2026-01-02T03:04:05UTC", "2026-01-02T03:04:05.-5Z", "2026-01-02T03:04:05+1:00",
		"9999-12-31T23:59:59.999999999-23:59", "2026-01-02T03:04:05+00:00",
	}
	var pcs []pc
	for _, s := range pins {
		var tm time.Time
		err := json.Unmarshal([]byte(strconv.Quote(s)), &tm)
		c := pc{In: s, OK: err == nil}
		if err == nil {
			_, off := tm.Zone()
			c.Secs, c.Nanos, c.Offset = tm.Unix(), tm.Nanosecond(), off
			b, _ := json.Marshal(tm)
			c.Again = string(b)
		}
		pcs = append(pcs, c)
	}
	// The two plain layouts tool text uses (surface.md §7.9).
	lay := time.Date(2026, 3, 4, 5, 6, 7, 8, time.FixedZone("", -5*3600))
	writeGolden(t, "gocompat/time.json", map[string]any{
		"format": fcs, "parse": pcs,
		"layoutDateTime": lay.Format("2006-01-02 15:04:05"), "layoutShort": lay.Format("01-02 15:04:05"),
	})
}

func TestGoldenQuote(t *testing.T) {
	type qc struct {
		Hex string `json:"hex"`
		Out string `json:"out"`
	}
	var cases []qc
	ins := append(stringInputs(), "\a\b\f\n\r\t\v", "'", "\u00ad", "\u2028", "\ufeff", "\U0001F600")
	for _, s := range ins {
		q := strconv.Quote(s)
		if q != fmt.Sprintf("%q", s) {
			t.Fatalf("%%q differs from Quote for %x", s)
		}
		cases = append(cases, qc{hx(s), q})
	}
	writeGolden(t, "gocompat/quote.json", map[string]any{"cases": cases})
}

func TestGoldenText(t *testing.T) {
	type ef struct {
		A  string `json:"a"`
		B  string `json:"b"`
		Eq bool   `json:"eq"`
	}
	pairs := [][2]string{
		{"Go", "GO"}, {"k", "\u212a"}, {"K", "\u212a"}, {"s", "ſ"}, {"S", "ſ"}, {"σ", "ς"}, {"Σ", "ς"},
		{"straße", "STRASSE"}, {"ǅ", "ǆ"}, {"Ǆ", "ǅ"}, {"İ", "i"}, {"ı", "I"}, {"ß", "ẞ"}, {"µ", "μ"},
		{"µ", "Μ"}, {"abc", "abcd"}, {"", ""}, {"\xff", "\xfe"}, {"\xff", "\ufffd"}, {"Ab\xff", "aB\xfe"},
		{"θ", "ϑ"}, {"Θ", "ϴ"}, {"å", "Å"}, {"Å", "å"}, {"ﬀ", "FF"}, {"Ⓐ", "ⓐ"}, {"𐐀", "𐐨"},
		{"résumé", "RÉSUMÉ"}, {"a\u0300", "A\u0300"},
	}
	var efs []ef
	for _, p := range pairs {
		efs = append(efs, ef{hx(p[0]), hx(p[1]), strings.EqualFold(p[0], p[1])})
	}
	type lc struct {
		In    string `json:"in"`
		Lower string `json:"lower"`
		Upper string `json:"upper"`
	}
	var lcs []lc
	for _, s := range append(stringInputs(), "İSTANBUL", "ΣΑΣ", "Straße", "ǅungla", "ẞ", "Ω", "K", "Å", "ﬀ") {
		lcs = append(lcs, lc{hx(s), hx(strings.ToLower(s)), hx(strings.ToUpper(s))})
	}
	// Per-rune spot checks over every rune, folded into a digest per
	// property so the golden stays small.
	digest := map[string]string{}
	props := map[string]func(rune) bool{
		"print": unicode.IsPrint, "graphic": unicode.IsGraphic, "letter": unicode.IsLetter,
		"digit": unicode.IsDigit, "space": unicode.IsSpace, "control": unicode.IsControl,
		"upper": unicode.IsUpper, "lower": unicode.IsLower, "punct": unicode.IsPunct,
		"mark": unicode.IsMark, "number": unicode.IsNumber, "symbol": unicode.IsSymbol,
		"cf": func(r rune) bool { return unicode.Is(unicode.Cf, r) },
	}
	for name, f := range props {
		var n int
		var sum uint64
		for r := rune(0); r <= unicode.MaxRune; r++ {
			if f(r) {
				n++
				sum = sum*31 + uint64(r)
			}
		}
		digest[name] = fmt.Sprintf("%d:%x", n, sum)
	}
	maps := map[string]func(rune) rune{"toLower": unicode.ToLower, "toUpper": unicode.ToUpper, "simpleFold": unicode.SimpleFold, "toTitle": unicode.ToTitle}
	for name, f := range maps {
		var n int
		var sum uint64
		for r := rune(0); r <= unicode.MaxRune; r++ {
			if m := f(r); m != r {
				n++
				sum = sum*31 + uint64(r)*7 + uint64(m)
			}
		}
		digest[name] = fmt.Sprintf("%d:%x", n, sum)
	}
	writeGolden(t, "gocompat/text.json", map[string]any{
		"unicodeVersion": unicode.Version, "equalFold": efs, "case": lcs, "digest": digest,
	})
	if out := os.Getenv("IRORI_TABLES_OUT"); out != "" {
		writeTables(t, out, props, maps)
	}
}

// writeTables emits crates/gocompat/src/text/tables.rs from Go's unicode
// package: one range list per property and one compressed delta table per
// case mapping.
func writeTables(t *testing.T, path string, props map[string]func(rune) bool, maps map[string]func(rune) rune) {
	var b strings.Builder
	fmt.Fprintf(&b, "// Code generated by tools/goldens/gocompat_golden_test.go from Go %s\n", runtime.Version())
	fmt.Fprintf(&b, "// (package unicode, Unicode %s). DO NOT EDIT; rerun tools/goldens/export.sh.\n\n", unicode.Version)
	fmt.Fprintf(&b, "/// The Unicode version of Go's `unicode` package these tables came from.\npub const UNICODE_VERSION: &str = %q;\n\n", unicode.Version)
	names := []string{"print", "graphic", "letter", "digit", "space", "control", "upper", "lower", "punct", "mark", "number", "symbol", "cf"}
	for _, name := range names {
		f := props[name]
		var rs [][2]rune
		for r := rune(0); r <= unicode.MaxRune; r++ {
			if !f(r) {
				continue
			}
			if n := len(rs); n > 0 && rs[n-1][1] == r-1 {
				rs[n-1][1] = r
			} else {
				rs = append(rs, [2]rune{r, r})
			}
		}
		fmt.Fprintf(&b, "#[rustfmt::skip]\npub static %s: &[(u32, u32)] = &[\n", strings.ToUpper(name))
		for i, r := range rs {
			if i%8 == 0 {
				b.WriteString("   ")
			}
			fmt.Fprintf(&b, " (0x%x, 0x%x),", r[0], r[1])
			if i%8 == 7 || i == len(rs)-1 {
				b.WriteString("\n")
			}
		}
		b.WriteString("];\n\n")
	}
	for _, name := range []string{"toLower", "toUpper", "toTitle", "simpleFold"} {
		f := maps[name]
		// (lo, hi, stride, delta): every stride-th rune in [lo, hi] maps to r+delta.
		type ent struct {
			lo, hi, stride rune
			delta          int64
		}
		var es []ent
		for r := rune(0); r <= unicode.MaxRune; r++ {
			m := f(r)
			if m == r {
				continue
			}
			d := int64(m) - int64(r)
			if n := len(es); n > 0 {
				e := &es[n-1]
				if e.delta == d && e.lo == e.hi && (r-e.hi == 1 || r-e.hi == 2) {
					e.stride, e.hi = r-e.hi, r
					continue
				}
				if e.delta == d && e.lo != e.hi && r-e.hi == e.stride {
					e.hi = r
					continue
				}
			}
			es = append(es, ent{r, r, 1, d})
		}
		fmt.Fprintf(&b, "#[rustfmt::skip]\npub static %s: &[(u32, u32, u32, i32)] = &[\n", camelToUpperSnake(name))
		for i, e := range es {
			if i%4 == 0 {
				b.WriteString("   ")
			}
			fmt.Fprintf(&b, " (0x%x, 0x%x, %d, %d),", e.lo, e.hi, e.stride, e.delta)
			if i%4 == 3 || i == len(es)-1 {
				b.WriteString("\n")
			}
		}
		b.WriteString("];\n\n")
	}
	if err := os.WriteFile(path, []byte(strings.TrimRight(b.String(), "\n")+"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
}

func camelToUpperSnake(s string) string {
	var b strings.Builder
	for _, r := range s {
		if unicode.IsUpper(r) {
			b.WriteByte('_')
		}
		b.WriteRune(unicode.ToUpper(r))
	}
	return b.String()
}

func TestGoldenRegex(t *testing.T) {
	type rc struct {
		Pat   string  `json:"pat"`
		In    string  `json:"in"`
		Found [][]int `json:"found"`
	}
	cases := [][2]string{
		{`\w+`, "héllo wörld_1 ٣"}, {`\d+`, "12 ٣٤ 5"}, {`\s+`, "a \t\n\v\f\rb\u00a0c\u2003d\u0085e"},
		{`\bcat\b`, "cat écat caté concat cat_ cat"}, {`\Bx\B`, "axb x éxé"}, {`[\w-]+`, "a-b é_c"},
		{`[^\s]+`, "a b\vc"}, {`[\S]+`, "a b\u00a0c"}, {`\W+`, "a, é!b"}, {`\D+`, "1é2"},
		{`(?i)k`, "kK\u212a"}, {`(?i)straße`, "STRASSE straße STRAßE"}, {`(?i)σ`, "Σσς"},
		{`#(\d{2,5})\b`, "fix #12 and #123456 and #99é"}, {`a*`, "baaab"}, {`x*`, "éxé"},
		{`[[:alpha:]]+`, "abc é"}, {`\pL+`, "abc é ٣"}, {`.`, "a\nb"}, {`(?s).`, "a\nb"},
		{`^\s*$`, "  \v "}, {`(?m)^\w`, "a\nb\n é"}, {`[\d\s]+`, "1 2\v3"},
	}
	var rcs []rc
	for _, c := range cases {
		re := regexp.MustCompile(c[0])
		found := re.FindAllStringIndex(c[1], -1)
		if found == nil {
			found = [][]int{}
		}
		rcs = append(rcs, rc{c[0], c[1], found})
	}
	writeGolden(t, "gocompat/regex.json", map[string]any{"cases": rcs})
}
