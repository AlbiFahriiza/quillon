//! Template variables (plan §4 F12, §12 v0.6): typed variables declared on
//! a document, filled per data row at render time. Substitution happens on
//! the JSON tree *before* validation, so the result goes through the full
//! schema + semantic pipeline (P1 document-first is never bypassed) and a
//! bad value surfaces as a normal JSON-pointer error.

use crate::color::Color;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The declared type of one template variable; rows are checked against it
/// at substitution time, so a wrong-typed CSV cell fails before rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VariableType {
    /// Free text (names, titles).
    String,
    /// A number, rendered into string fields ("score: {{score}}").
    Number,
    /// A #rrggbb color, filling color fields.
    Color,
    /// A media path, filling asset `src` (or id) fields.
    Asset,
}

/// One declared template variable with its fallback: a template rendered
/// without `--data` uses the defaults and stays a valid document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VariableDef {
    #[serde(rename = "type")]
    pub variable_type: VariableType,
    /// Used when a data row omits this column.
    pub default: String,
}

/// A substitution failure: operational input (bad row), not a document
/// validation error — the CLI exits 1 for these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    pub message: String,
}

impl std::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TemplateError {}

fn err(message: impl Into<String>) -> TemplateError {
    TemplateError {
        message: message.into(),
    }
}

impl VariableType {
    /// Validate a raw row value against the declared type.
    fn check(&self, name: &str, value: &str) -> Result<(), TemplateError> {
        match self {
            VariableType::String => Ok(()),
            VariableType::Number => value.parse::<f64>().map(|_| ()).map_err(|_| {
                err(format!(
                    "variable \"{name}\" is a number but the row value {value:?} is not"
                ))
            }),
            VariableType::Color => Color::parse(value).map(|_| ()).ok_or_else(|| {
                err(format!(
                    "variable \"{name}\" is a color but {value:?} is not #rrggbb"
                ))
            }),
            VariableType::Asset => {
                if value.trim().is_empty() {
                    Err(err(format!(
                        "variable \"{name}\" is an asset but the row value is empty"
                    )))
                } else {
                    Ok(())
                }
            }
        }
    }
}

/// Resolve one variable's value for a row: the row cell when present, the
/// declared default otherwise; type-checked either way.
fn resolve(
    name: &str,
    def: &VariableDef,
    row: &BTreeMap<String, String>,
) -> Result<String, TemplateError> {
    let value = row.get(name).unwrap_or(&def.default).clone();
    def.variable_type.check(name, &value)?;
    Ok(value)
}

/// Interpolate every `{{name}}` marker in `text` using resolved values.
/// An unknown marker names the variable; empty markers are a syntax error.
fn interpolate(text: &str, values: &BTreeMap<String, String>) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            // An unterminated marker is left alone: it may be intentional
            // literal text, and validation will judge the final document.
            out.push_str(&rest[start..]);
            return Ok(out);
        };
        let name = after[..end].trim();
        if name.is_empty() {
            return Err(err("empty {{}} marker in the document"));
        }
        let Some(value) = values.get(name) else {
            return Err(err(format!(
                "the document references variable \"{name}\", which is not declared under \"variables\""
            )));
        };
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Substitute variables into a document JSON tree (design note v0.6 D1).
/// Every string in the tree — including nested asset fields — has its
/// `{{name}}` markers filled with the row's (type-checked) values. The
/// returned tree must still be validated before use.
pub fn apply_template_json(
    doc: &Value,
    defs: &BTreeMap<String, VariableDef>,
    row: &BTreeMap<String, String>,
) -> Result<Value, TemplateError> {
    // Columns the template does not declare are almost certainly typos.
    for column in row.keys() {
        if !defs.contains_key(column) {
            return Err(err(format!(
                "the data row has column \"{column}\", which the document does not declare under \"variables\""
            )));
        }
    }
    let mut values = BTreeMap::new();
    for (name, def) in defs {
        values.insert(name.clone(), resolve(name, def, row)?);
    }
    substitute_tree(doc, &values)
}

/// Fill a non-document pattern (the `--out` filename template) with a
/// row's resolved values — same markers, same type checks.
pub fn fill_pattern(
    pattern: &str,
    defs: &BTreeMap<String, VariableDef>,
    row: &BTreeMap<String, String>,
) -> Result<String, TemplateError> {
    let mut values = BTreeMap::new();
    for (name, def) in defs {
        values.insert(name.clone(), resolve(name, def, row)?);
    }
    interpolate(pattern, &values)
}

fn substitute_tree(
    value: &Value,
    values: &BTreeMap<String, String>,
) -> Result<Value, TemplateError> {
    match value {
        Value::String(text) => Ok(Value::String(interpolate(text, values)?)),
        Value::Array(items) => items
            .iter()
            .map(|item| substitute_tree(item, values))
            .collect(),
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, item) in map {
                out.insert(key.clone(), substitute_tree(item, values)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn defs() -> BTreeMap<String, VariableDef> {
        let mut defs = BTreeMap::new();
        defs.insert(
            "name".into(),
            VariableDef {
                variable_type: VariableType::String,
                default: "friend".into(),
            },
        );
        defs.insert(
            "score".into(),
            VariableDef {
                variable_type: VariableType::Number,
                default: "0".into(),
            },
        );
        defs.insert(
            "accent".into(),
            VariableDef {
                variable_type: VariableType::Color,
                default: "#ff8800".into(),
            },
        );
        defs
    }

    #[test]
    fn markers_fill_from_the_row_with_type_checks() {
        let doc = json!({
            "text": "Hi {{name}}, score {{score}}",
            "color": "{{accent}}",
            "untouched": 42
        });
        let mut row = BTreeMap::new();
        row.insert("name".to_string(), "Ada".to_string());
        row.insert("score".to_string(), "9.5".to_string());
        row.insert("accent".to_string(), "#00ff00".to_string());
        let out = apply_template_json(&doc, &defs(), &row).unwrap();
        assert_eq!(out["text"], json!("Hi Ada, score 9.5"));
        assert_eq!(out["color"], json!("#00ff00"));
        assert_eq!(out["untouched"], json!(42));
    }

    #[test]
    fn missing_columns_fall_back_to_declared_defaults() {
        let doc = json!({ "text": "Hi {{name}}, score {{score}}" });
        let out = apply_template_json(&doc, &defs(), &BTreeMap::new()).unwrap();
        assert_eq!(out["text"], json!("Hi friend, score 0"));
    }

    #[test]
    fn unknown_marker_is_a_named_error() {
        let doc = json!({ "text": "{{nope}}" });
        let error = apply_template_json(&doc, &defs(), &BTreeMap::new()).unwrap_err();
        assert!(error.message.contains("\"nope\""), "{}", error.message);
    }

    #[test]
    fn undeclared_row_column_is_rejected() {
        let doc = json!({ "text": "hi" });
        let mut row = BTreeMap::new();
        row.insert("nmae".to_string(), "typo".to_string());
        let error = apply_template_json(&doc, &defs(), &row).unwrap_err();
        assert!(error.message.contains("\"nmae\""), "{}", error.message);
    }

    #[test]
    fn wrong_typed_values_are_named_errors() {
        let doc = json!({ "score": "{{score}}" });
        let mut row = BTreeMap::new();
        row.insert("score".to_string(), "high".to_string());
        let error = apply_template_json(&doc, &defs(), &row).unwrap_err();
        assert!(error.message.contains("\"score\""), "{}", error.message);

        let empty_defs: BTreeMap<String, VariableDef> = BTreeMap::new();
        let mut row = BTreeMap::new();
        row.insert("accent".to_string(), "orange".to_string());
        let error = apply_template_json(&doc, &empty_defs, &row).unwrap_err();
        // No defs declared at all: the column itself is the problem.
        assert!(error.message.contains("\"accent\""), "{}", error.message);
    }

    #[test]
    fn unterminated_marker_stays_literal_text() {
        let doc = json!({ "text": "a {{ b" });
        let out = apply_template_json(&doc, &defs(), &BTreeMap::new()).unwrap();
        assert_eq!(out["text"], json!("a {{ b"));
    }

    #[test]
    fn substitution_reaches_nested_assets() {
        let doc = json!({
            "assets": { "photo": { "type": "image", "src": "img/{{name}}.png" } }
        });
        let mut row = BTreeMap::new();
        row.insert("name".to_string(), "ada".to_string());
        let out = apply_template_json(&doc, &defs(), &row).unwrap();
        assert_eq!(out["assets"]["photo"]["src"], json!("img/ada.png"));
    }
}
