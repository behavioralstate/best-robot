//! A ROS 2 message type as JSON Schema, from the typedefs rosapi gives (`message_details`, `action_goal_details`
//! and their kin): the first typedef is the type itself, the rest the types it nests. Every object is closed, so a
//! goal with a field the type lacks is refused at the door, naming it.

use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
pub struct Typedef {
    #[serde(rename = "type")]
    pub name: String,
    pub fieldnames: Vec<String>,
    pub fieldtypes: Vec<String>,
    /// -1 a single value, 0 a sequence of any length, n an array of exactly n.
    pub fieldarraylen: Vec<i64>,
}

pub fn of(typedefs: &[Typedef]) -> Result<Value, String> {
    let root = typedefs.first().ok_or("rosapi gave no typedef")?;
    object(root, typedefs, 0)
}

fn object(t: &Typedef, all: &[Typedef], depth: usize) -> Result<Value, String> {
    if depth > 32 {
        return Err(format!("{} nests deeper than 32 types", t.name));
    }
    if t.fieldnames.len() != t.fieldtypes.len() || t.fieldnames.len() != t.fieldarraylen.len() {
        return Err(format!(
            "rosapi gave {} with fields, types and lengths that do not line up",
            t.name
        ));
    }
    let mut properties = serde_json::Map::new();
    for ((name, ty), len) in t.fieldnames.iter().zip(&t.fieldtypes).zip(&t.fieldarraylen) {
        properties.insert(name.clone(), field(ty, *len, all, depth)?);
    }
    Ok(json!({ "type": "object", "properties": properties, "additionalProperties": false }))
}

fn field(ty: &str, len: i64, all: &[Typedef], depth: usize) -> Result<Value, String> {
    // rosbridge carries a sequence of bytes as one base64 string.
    if len >= 0 && matches!(ty, "uint8" | "char") {
        return Ok(json!({ "type": "string", "contentEncoding": "base64" }));
    }
    let one = match ty {
        "bool" | "boolean" => json!({ "type": "boolean" }),
        "float" | "double" | "float32" | "float64" => json!({ "type": "number" }),
        "int8" | "int16" | "int32" | "int64" => json!({ "type": "integer" }),
        "uint8" | "uint16" | "uint32" | "uint64" | "byte" | "char" | "octet" => {
            json!({ "type": "integer", "minimum": 0 })
        }
        "string" | "wstring" => json!({ "type": "string" }),
        nested => {
            let t = all
                .iter()
                .find(|t| t.name == nested)
                .ok_or_else(|| format!("rosapi gave no typedef for {nested}"))?;
            object(t, all, depth + 1)?
        }
    };
    Ok(match len {
        -1 => one,
        0 => json!({ "type": "array", "items": one }),
        n => json!({ "type": "array", "items": one, "minItems": n, "maxItems": n }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typedefs(v: Value) -> Vec<Typedef> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_goal_becomes_a_closed_object_of_its_fields() {
        // As rosapi answers action_goal_details for example_interfaces/action/Fibonacci.
        let s = of(&typedefs(
            json!([{ "type": "example_interfaces/Fibonacci_Goal", "fieldnames": ["order"],
            "fieldtypes": ["int32"], "fieldarraylen": [-1], "examples": ["0"] }]),
        ))
        .unwrap();
        assert_eq!(
            s,
            json!({ "type": "object", "properties": { "order": { "type": "integer" } }, "additionalProperties": false })
        );
    }

    #[test]
    fn nested_types_sequences_arrays_and_bytes_each_take_their_shape() {
        let s = of(&typedefs(json!([
            { "type": "pkg/Root", "fieldnames": ["header", "cells", "corners", "data", "ok"],
              "fieldtypes": ["std_msgs/Header", "float", "float64", "uint8", "boolean"], "fieldarraylen": [-1, 0, 4, 0, -1] },
            { "type": "std_msgs/Header", "fieldnames": ["stamp", "frame_id"],
              "fieldtypes": ["builtin_interfaces/Time", "string"], "fieldarraylen": [-1, -1] },
            { "type": "builtin_interfaces/Time", "fieldnames": ["sec", "nanosec"],
              "fieldtypes": ["int32", "uint32"], "fieldarraylen": [-1, -1] }
        ])))
        .unwrap();
        let p = &s["properties"];
        assert_eq!(
            p["header"]["properties"]["stamp"]["properties"]["nanosec"]["minimum"],
            0
        );
        assert_eq!(p["cells"], json!({ "type": "array", "items": { "type": "number" } }));
        assert_eq!(p["corners"]["minItems"], 4);
        assert_eq!(p["data"]["contentEncoding"], "base64");
        assert_eq!(p["ok"]["type"], "boolean");
    }

    #[test]
    fn a_type_rosapi_did_not_describe_is_refused_by_name() {
        let e = of(&typedefs(
            json!([{ "type": "pkg/Root", "fieldnames": ["x"], "fieldtypes": ["pkg/Missing"],
            "fieldarraylen": [-1] }]),
        ))
        .unwrap_err();
        assert!(e.contains("pkg/Missing"), "{e}");
    }
}
