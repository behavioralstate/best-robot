//! Reading the JSON Schemas the ports carry: what a JSON Pointer names in a value one of them describes.

use serde_json::Value;

/// The schema of the field `pointer` names in a value `schema` describes, or null when there is none.
pub fn at<'a>(schema: &'a Value, pointer: &str) -> &'a Value {
    pointer.split('/').skip(1).fold(schema, |s, key| &s["properties"][key])
}

/// Whether `schema` allows a value of JSON type `kind`, alone or among others (`["number", "null"]`).
pub fn allows(schema: &Value, kind: &str) -> bool {
    match &schema["type"] {
        Value::String(t) => t == kind,
        Value::Array(ts) => ts.iter().any(|t| t == kind),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pointer_names_a_nested_field_or_nothing() {
        let s = json!({ "type": "object", "properties": { "twist": { "type": "object", "properties": {
            "x": { "type": ["number", "null"] } } } } });
        assert!(allows(at(&s, "/twist/x"), "number"));
        assert!(!allows(at(&s, "/twist/y"), "number"));
        assert!(!allows(at(&s, "/x"), "number"));
        assert!(!allows(at(&s, "/twist"), "number"));
    }
}
