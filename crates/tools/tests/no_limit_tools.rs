//! Decision 23 for the limits on agents (card #576): the app sets them, and
//! no tool an agent can call takes one. A tool that needs to is a change
//! to that decision, not to this list.

use serde_json::Value;

fn names(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                out.push(k.to_lowercase());
                names(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| names(x, out)),
        _ => {}
    }
}

#[test]
fn no_tool_takes_a_limit() {
    let specs: Value = serde_json::from_str(include_str!("../src/specs.json")).unwrap();
    let mut seen = Vec::new();
    for tool in specs.as_array().expect("a list of tools") {
        let name = tool["name"].as_str().unwrap_or_default();
        let schema = tool.get("inputSchema").or_else(|| tool.get("input_schema"));
        assert!(schema.is_some(), "{name} has no schema: {tool}");
        names(schema.unwrap(), &mut seen);
        seen.push(name.to_lowercase());
    }
    for bad in [
        "cap",
        "spawn_cap",
        "idle_mins",
        "idlemins",
        "crash_limit",
        "crashlimit",
        "start_tries",
        "starttries",
    ] {
        assert!(!seen.iter().any(|n| n == bad), "a tool takes {bad}");
    }
}
