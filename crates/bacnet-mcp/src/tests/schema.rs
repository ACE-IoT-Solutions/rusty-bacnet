//! Every tool's structured content, success and error alike, against the
//! output schema the tool declares.
//!
//! The check covers the JSON Schema keywords the output schemas use:
//! `type` (one or a list), `properties`, `required`, `items`, `enum`,
//! `anyOf` and `additionalProperties: false`.
//! A test asserts the schemas use no other keyword that constrains a
//! value, so a new one can't pass unchecked.

use serde_json::{Value, json};

use super::Lab;
use crate::Policy;

/// The keywords this check understands; descriptions are notes.
const KNOWN: [&str; 8] = [
    "type",
    "properties",
    "required",
    "items",
    "enum",
    "anyOf",
    "additionalProperties",
    "description",
];

/// Why `value` doesn't fit `schema`, at `path`.
fn violations(schema: &Value, value: &Value, path: &str, out: &mut Vec<String>) {
    let Some(schema) = schema.as_object() else {
        return;
    };
    for key in schema.keys() {
        assert!(
            KNOWN.contains(&key.as_str()),
            "{path}: the check doesn't know `{key}`"
        );
    }
    if let Some(types) = schema.get("type") {
        let types: Vec<&str> = match types {
            Value::String(one) => vec![one.as_str()],
            Value::Array(many) => many.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let fits = types.iter().any(|t| match *t {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.is_i64() || value.is_u64(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        });
        if !fits {
            out.push(format!("{path}: {value} is not {types:?}"));
            return;
        }
    }
    if let Some(Value::Array(choices)) = schema.get("anyOf") {
        let fits_one = choices.iter().any(|choice| {
            let mut inner = Vec::new();
            violations(choice, value, path, &mut inner);
            inner.is_empty()
        });
        if !fits_one {
            out.push(format!("{path}: {value} fits none of anyOf"));
            return;
        }
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && let (Some(object), properties) = (value.as_object(), schema.get("properties"))
    {
        for key in object.keys() {
            if properties.and_then(|p| p.get(key)).is_none() {
                out.push(format!("{path}: unexpected {key}"));
            }
        }
    }
    if let Some(Value::Array(allowed)) = schema.get("enum")
        && !allowed.contains(value)
    {
        out.push(format!("{path}: {value} is not one of {allowed:?}"));
    }
    if let (Some(Value::Object(properties)), Some(object)) =
        (schema.get("properties"), value.as_object())
    {
        for (key, inner) in properties {
            if let Some(field) = object.get(key) {
                violations(inner, field, &format!("{path}.{key}"), out);
            }
        }
    }
    if let (Some(Value::Array(required)), Some(object)) =
        (schema.get("required"), value.as_object())
    {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                out.push(format!("{path}: missing {key}"));
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), value.as_array()) {
        for (i, item) in array.iter().enumerate() {
            violations(items, item, &format!("{path}[{i}]"), out);
        }
    }
}

/// The validator itself catches a wrong type, a missing field and a value
/// outside an enum.
#[test]
fn the_check_finds_violations() {
    let schema = json!({
        "type": "object",
        "properties": { "n": { "type": "integer" }, "e": { "enum": ["a"] } },
        "required": ["n"],
    });
    let mut out = Vec::new();
    violations(&schema, &json!({ "n": "1", "e": "b" }), "$", &mut out);
    violations(&schema, &json!({}), "$", &mut out);
    assert_eq!(out.len(), 3, "{out:?}");
    let either = json!({ "anyOf": [
        { "type": "integer" },
        { "type": "object", "properties": { "a": {} }, "additionalProperties": false },
    ] });
    let mut out = Vec::new();
    violations(&either, &json!(3), "$", &mut out);
    violations(&either, &json!({ "a": 1 }), "$", &mut out);
    assert!(out.is_empty(), "{out:?}");
    violations(&either, &json!({ "b": 1 }), "$", &mut out);
    violations(&either, &json!("x"), "$", &mut out);
    assert_eq!(out.len(), 2, "{out:?}");
}

/// Calls that succeed and calls that fail, for every tool.
#[tokio::test(start_paused = true)]
async fn results_fit_their_output_schemas() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize().await;
    let tools = lab.agent.request("tools/list", json!({})).await;
    let schemas: Vec<(String, Value)> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["outputSchema"].clone(),
            )
        })
        .collect();
    let calls = [
        ("bacnet_discover", json!({ "range": "1201", "names": true })),
        ("bacnet_discover", json!({ "send": false })),
        ("bacnet_discover", json!({ "range": "1-4194303" })),
        (
            "bacnet_discover",
            json!({ "scope": "network", "network": 0 }),
        ),
        (
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "ai:3" }], "preset": "point" }),
        ),
        (
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "ai:1", "properties": ["all"] }] }),
        ),
        (
            "bacnet_read",
            json!({ "device": "4242", "reads": [{ "object": "ai:1" }] }),
        ),
        (
            "bacnet_read",
            json!({ "device": "2001:300", "reads": [{ "object": "ai:1" }] }),
        ),
        (
            "bacnet_read",
            json!({ "device": "1201", "reads": [{ "object": "nope:1" }] }),
        ),
        (
            "bacnet_list_objects",
            json!({ "device": "1201", "names": true }),
        ),
        ("bacnet_list_objects", json!({ "device": "4242" })),
        (
            "bacnet_list_objects",
            json!({ "device": "1201", "cursor": "00.00" }),
        ),
        (
            "bacnet_describe_device",
            json!({ "device": "1201", "count_objects": true }),
        ),
        ("bacnet_describe_device", json!({ "device": "2001:300" })),
    ];
    let mut errors = 0;
    for (tool, args) in calls {
        let result = lab.agent.call(tool, args.clone()).await;
        if result["isError"] == json!(true) {
            errors += 1;
        }
        let schema = &schemas.iter().find(|(name, _)| name == tool).unwrap().1;
        let mut out = Vec::new();
        violations(schema, &result["structuredContent"], "$", &mut out);
        assert!(out.is_empty(), "{tool} {args}: {out:?}");
    }
    assert!(errors >= 7, "error results were checked too: {errors}");
    lab.stop().await;
}
