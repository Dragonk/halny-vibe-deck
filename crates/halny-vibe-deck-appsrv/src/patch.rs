//! JSON-Patch applier for the vibe-app-server wire format.
//!
//! The app-server patch dialect is *not* RFC 6902: it has an extra
//! `append` op (string/array concat) and pointers are simple paths
//! (`/a/b/0`). RFC-6902-style libraries don't fit — keep this self
//! contained and unit tested (docs/appserver-protocol.md §9, pitfall 4).

use anyhow::{bail, Result};
use serde_json::Value;

/// One patch operation (`models.py` JsonPatchOperation).
#[derive(Clone, Debug, serde::Deserialize)]
pub struct PatchOp {
    pub op: String,
    /// JSON pointer (`/history/0/content`); empty = whole document.
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub value: Value,
}

/// Parse a JSON-pointer path into tokens (`/a/b/0` → ["a", "b", "0"]).
/// `~0`/`~1` escapes decoded; empty string → no tokens.
fn tokens(path: &str) -> Vec<String> {
    if path.is_empty() || path == "/" {
        return Vec::new();
    }
    path.trim_start_matches('/')
        .split('/')
        .map(|t| t.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn get<'a>(root: &'a mut Value, path: &str) -> Result<&'a mut Value> {
    let mut cur = root;
    for t in tokens(path) {
        cur = match cur {
            Value::Object(map) => map
                .get_mut(&t)
                .ok_or_else(|| anyhow::anyhow!("patch path: no key {t:?}"))?,
            Value::Array(arr) => {
                let i: usize = t.parse()?;
                arr.get_mut(i)
                    .ok_or_else(|| anyhow::anyhow!("patch path: index {i} out of range"))?
            }
            _ => bail!("patch path: {t:?} into scalar"),
        };
    }
    Ok(cur)
}

/// Like `get`, but creates missing object keys (JSON-Patch `add`).
fn get_or_create<'a>(root: &'a mut Value, path: &str) -> Result<&'a mut Value> {
    let mut cur = root;
    for t in tokens(path) {
        cur = match cur {
            Value::Object(map) => map.entry(t).or_insert(Value::Null),
            Value::Array(arr) => {
                let i: usize = t.parse()?;
                arr.get_mut(i)
                    .ok_or_else(|| anyhow::anyhow!("patch path: index {i} out of range"))?
            }
            _ => bail!("patch path: {t:?} into scalar"),
        };
    }
    Ok(cur)
}

/// Apply one operation in place.
pub fn apply(root: &mut Value, op: &PatchOp) -> Result<()> {
    match op.op.as_str() {
        "add" => {
            // add creates missing object keys (RFC 6902 semantics);
            // the app-server uses it for new fields on entries
            let v = get_or_create(root, &op.path)?;
            *v = op.value.clone();
        }
        "replace" => {
            let v = get(root, &op.path)?;
            *v = op.value.clone();
        }
        // Non-standard: append to a string or array (streaming text!).
        "append" => {
            let v = get(root, &op.path)?;
            match v {
                Value::String(s) => {
                    if let Some(add) = op.value.as_str() {
                        s.push_str(add);
                    } else {
                        bail!("append: string expected, got {}", op.value);
                    }
                }
                Value::Array(arr) => {
                    if let Some(add) = op.value.as_array() {
                        arr.extend(add.iter().cloned());
                    } else {
                        arr.push(op.value.clone());
                    }
                }
                _ => bail!("append: target not string/array at {}", op.path),
            }
        }
        "remove" => {
            let path = &op.path;
            let t = tokens(path)
                .pop()
                .ok_or_else(|| anyhow::anyhow!("remove: empty path"))?;
            let parent_path = path
                .strip_suffix(&format!("/{t}"))
                .unwrap_or("");
            let parent = get(root, parent_path)?;
            match parent {
                Value::Object(map) => {
                    if map.remove(&t).is_none() {
                        bail!("remove: no key {t:?}");
                    }
                }
                Value::Array(arr) => {
                    let i: usize = t.parse()?;
                    if i >= arr.len() {
                        bail!("remove: index {i} out of range");
                    }
                    arr.remove(i);
                }
                _ => bail!("remove: parent not container"),
            }
        }
        "test" => {
            let v = get(root, &op.path)?;
            if *v != op.value {
                bail!("test failed at {}", op.path);
            }
        }
        other => bail!("unknown patch op {other:?}"),
    }
    Ok(())
}

/// Apply a patch series in order.
pub fn apply_all(root: &mut Value, ops: &[PatchOp]) -> Result<()> {
    for op in ops {
        apply(root, op)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn apply_json(doc: Value, ops: Value) -> Result<Value> {
        let ops: Vec<PatchOp> = serde_json::from_value(ops)?;
        let mut doc = doc;
        apply_all(&mut doc, &ops)?;
        Ok(doc)
    }

    #[test]
    fn add_replace_remove() {
        let doc = json!({"a": {"b": 1}, "list": [1, 2, 3]});
        let out = apply_json(
            doc,
            json!([
                {"op": "add", "path": "/a/c", "value": 2},
                {"op": "replace", "path": "/a/b", "value": 9},
                {"op": "remove", "path": "/list/1"},
            ]),
        )
        .unwrap();
        assert_eq!(out, json!({"a": {"b": 9, "c": 2}, "list": [1, 3]}));
    }

    #[test]
    fn append_string_and_array() {
        let doc = json!({"text": "Hel", "items": [1]});
        let out = apply_json(
            doc,
            json!([
                {"op": "append", "path": "/text", "value": "lo"},
                {"op": "append", "path": "/items", "value": {"k": 2}},
            ]),
        )
        .unwrap();
        assert_eq!(out["text"], "Hello");
        assert_eq!(out["items"], json!([1, {"k": 2}]));
    }

    #[test]
    fn escapes_and_root_path() {
        let doc = json!({"a/b": 1, "c~": 2});
        let out = apply_json(
            doc,
            json!([
                {"op": "replace", "path": "/a~1b", "value": 9},
                {"op": "replace", "path": "/c~0", "value": 8},
            ]),
        )
        .unwrap();
        assert_eq!(out["a/b"], 9);
        assert_eq!(out["c~"], 8);
    }

    #[test]
    fn errors_are_reported() {
        let mut doc = json!({"a": 1});
        let ops: Vec<PatchOp> =
            serde_json::from_value(json!([{ "op": "remove", "path": "/b" }])).unwrap();
        assert!(apply_all(&mut doc.clone(), &ops).is_err());
        let ops: Vec<PatchOp> = serde_json::from_value(
            json!([{ "op": "append", "path": "/a", "value": {"x": 1}}]),
        )
        .unwrap();
        assert!(apply_all(&mut doc, &ops).is_err());
    }
}
