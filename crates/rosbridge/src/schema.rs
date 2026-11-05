//! A ROS 2 message type as JSON Schema, from the typedefs rosapi gives (`message_details`, `action_goal_details`
//! and their kin): the first typedef is the type itself, the rest the types it nests. Every object is closed and every
//! integer held to its width, so a goal with a field the type lacks, or a number rosbridge would wrap, is refused at
//! the door, naming it.

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

/// Which way a message travels, since rosbridge reads and writes floats differently.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// A goal the door sends: a float must be a number.
    In,
    /// Feedback, a result or a topic message the robot sends: rosbridge writes NaN and infinity as null.
    Out,
}

pub fn of(typedefs: &[Typedef], way: Way) -> Result<Value, String> {
    let root = typedefs.first().ok_or("rosapi gave no typedef")?;
    object(root, typedefs, way, 0)
}

/// `value` with every integral number written as an integer where `schema` asks for an integer: JSON Schema counts
/// `4.0` as an integer, rosbridge refuses it for an int field.
pub fn integers(value: Value, schema: &Value) -> Value {
    match value {
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| {
                    let s = &schema["properties"][&k];
                    (k, integers(v, s))
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.into_iter().map(|v| integers(v, &schema["items"])).collect()),
        Value::Number(n) if schema["type"] == "integer" && n.is_f64() => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            if f.fract() != 0.0 {
                Value::Number(n)
            } else if f >= i64::MIN as f64 && f < i64::MAX as f64 {
                json!(f as i64)
            } else if f >= 0.0 && f < u64::MAX as f64 {
                json!(f as u64)
            } else {
                Value::Number(n)
            }
        }
        other => other,
    }
}

fn object(t: &Typedef, all: &[Typedef], way: Way, depth: usize) -> Result<Value, String> {
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
        properties.insert(name.clone(), field(ty, *len, all, way, depth)?);
    }
    Ok(json!({ "type": "object", "properties": properties, "additionalProperties": false }))
}

fn field(ty: &str, len: i64, all: &[Typedef], way: Way, depth: usize) -> Result<Value, String> {
    // rosbridge carries a sequence of bytes as one base64 string.
    if len >= 0 && matches!(ty, "uint8" | "char") {
        return Ok(json!({ "type": "string", "contentEncoding": "base64" }));
    }
    let int = |min: i64, max: u64| json!({ "type": "integer", "minimum": min, "maximum": max });
    let one = match ty {
        "bool" | "boolean" => json!({ "type": "boolean" }),
        "float" | "double" | "float32" | "float64" => match way {
            Way::In if matches!(ty, "float" | "float32") => {
                json!({ "type": "number", "minimum": -f32::MAX, "maximum": f32::MAX })
            }
            Way::In => json!({ "type": "number" }),
            Way::Out => {
                json!({ "type": ["number", "null"], "description": "null where the robot reported NaN or infinity" })
            }
        },
        "int8" => int(i8::MIN.into(), i8::MAX as u64),
        "int16" => int(i16::MIN.into(), i16::MAX as u64),
        "int32" => int(i32::MIN.into(), i32::MAX as u64),
        "int64" => int(i64::MIN, i64::MAX as u64),
        "uint8" | "byte" | "char" | "octet" => int(0, u8::MAX.into()),
        "uint16" => int(0, u16::MAX.into()),
        "uint32" => int(0, u32::MAX.into()),
        "uint64" => int(0, u64::MAX),
        "string" | "wstring" => json!({ "type": "string" }),
        nested => {
            let t = all
                .iter()
                .find(|t| t.name == nested)
                .ok_or_else(|| format!("rosapi gave no typedef for {nested}"))?;
            object(t, all, way, depth + 1)?
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
        let s = of(
            &typedefs(
                json!([{ "type": "example_interfaces/Fibonacci_Goal", "fieldnames": ["order"],
            "fieldtypes": ["int32"], "fieldarraylen": [-1], "examples": ["0"] }]),
            ),
            Way::In,
        )
        .unwrap();
        assert_eq!(
            s,
            json!({ "type": "object", "properties": { "order":
                { "type": "integer", "minimum": -2147483648i64, "maximum": 2147483647 } },
                "additionalProperties": false })
        );
    }

    #[test]
    fn each_integer_is_held_to_its_width_so_rosbridge_never_wraps_it() {
        let t = typedefs(json!([{ "type": "pkg/Root", "fieldnames": ["a", "b", "c", "d"],
            "fieldtypes": ["int8", "uint16", "uint64", "int64"], "fieldarraylen": [-1, -1, -1, -1] }]));
        let p = &of(&t, Way::In).unwrap()["properties"];
        assert_eq!(
            (p["a"]["minimum"].clone(), p["a"]["maximum"].clone()),
            (json!(-128), json!(127))
        );
        assert_eq!(
            (p["b"]["minimum"].clone(), p["b"]["maximum"].clone()),
            (json!(0), json!(65535))
        );
        assert_eq!(p["c"]["maximum"], json!(u64::MAX));
        assert_eq!(p["d"]["minimum"], json!(i64::MIN));
    }

    #[test]
    fn a_float_the_robot_sends_may_be_null_since_rosbridge_writes_nan_so_but_a_goal_s_may_not() {
        let t = typedefs(
            json!([{ "type": "pkg/Root", "fieldnames": ["x"], "fieldtypes": ["float64"],
            "fieldarraylen": [-1] }]),
        );
        assert_eq!(of(&t, Way::In).unwrap()["properties"]["x"]["type"], "number");
        assert_eq!(
            of(&t, Way::Out).unwrap()["properties"]["x"]["type"],
            json!(["number", "null"])
        );
    }

    #[test]
    fn an_integral_float_is_sent_as_an_integer_where_the_type_asks_for_one() {
        let t = typedefs(json!([
            { "type": "pkg/Root", "fieldnames": ["order", "speed", "steps"],
              "fieldtypes": ["int32", "float64", "pkg/Step"], "fieldarraylen": [-1, -1, 0] },
            { "type": "pkg/Step", "fieldnames": ["n"], "fieldtypes": ["uint8"], "fieldarraylen": [-1] }
        ]));
        let s = of(&t, Way::In).unwrap();
        let goal = integers(json!({ "order": 4.0, "speed": 2.0, "steps": [{ "n": 3.0 }] }), &s);
        assert_eq!(goal.to_string(), r#"{"order":4,"speed":2.0,"steps":[{"n":3}]}"#);
        // Above i64, a uint64 still goes as an integer.
        let big = integers(
            json!({ "order": 1e19 }),
            &json!({ "properties": { "order": { "type": "integer" } } }),
        );
        assert_eq!(big.to_string(), r#"{"order":10000000000000000000}"#);
    }

    #[test]
    fn a_pointer_finds_the_field_it_names() {
        let t = typedefs(json!([
            { "type": "pkg/Root", "fieldnames": ["pose"], "fieldtypes": ["pkg/Pose"], "fieldarraylen": [-1] },
            { "type": "pkg/Pose", "fieldnames": ["x"], "fieldtypes": ["float64"], "fieldarraylen": [-1] }
        ]));
        let s = of(&t, Way::In).unwrap();
        assert_eq!(robot::schema::at(&s, "/pose/x")["type"], "number");
        assert!(robot::schema::at(&s, "/pose/y").is_null());
    }

    #[test]
    fn a_float32_goal_field_is_held_to_what_a_float32_carries() {
        let t = typedefs(
            json!([{ "type": "pkg/Root", "fieldnames": ["x", "y"], "fieldtypes": ["float32", "float64"],
            "fieldarraylen": [-1, -1] }]),
        );
        let p = &of(&t, Way::In).unwrap()["properties"];
        assert_eq!(p["x"]["maximum"], json!(f32::MAX));
        assert!(p["y"].get("maximum").is_none());
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
        ])), Way::In)
        .unwrap();
        let p = &s["properties"];
        assert_eq!(
            p["header"]["properties"]["stamp"]["properties"]["nanosec"]["minimum"],
            0
        );
        assert_eq!(
            (p["cells"]["type"].clone(), p["cells"]["items"]["type"].clone()),
            (json!("array"), json!("number"))
        );
        assert_eq!(p["corners"]["minItems"], 4);
        assert_eq!(p["data"]["contentEncoding"], "base64");
        assert_eq!(p["ok"]["type"], "boolean");
    }

    #[test]
    fn a_type_rosapi_did_not_describe_is_refused_by_name() {
        let e = of(
            &typedefs(
                json!([{ "type": "pkg/Root", "fieldnames": ["x"], "fieldtypes": ["pkg/Missing"],
            "fieldarraylen": [-1] }]),
            ),
            Way::In,
        )
        .unwrap_err();
        assert!(e.contains("pkg/Missing"), "{e}");
    }
}
