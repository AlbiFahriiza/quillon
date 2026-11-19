//! Semantic document diff: two documents → the list of canonical-JSON paths
//! where they differ, so agents can verify their edits without reading the
//! whole document back (plan §8 `diff_documents`). Deterministic by
//! construction: canonical form is sorted-key JSON (plan §5.2), so the same
//! pair of documents always yields the same entries in the same order.

use crate::document::Document;
use serde_json::Value;

/// One changed subtree: JSON-pointer path plus the canonical values on both
/// sides (`null` marks a side where the subtree does not exist).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DiffEntry {
    pub path: String,
    pub before: Value,
    pub after: Value,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DiffReport {
    pub changed: bool,
    pub entries: Vec<DiffEntry>,
}

/// Diff two documents in canonical form. Index-based for arrays (scenes and
/// elements are ordered), key-based for objects.
pub fn diff_documents(a: &Document, b: &Document) -> DiffReport {
    // serde_json::to_value is the same canonical form canonical_json encodes:
    // without the `preserve_order` feature its map type is BTreeMap-backed,
    // so object keys iterate sorted and the diff is deterministic.
    let (a, b) = match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(a), Ok(b)) => (a, b),
        // Serialization is infallible for Document in practice (canonical.rs);
        // if it ever fails, report the whole tree as changed rather than
        // panicking inside an MCP tool.
        _ => {
            return DiffReport {
                changed: true,
                entries: vec![DiffEntry {
                    path: String::new(),
                    before: Value::Null,
                    after: Value::Null,
                }],
            };
        }
    };
    let mut entries = Vec::new();
    diff_value(&a, &b, "", &mut entries);
    DiffReport {
        changed: !entries.is_empty(),
        entries,
    }
}

fn diff_value(a: &Value, b: &Value, path: &str, out: &mut Vec<DiffEntry>) {
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            for (k, va) in ma {
                match mb.get(k) {
                    Some(vb) => diff_value(va, vb, &format!("{path}/{k}"), out),
                    None => out.push(DiffEntry {
                        path: format!("{path}/{k}"),
                        before: va.clone(),
                        after: Value::Null,
                    }),
                }
            }
            for (k, vb) in mb {
                if !ma.contains_key(k) {
                    out.push(DiffEntry {
                        path: format!("{path}/{k}"),
                        before: Value::Null,
                        after: vb.clone(),
                    });
                }
            }
        }
        (Value::Array(aa), Value::Array(ab)) => {
            for (i, (va, vb)) in aa.iter().zip(ab.iter()).enumerate() {
                diff_value(va, vb, &format!("{path}/{i}"), out);
            }
            for (offset, va) in aa.iter().skip(ab.len()).enumerate() {
                out.push(DiffEntry {
                    path: format!("{path}/{}", ab.len() + offset),
                    before: va.clone(),
                    after: Value::Null,
                });
            }
            for (offset, vb) in ab.iter().skip(aa.len()).enumerate() {
                out.push(DiffEntry {
                    path: format!("{path}/{}", aa.len() + offset),
                    before: Value::Null,
                    after: vb.clone(),
                });
            }
        }
        _ => {
            if a != b {
                out.push(DiffEntry {
                    path: path.to_string(),
                    before: a.clone(),
                    after: b.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: &str) -> Document {
        serde_json::from_str(json).expect("test document parses")
    }

    const BASE: &str = r##"{
        "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
        "scenes": [ { "id": "s1", "duration": 705600000,
            "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 50, "h": 50,
                           "color": "#ff0000" } ] } ]
    }"##;

    #[test]
    fn identical_documents_report_no_change() {
        let (a, b) = (doc(BASE), doc(BASE));
        let report = diff_documents(&a, &b);
        assert!(!report.changed);
        assert!(report.entries.is_empty());
    }

    #[test]
    fn changed_scene_duration_is_reported_with_pointer_path() {
        let b = doc(r##"{ "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "scenes": [ { "id": "s1", "duration": 352800000,
                "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 50, "h": 50,
                               "color": "#ff0000" } ] } ] }"##);
        let report = diff_documents(&doc(BASE), &b);
        assert!(report.changed);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path, "/scenes/0/duration");
        assert_eq!(report.entries[0].before, serde_json::json!(705_600_000));
        assert_eq!(report.entries[0].after, serde_json::json!(352_800_000));
    }

    #[test]
    fn changed_element_property_points_into_the_element() {
        let b = doc(r##"{ "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "scenes": [ { "id": "s1", "duration": 705600000,
                "elements": [ { "type": "rect", "x": 99, "y": 10, "w": 50, "h": 50,
                               "color": "#ff0000" } ] } ] }"##);
        let report = diff_documents(&doc(BASE), &b);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path, "/scenes/0/elements/0/x");
    }

    #[test]
    fn added_scene_is_reported_as_null_before() {
        let b = doc(r##"{ "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "scenes": [ { "id": "s1", "duration": 705600000,
                "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 50, "h": 50,
                               "color": "#ff0000" } ] },
              { "id": "s2", "duration": 705600000, "elements": [] } ] }"##);
        let report = diff_documents(&doc(BASE), &b);
        assert!(report.changed);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path, "/scenes/1");
        assert_eq!(report.entries[0].before, Value::Null);
        assert!(report.entries[0].after.is_object());
    }

    #[test]
    fn removed_scene_is_reported_as_null_after() {
        let empty_second = doc(r##"{ "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "scenes": [ { "id": "s1", "duration": 705600000,
                "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 50, "h": 50,
                               "color": "#ff0000" } ] },
              { "id": "s2", "duration": 705600000, "elements": [] } ] }"##);
        let report = diff_documents(&empty_second, &doc(BASE));
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path, "/scenes/1");
        assert_eq!(report.entries[0].after, Value::Null);
    }

    #[test]
    fn added_asset_is_reported() {
        let b = doc(r##"{ "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "assets": { "img": { "type": "image", "src": "a.png" } },
            "scenes": [ { "id": "s1", "duration": 705600000,
                "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 50, "h": 50,
                               "color": "#ff0000" } ] } ] }"##);
        let report = diff_documents(&doc(BASE), &b);
        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path, "/assets/img");
        assert_eq!(report.entries[0].before, Value::Null);
    }

    #[test]
    fn diff_is_symmetric_in_path_but_not_in_before_after() {
        let a = doc(BASE);
        let b = doc(r##"{ "v": 1, "size": { "w": 320, "h": 240 }, "fps": 10,
            "scenes": [ { "id": "s1", "duration": 705600000,
                "elements": [ { "type": "rect", "x": 10, "y": 10, "w": 50, "h": 50,
                               "color": "#00ff00" } ] } ] }"##);
        let ab = diff_documents(&a, &b);
        let ba = diff_documents(&b, &a);
        assert_eq!(ab.entries[0].path, ba.entries[0].path);
        assert_eq!(ab.entries[0].before, ba.entries[0].after);
        assert_eq!(ab.entries[0].after, ba.entries[0].before);
    }
}
