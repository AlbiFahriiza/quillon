//! JSON Schema for the document format, derived from the Rust types with
//! schemars (draft 2020-12, `oneOf` discriminated unions — verified
//! empirically in research/J). Published as `docs/schema.json` via
//! `cargo xtask export-schema`.

use crate::document::Document;
use schemars::{JsonSchema, schema_for};
use serde_json::Value;
use std::sync::OnceLock;

/// Generate the document schema (slow first call, cached afterwards).
pub fn document_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| schema_for!(Document).to_value())
}

/// The schema entry for a type other than the root document (used by tests
/// and tooling that want a sub-schema).
pub fn schema_for_type<T: JsonSchema>() -> Value {
    schema_for!(T).to_value()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Element;

    #[test]
    fn root_schema_is_draft_2020_12() {
        let schema = document_schema();
        assert_eq!(
            schema.get("$schema").and_then(Value::as_str),
            Some("https://json-schema.org/draft/2020-12/schema")
        );
    }

    #[test]
    fn element_schema_discriminates_by_type() {
        let schema = schema_for_type::<Element>();
        let variants = schema
            .get("oneOf")
            .and_then(Value::as_array)
            .expect("oneOf");
        let mut discriminators = Vec::new();
        for variant in variants {
            let tag = variant
                .pointer("/properties/type/const")
                .and_then(Value::as_str)
                .expect("const discriminator");
            discriminators.push(tag);
        }
        discriminators.sort();
        assert_eq!(discriminators, ["image", "rect", "text"]);
    }
}
