//! gocompat against vectors Go 1.26.5 produced (testdata/golden/gocompat,
//! exported by tools/goldens/export.sh with `go test -overlay`).

use std::collections::BTreeMap;

use gocompat::json::{self, AnyValue, Out, RawJson, Schema};
use gocompat::time::GoTime;
use serde::Serialize;
use serde_json::Value as J;

fn golden(name: &str) -> J {
    let p = format!(
        "{}/../../testdata/golden/gocompat/{name}",
        manifest_dir().display()
    );
    serde_json::from_slice(&std::fs::read(&p).unwrap_or_else(|e| panic!("{p}: {e}"))).unwrap()
}

fn unhex(v: &J) -> Vec<u8> {
    hex::decode(v.as_str().unwrap()).unwrap()
}

fn s(v: &J) -> &str {
    v.as_str().unwrap()
}

#[test]
fn valid_matches_go() {
    let g = golden("valid.json");
    for c in g["cases"].as_array().unwrap() {
        let input = unhex(&c["hex"]);
        assert_eq!(
            json::valid(&input),
            c["ok"].as_bool().unwrap(),
            "{:?}",
            String::from_utf8_lossy(&input)
        );
    }
    for d in g["deep"].as_array().unwrap() {
        let n = d["depth"].as_u64().unwrap() as usize;
        let obj = d["obj"].as_bool().unwrap();
        let text = if obj {
            format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n))
        } else {
            format!("{}{}", "[".repeat(n), "]".repeat(n))
        };
        assert_eq!(
            json::valid(text.as_bytes()),
            d["ok"].as_bool().unwrap(),
            "depth {n} obj {obj}"
        );
        // The tree builders cope with every depth Go accepts.
        if json::valid(text.as_bytes()) {
            assert_eq!(
                json::canonical_raw(text.as_bytes()).unwrap(),
                json::compact(text.as_bytes(), false).unwrap()
            );
            let _ = json::round_trip(text.as_bytes()).unwrap();
        }
    }
}

#[test]
fn syntax_errors_carry_gos_text() {
    assert_eq!(
        json::check_valid(b"").unwrap_err().to_string(),
        "unexpected end of JSON input"
    );
    assert_eq!(
        json::check_valid(b"x").unwrap_err().to_string(),
        "invalid character 'x' looking for beginning of value"
    );
    assert_eq!(
        json::check_valid(b"{\"a\":1}}").unwrap_err().to_string(),
        "invalid character '}' after top-level value"
    );
    assert_eq!(
        json::check_valid(b"\"\x01\"").unwrap_err().to_string(),
        "invalid character '\\x01' in string literal"
    );
}

#[test]
fn strings_and_floats_match_go_marshal() {
    let g = golden("marshal.json");
    for c in g["strings"].as_array().unwrap() {
        let input = unhex(&c["hex"]);
        let mut a = Vec::new();
        json::append_string(&mut a, &input, true);
        assert_eq!(String::from_utf8(a).unwrap(), s(&c["html"]), "{input:x?}");
        let mut b = Vec::new();
        json::append_string(&mut b, &input, false);
        assert_eq!(String::from_utf8(b).unwrap(), s(&c["noHtml"]), "{input:x?}");
        if let Ok(st) = std::str::from_utf8(&input) {
            assert_eq!(
                String::from_utf8(json::marshal(st).unwrap()).unwrap(),
                s(&c["html"])
            );
        }
    }
    for c in g["f64"].as_array().unwrap() {
        let f = f64::from_bits(u64::from_str_radix(s(&c["bits"]), 16).unwrap());
        match json::format_f64(f) {
            Ok(out) => assert_eq!(out, s(&c["out"]), "{f:e}"),
            Err(_) => assert!(c["err"].as_bool().unwrap_or(false), "{f} should encode"),
        }
    }
    for c in g["f32"].as_array().unwrap() {
        let f = f32::from_bits(u32::from_str_radix(s(&c["bits"]), 16).unwrap());
        match json::format_f32(f) {
            Ok(out) => assert_eq!(out, s(&c["out"]), "{f:e}"),
            Err(_) => assert!(c["err"].as_bool().unwrap_or(false), "{f} should encode"),
        }
    }
}

#[derive(Serialize)]
struct MInner {
    x: i64,
    #[serde(skip_serializing_if = "String::is_empty")]
    y: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}
fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn ser_bytes<S: serde::Serializer>(b: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
    match b {
        Some(b) => s.serialize_bytes(b),
        None => s.serialize_none(),
    }
}

/// The Go `mStruct` in tools/goldens/gocompat_golden_test.go, field for field.
#[derive(Serialize)]
struct MStruct {
    name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    empty: String,
    #[serde(skip_serializing_if = "is_zero")]
    zero: i64,
    kept: i64,
    #[serde(rename = "false", skip_serializing_if = "is_false")]
    false_: bool,
    #[serde(rename = "true")]
    true_: bool,
    #[serde(rename = "nilSlice")]
    nil_slice: Option<Vec<String>>,
    #[serde(rename = "nilOmit", skip_serializing_if = "Option::is_none")]
    nil_omit: Option<Vec<String>>,
    #[serde(rename = "emptySlice")]
    empty_slice: Option<Vec<String>>,
    #[serde(rename = "nilMap")]
    nil_map: Option<BTreeMap<String, i64>>,
    map: std::collections::HashMap<String, i64>,
    #[serde(rename = "intMap")]
    int_map: std::collections::HashMap<i64, String>,
    #[serde(serialize_with = "ser_bytes")]
    bytes: Option<Vec<u8>>,
    #[serde(rename = "nilBytes", serialize_with = "ser_bytes")]
    nil_bytes: Option<Vec<u8>>,
    #[serde(rename = "emptyBytes", serialize_with = "ser_bytes")]
    empty_bytes: Option<Vec<u8>>,
    ptr: Option<MInner>,
    #[serde(rename = "ptrOmit", skip_serializing_if = "Option::is_none")]
    ptr_omit: Option<MInner>,
    inner: MInner,
    /// `omitempty` never omits a time.Time.
    #[serde(rename = "zeroTime")]
    zero_time: GoTime,
    /// `omitzero` does.
    #[serde(rename = "omitZeroTime", skip_serializing_if = "GoTime::is_zero")]
    omit_zero_time: GoTime,
    at: GoTime,
    local: GoTime,
    raw: Option<RawJson>,
    #[serde(rename = "nilRaw")]
    nil_raw: Option<RawJson>,
    #[serde(rename = "html<&>")]
    html: String,
    f64: f64,
    f32: f32,
    i64: i64,
    u64: u64,
    any: AnyValue,
    #[serde(rename = "anyMap")]
    any_map: BTreeMap<String, AnyValue>,
    strs: BTreeMap<String, String>,
}

fn marshal_value() -> MStruct {
    let mut obj = BTreeMap::new();
    obj.insert("z".to_string(), AnyValue::Number(1.0));
    obj.insert("a".to_string(), AnyValue::Array(vec![]));
    MStruct {
        name: "n<1>".into(),
        empty: String::new(),
        zero: 0,
        kept: 0,
        false_: false,
        true_: true,
        nil_slice: None,
        nil_omit: None,
        empty_slice: Some(vec![]),
        nil_map: None,
        map: [
            ("b", 2),
            ("a", 1),
            ("B", 3),
            ("é", 4),
            ("", 5),
            ("a\u{2028}", 6),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect(),
        int_map: [(10, "ten"), (9, "nine"), (-1, "neg")]
            .into_iter()
            .map(|(k, v)| (k, v.to_string()))
            .collect(),
        bytes: Some(b"hello\x00\xff".to_vec()),
        nil_bytes: None,
        empty_bytes: Some(vec![]),
        ptr: None,
        ptr_omit: None,
        inner: MInner {
            x: 7,
            y: String::new(),
        },
        zero_time: GoTime::ZERO,
        omit_zero_time: GoTime::ZERO,
        at: GoTime::unix(1_793_197_523, 947_220_900),
        local: GoTime::unix(1_767_344_645, 120_000_000).with_offset(-6 * 3600),
        raw: Some(RawJson(b" { \"k\" : [1, 2 ,\"<&>\"] } ".to_vec())),
        nil_raw: None,
        html: "a&b".into(),
        f64: 1e21,
        f32: 0.1,
        i64: i64::MIN,
        u64: u64::MAX,
        any: AnyValue::Array(vec![
            AnyValue::Number(1.5),
            AnyValue::String("x".into()),
            AnyValue::Null,
            AnyValue::Bool(true),
            AnyValue::Object(obj),
        ]),
        any_map: [("hlc".to_string(), AnyValue::Number((1u64 << 62) as f64))]
            .into_iter()
            .collect(),
        strs: [("k".to_string(), "\u{2029}".to_string())]
            .into_iter()
            .collect(),
    }
}

#[test]
fn struct_marshal_matches_go() {
    let g = golden("marshal.json");
    let v = marshal_value();
    assert_eq!(
        String::from_utf8(json::marshal(&v).unwrap()).unwrap(),
        s(&g["struct"])
    );
    assert_eq!(
        String::from_utf8(json::marshal_indent(&v, "", "  ").unwrap()).unwrap(),
        s(&g["structIndent"])
    );
    assert_eq!(
        String::from_utf8(json::marshal_indent(&v.inner, ">", "\t").unwrap()).unwrap(),
        s(&g["innerIndentPrefix"])
    );
    let line = json::encode_line(&v, json::Options { escape_html: false }).unwrap();
    assert_eq!(
        String::from_utf8(line).unwrap(),
        s(&g["structEncoderNoHtml"])
    );
    for c in g["raw"].as_array().unwrap() {
        let input = s(&c["in"]).as_bytes();
        assert_eq!(
            String::from_utf8(json::compact(input, false).unwrap()).unwrap(),
            s(&c["compact"])
        );
        assert_eq!(
            String::from_utf8(json::compact(input, true).unwrap()).unwrap(),
            s(&c["compactEscaped"])
        );
        assert_eq!(
            String::from_utf8(json::marshal(&RawJson(input.to_vec())).unwrap()).unwrap(),
            s(&c["compactEscaped"])
        );
        assert_eq!(
            String::from_utf8(json::indent(input, "", "  ").unwrap()).unwrap(),
            s(&c["indent"])
        );
    }
    assert!(g["emptyRawErr"].as_bool().unwrap());
    assert!(json::marshal(&RawJson(Vec::new())).is_err());
    assert!(json::marshal(&f64::NAN).is_err());
}

#[test]
fn canonical_matches_identity_canonical() {
    let g = golden("canonical.json");
    for c in g["cases"].as_array().unwrap() {
        let input = unhex(&c["hex"]);
        match json::canonical_raw(&input) {
            Ok(out) => assert_eq!(
                String::from_utf8(out).unwrap(),
                s(&c["out"]),
                "{:?}",
                String::from_utf8_lossy(&input)
            ),
            Err(e) => assert!(c["err"].as_bool().unwrap_or(false), "{e}"),
        }
    }
    // ESC vector (identity.md §7.1): a Go map through Canonical.
    let mut m: BTreeMap<&str, Option<RawJson>> = BTreeMap::new();
    m.insert("big", Some(RawJson(b"4611686018427387904".to_vec())));
    m.insert("m", None);
    m.insert("n", Some(RawJson(b"1".to_vec())));
    m.insert("nil", None);
    let st = "a<b>&\u{2028}\u{2029}\u{1}\u{8}\u{c}\n\t\u{7f}\"\\é\u{2028}\u{2029}";
    m.insert("s", Some(RawJson(json::marshal(st).unwrap())));
    assert_eq!(
        String::from_utf8(json::canonical(&m).unwrap()).unwrap(),
        s(&g["esc"])
    );
    // A cert-shaped struct; struct order differs from the sorted output.
    #[derive(Serialize)]
    struct Cert {
        #[serde(rename = "personId")]
        person_id: &'static str,
        #[serde(rename = "deviceId")]
        device_id: &'static str,
        #[serde(rename = "devicePub")]
        device_pub: &'static str,
        label: &'static str,
        role: &'static str,
        at: GoTime,
    }
    let cert = Cert {
        person_id: "p",
        device_id: "d",
        device_pub: "k",
        label: "a<b",
        role: "desktop",
        at: GoTime::unix(1, 0),
    };
    assert_eq!(
        String::from_utf8(json::canonical(&cert).unwrap()).unwrap(),
        s(&g["cert"])
    );
}

/// identity.md §7.1 CANON: a fixture root self-cert body (Go json.Marshal of
/// the Cert struct) re-canonicalised; `want` is Go's `identity.Canonical`.
#[test]
fn canon_vector_from_a_root_cert() {
    let raw = br#"{"personId":"3097e2dee2cb4a34b53840cdb705aed7","deviceId":"a1b2c3d4e5f60718293a4b5c6d7e8f90","devicePub":"22fc297792f0b6ffc0bfcfdb7edb0c0aa14e025a365ec0e342e86e3829cb74b6","label":"DESKTOP-FIXTURE","role":"desktop","at":"2026-10-28T14:25:23.9472209Z"}"#;
    let want = r#"{"at":"2026-10-28T14:25:23.9472209Z","deviceId":"a1b2c3d4e5f60718293a4b5c6d7e8f90","devicePub":"22fc297792f0b6ffc0bfcfdb7edb0c0aa14e025a365ec0e342e86e3829cb74b6","label":"DESKTOP-FIXTURE","personId":"3097e2dee2cb4a34b53840cdb705aed7","role":"desktop"}"#;
    assert_eq!(
        String::from_utf8(json::canonical_raw(raw).unwrap()).unwrap(),
        want
    );
}

fn inner_schema() -> Schema {
    Schema::object(vec![("x", Schema::Int(64)), ("y", Schema::String)])
}

/// The Go `lStruct` in the exporter.
fn l_schema() -> Schema {
    Schema::object(vec![
        ("name", Schema::String),
        ("taskId", Schema::String),
        ("n", Schema::Int(64)),
        ("u", Schema::Uint(64)),
        ("i8", Schema::Int(8)),
        ("f", Schema::Float(64)),
        ("f32", Schema::Float(32)),
        ("b", Schema::Bool),
        ("s", Schema::Slice(Box::new(Schema::String))),
        ("o", Schema::Ptr(Box::new(inner_schema()))),
        ("v", inner_schema()),
        ("vs", Schema::Slice(Box::new(inner_schema()))),
        ("m", Schema::Map(Box::new(Schema::Int(64)))),
        ("a", Schema::Any),
        ("at", Schema::Time),
        ("r", Schema::Raw),
        ("K", Schema::String),
        ("k", Schema::String),
        ("σ", Schema::String),
    ])
}

/// Serialize a decoded `Out` the way Go marshals the struct it filled.
struct GoOut<'a>(&'a Out);

impl Serialize for GoOut<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq, SerializeStruct};
        match self.0 {
            Out::Bool(b) => s.serialize_bool(*b),
            Out::Int(i) => s.serialize_i64(*i),
            Out::Uint(u) => s.serialize_u64(*u),
            Out::Float(f) => s.serialize_f64(*f),
            Out::Float32(f) => s.serialize_f32(*f),
            Out::String(t) => s.serialize_str(t),
            Out::Time(t) => t.serialize(s),
            Out::Raw(None) => s.serialize_none(),
            Out::Raw(Some(r)) => RawJson(r.clone()).serialize(s),
            Out::Any(None) => s.serialize_none(),
            Out::Any(Some(a)) => a.serialize(s),
            Out::Slice(None) | Out::Map(None) | Out::Ptr(None) => s.serialize_none(),
            Out::Slice(Some(items)) => {
                let mut q = s.serialize_seq(Some(items.len()))?;
                for it in items {
                    q.serialize_element(&GoOut(it))?;
                }
                q.end()
            }
            Out::Map(Some(m)) => {
                let mut q = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    q.serialize_entry(k, &GoOut(v))?;
                }
                q.end()
            }
            Out::Ptr(Some(b)) => GoOut(b).serialize(s),
            Out::Struct(fields) => {
                let mut q = s.serialize_struct("", fields.len())?;
                for (k, v) in fields {
                    q.serialize_field(k, &GoOut(v))?;
                }
                q.end()
            }
        }
    }
}

#[test]
fn lenient_matches_go_unmarshal() {
    let g = golden("lenient.json");
    let schema = l_schema();
    let mut n = 0;
    for c in g["cases"].as_array().unwrap() {
        let input = unhex(&c["hex"]);
        let shown = String::from_utf8_lossy(&input).into_owned();
        let got = json::unmarshal(&input, &schema);
        if !c["ok"].as_bool().unwrap() {
            assert!(
                got.is_err(),
                "Go refused {shown}, Rust took {:?}",
                got.map(|o| json::marshal(&GoOut(&o)))
            );
            continue;
        }
        let out = got.unwrap_or_else(|e| panic!("Go took {shown}, Rust refused: {e}"));
        let enc = String::from_utf8(json::marshal(&GoOut(&out)).unwrap()).unwrap();
        assert_eq!(enc, s(&c["out"]), "{shown}");
        n += 1;
    }
    assert!(n > 50);
}

#[test]
fn anymap_matches_go() {
    let g = golden("anymap.json");
    for c in g["cases"].as_array().unwrap() {
        let input = unhex(&c["hex"]);
        let got = json::round_trip(&input);
        if c["ok"].as_bool().unwrap() {
            assert_eq!(String::from_utf8(got.unwrap()).unwrap(), s(&c["out"]));
        } else {
            assert!(got.is_err(), "{:?}", String::from_utf8_lossy(&input));
        }
    }
}

#[test]
fn time_matches_go() {
    let g = golden("time.json");
    for c in g["format"].as_array().unwrap() {
        let t = GoTime::unix(c["secs"].as_i64().unwrap(), c["nanos"].as_i64().unwrap())
            .with_offset(c["offset"].as_i64().unwrap() as i32);
        assert_eq!(t.format_rfc3339_nano(), s(&c["rfc3339nano"]), "{c}");
        match t.marshal_json() {
            Ok(j) => assert_eq!(j, s(&c["json"]), "{c}"),
            Err(e) => assert!(s(&c["err"]).ends_with(&e.to_string()), "{c}: {e}"),
        }
    }
    for c in g["parse"].as_array().unwrap() {
        let got = GoTime::parse_rfc3339(s(&c["in"]));
        if !c["ok"].as_bool().unwrap() {
            assert!(got.is_none(), "Go refused {c}");
            continue;
        }
        let t = got.unwrap_or_else(|| panic!("Go took {c}"));
        assert_eq!(
            (t.secs, i64::from(t.nanos), i64::from(t.offset)),
            (
                c["secs"].as_i64().unwrap(),
                c["nanos"].as_i64().unwrap(),
                c["offset"].as_i64().unwrap()
            ),
            "{c}"
        );
        match c["again"].as_str() {
            Some(again) => assert_eq!(t.marshal_json().unwrap(), again),
            None => assert!(t.marshal_json().is_err(), "{c}"),
        }
    }
    let lay = GoTime::unix(1_772_618_767, 8).with_offset(-5 * 3600);
    assert_eq!(lay.format_date_time(), s(&g["layoutDateTime"]));
    assert_eq!(lay.format_short(), s(&g["layoutShort"]));
    assert!(GoTime::ZERO.is_zero());
    assert_eq!(
        GoTime::ZERO.marshal_json().unwrap(),
        "\"0001-01-01T00:00:00Z\""
    );
}

#[test]
fn quote_matches_strconv_quote() {
    let g = golden("quote.json");
    for c in g["cases"].as_array().unwrap() {
        let input = unhex(&c["hex"]);
        assert_eq!(
            gocompat::quote::quote_bytes(&input),
            s(&c["out"]),
            "{input:x?}"
        );
    }
    // coord.md §7.7 vector.
    assert_eq!(
        gocompat::quote::quote("tab\there \"q\" é \x01"),
        r#""tab\there \"q\" é \x01""#
    );
}

type Pred = fn(char) -> bool;
type Map = fn(char) -> char;

#[test]
fn text_matches_go_unicode() {
    let g = golden("text.json");
    assert_eq!(s(&g["unicodeVersion"]), gocompat::text::UNICODE_VERSION);
    for c in g["equalFold"].as_array().unwrap() {
        let (a, b) = (unhex(&c["a"]), unhex(&c["b"]));
        assert_eq!(
            gocompat::text::equal_fold_bytes(&a, &b),
            c["eq"].as_bool().unwrap(),
            "{:?} {:?}",
            String::from_utf8_lossy(&a),
            String::from_utf8_lossy(&b)
        );
    }
    for c in g["case"].as_array().unwrap() {
        let input = unhex(&c["in"]);
        assert_eq!(
            gocompat::text::to_lower_bytes(&input).into_bytes(),
            unhex(&c["lower"]),
            "{input:x?}"
        );
        assert_eq!(
            gocompat::text::to_upper_bytes(&input).into_bytes(),
            unhex(&c["upper"]),
            "{input:x?}"
        );
    }
    // Whole-table digests: every rune of every property and mapping.
    use gocompat::text as t;
    let props: [(&str, Pred); 13] = [
        ("print", t::is_print),
        ("graphic", t::is_graphic),
        ("letter", t::is_letter),
        ("digit", t::is_digit),
        ("space", t::is_space),
        ("control", t::is_control),
        ("upper", t::is_upper),
        ("lower", t::is_lower),
        ("punct", t::is_punct),
        ("mark", t::is_mark),
        ("number", t::is_number),
        ("symbol", t::is_symbol),
        ("cf", t::is_cf),
    ];
    for (name, f) in props {
        let (mut n, mut sum) = (0u64, 0u64);
        for r in (0..=0x10ffffu32).filter_map(char::from_u32) {
            if f(r) {
                n += 1;
                sum = sum.wrapping_mul(31).wrapping_add(r as u64);
            }
        }
        assert_eq!(format!("{n}:{sum:x}"), s(&g["digest"][name]), "{name}");
    }
    let maps: [(&str, Map); 4] = [
        ("toLower", t::to_lower),
        ("toUpper", t::to_upper),
        ("toTitle", t::to_title),
        ("simpleFold", t::simple_fold),
    ];
    for (name, f) in maps {
        let (mut n, mut sum) = (0u64, 0u64);
        for r in (0..=0x10ffffu32).filter_map(char::from_u32) {
            let m = f(r);
            if m != r {
                n += 1;
                sum = sum.wrapping_mul(31).wrapping_add((r as u64) * 7 + m as u64);
            }
        }
        assert_eq!(format!("{n}:{sum:x}"), s(&g["digest"][name]), "{name}");
    }
}

#[test]
fn regex_from_go_matches_go_regexp() {
    let g = golden("regex.json");
    for c in g["cases"].as_array().unwrap() {
        let re = gocompat::regex::from_go(s(&c["pat"])).unwrap();
        let input = s(&c["in"]);
        let got: Vec<Vec<u64>> = re
            .find_iter(input)
            .map(|m| vec![m.start() as u64, m.end() as u64])
            .collect();
        let want: Vec<Vec<u64>> = c["found"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                p.as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_u64().unwrap())
                    .collect()
            })
            .collect();
        assert_eq!(
            got,
            want,
            "{} on {input:?} (as {})",
            s(&c["pat"]),
            gocompat::regex::translate_go(s(&c["pat"]))
        );
    }
}

#[test]
fn regex_compile_refuses_unicode_classes() {
    use gocompat::regex::compile;
    for p in [r"\w+", r"\d", r"[\s,]", r"\bx", r"\S", r"(?-u:\w)"] {
        assert!(compile(p).is_err(), "{p}");
    }
    for p in [
        r"[0-9A-Za-z_]+",
        r"(?-u:\b)cat(?-u:\b)",
        r"[\t\n\f\r ]+",
        r"\pL",
        r"\\w",
    ] {
        assert!(compile(p).is_ok(), "{p}");
    }
}

fn pairs<'a>(v: &'a J, key: &str) -> Vec<(&'a str, &'a str)> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (s(&c["in"]), s(&c["out"])))
        .collect()
}

#[test]
fn filepath_matches_go() {
    use gocompat::filepath::{unix, windows};
    let g = golden("filepath.json");
    let w = &g["windows"];
    for (i, o) in pairs(w, "clean") {
        assert_eq!(windows::clean(i), o, "windows Clean({i:?})");
    }
    for (i, o) in pairs(w, "volumeName") {
        assert_eq!(windows::volume_name(i), o, "windows VolumeName({i:?})");
    }
    for c in w["isAbs"].as_array().unwrap() {
        assert_eq!(
            windows::is_abs(s(&c["in"])),
            c["abs"].as_bool().unwrap(),
            "windows IsAbs({c})"
        );
    }
    for c in w["join"].as_array().unwrap() {
        let elem: Vec<&str> = c["elem"]
            .as_array()
            .map(|a| a.iter().map(s).collect())
            .unwrap_or_default();
        assert_eq!(windows::join(&elem), s(&c["out"]), "windows Join({elem:?})");
    }
    for c in w["rel"].as_array().unwrap() {
        let got = windows::rel(s(&c["base"]), s(&c["targ"]));
        if c["err"].as_bool().unwrap_or(false) {
            assert!(got.is_err(), "windows Rel({c}) = {got:?}");
        } else {
            assert_eq!(got.unwrap(), s(&c["out"]), "windows Rel({c})");
        }
    }
    let u = &g["unix"];
    for (i, o) in pairs(u, "clean") {
        assert_eq!(unix::clean(i), o, "unix Clean({i:?})");
    }
    for c in u["isAbs"].as_array().unwrap() {
        assert_eq!(
            unix::is_abs(s(&c["in"])),
            c["abs"].as_bool().unwrap(),
            "unix IsAbs({c})"
        );
    }
    for c in u["join"].as_array().unwrap() {
        let elem: Vec<&str> = c["elem"]
            .as_array()
            .map(|a| a.iter().map(s).collect())
            .unwrap_or_default();
        assert_eq!(unix::join(&elem), s(&c["out"]), "unix Join({elem:?})");
    }
    for c in u["rel"].as_array().unwrap() {
        let got = unix::rel(s(&c["base"]), s(&c["targ"]));
        if c["err"].as_bool().unwrap_or(false) {
            assert!(got.is_err(), "unix Rel({c}) = {got:?}");
        } else {
            assert_eq!(got.unwrap(), s(&c["out"]), "unix Rel({c})");
        }
    }
    assert_eq!(unix::abs_from(".irori/", "/Users/pat"), "/Users/pat/.irori");
}

#[cfg(windows)]
#[test]
fn windows_abs_is_lexical_and_clean() {
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        gocompat::filepath::abs("").unwrap(),
        gocompat::filepath::clean(&cwd)
    );
    assert_eq!(
        gocompat::filepath::abs(r"C:\a\b\..\c\.").unwrap(),
        r"C:\a\c"
    );
    assert_eq!(gocompat::filepath::abs("C:/x//y/").unwrap(), r"C:\x\y");
}

#[test]
fn lenient_board_row_shape() {
    // store.md §7.5: case-insensitive keys, last wins, top-level null is an
    // empty row, a type mismatch anywhere fails the row.
    let schema = Schema::object(vec![
        ("taskId", Schema::String),
        (
            "steps",
            Schema::Slice(Box::new(Schema::object(vec![("id", Schema::String)]))),
        ),
    ]);
    let o = json::unmarshal(
        br#"{"TASKID":"t1","steps":[{"ID":"a"},{"id":"b"}]}"#,
        &schema,
    )
    .unwrap();
    assert_eq!(o.get_str("taskId"), "t1");
    let ids: Vec<&str> = o
        .get("steps")
        .unwrap()
        .items()
        .iter()
        .map(|s| s.get_str("id"))
        .collect();
    assert_eq!(ids, ["a", "b"]);
    assert_eq!(
        json::unmarshal(b"null", &schema).unwrap().get_str("taskId"),
        ""
    );
    assert!(json::unmarshal(br#"{"steps":"x"}"#, &schema).is_err());
    assert!(json::unmarshal(br#"[1]"#, &schema).is_err());
}

/// Go `os.Getwd` on Unix: `$PWD` wins only when absolute and the same
/// directory as `.`; otherwise the system's (symlink-resolved) answer.
#[test]
fn unix_getwd_prefers_pwd_like_go() {
    use gocompat::filepath::unix;
    let sys = || Ok("/private/real".to_string());
    let same = |_: &str| true;
    let other = |_: &str| false;
    assert_eq!(
        unix::getwd_with(Some("/link"), &same, &sys).unwrap(),
        "/link"
    );
    assert_eq!(
        unix::getwd_with(Some("/link"), &other, &sys).unwrap(),
        "/private/real"
    );
    assert_eq!(
        unix::getwd_with(Some("rel"), &same, &sys).unwrap(),
        "/private/real"
    );
    assert_eq!(
        unix::getwd_with(None, &same, &sys).unwrap(),
        "/private/real"
    );
}

/// The package dir as cargo gives it when it runs the test, so a test binary
/// built in another tree (a shared or copied target dir) still reads this
/// tree's files; the compile-time path only when run outside cargo.
fn manifest_dir() -> std::path::PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| env!("CARGO_MANIFEST_DIR").into(), Into::into)
}
