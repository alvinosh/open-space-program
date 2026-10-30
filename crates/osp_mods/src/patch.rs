//! Data patches: change records defined by other mods without replacing them.
//!
//! A patch file (`patches/*.json5`) holds a list of operations:
//!
//! ```json5
//! [
//!   // Deep-merge fields into a record (null deletes a key).
//!   { op: "merge", collection: "engines", target: "K-1 Booster", value: { ignitions: 5 } },
//!   // Set a value at a path (missing objects are created).
//!   { op: "set", collection: "bodies", target: "Earth", path: "atmosphere.height", value: 150000 },
//!   // Remove a record, or a value inside it.
//!   { op: "remove", collection: "craft", target: "pathfinder", path: "stages.name=Payload" },
//!   // Insert into an array (append if `index` is omitted).
//!   { op: "insert", collection: "layouts", target: "flight_hud", path: "#readout.children", value: { widget: "label", text: "hi" } },
//!   // Duplicate a record under a new id, optionally merging changes.
//!   { op: "copy", collection: "engines", target: "K-1 Booster", new_id: "K-1B", value: { thrust_vac: 1.0e6 } },
//! ]
//! ```
//!
//! `target` may use `*` wildcards; `where` restricts targets to records containing the given
//! fields. `pass` ("first", "default", "final") orders patches across all mods, and
//! `optional: true` silences the warning when nothing matches (for compatibility patches).
//!
//! Paths are dot-separated: `key`, array `index` (negative counts from the end),
//! `field=value` (the array element whose `field` equals `value`), and `#id` (the first
//! object anywhere below with that `id`, handy for UI layouts). Wrap a segment in brackets
//! when its value contains dots: `bindings[action=vessel.stage].key`.

use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pass {
    First,
    #[default]
    Default,
    Final,
}

impl Pass {
    pub const ALL: [Pass; 3] = [Pass::First, Pass::Default, Pass::Final];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Merge,
    Set,
    Remove,
    Insert,
    Copy,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub op: Op,
    pub collection: String,
    pub target: String,
    #[serde(default, rename = "where")]
    pub filter: Option<Value>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub index: Option<i64>,
    #[serde(default)]
    pub new_id: Option<String>,
    #[serde(default)]
    pub pass: Pass,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(i64),
    Match(String, String),
    Descendant(String),
    Append,
}

/// Split a path on dots, keeping `[...]` groups (which may contain dots) as their own
/// segments: `bindings[action=vessel.stage].key` -> `bindings`, `action=vessel.stage`, `key`.
fn split_path(path: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        match c {
            '.' => parts.push(std::mem::take(&mut current)),
            '[' => {
                parts.push(std::mem::take(&mut current));
                parts.push(chars.by_ref().take_while(|&c| c != ']').collect());
            }
            c => current.push(c),
        }
    }
    parts.push(current);
    parts.retain(|p| !p.is_empty());
    parts
}

fn parse_path(path: &str) -> Vec<Segment> {
    split_path(path)
        .iter()
        .map(|s| {
            if s == "-" {
                Segment::Append
            } else if let Some(id) = s.strip_prefix('#') {
                Segment::Descendant(id.to_owned())
            } else if let Some((k, v)) = s.split_once('=') {
                Segment::Match(k.to_owned(), v.to_owned())
            } else if let Ok(i) = s.parse::<i64>() {
                Segment::Index(i)
            } else {
                Segment::Key(s.to_owned())
            }
        })
        .collect()
}

fn resolve_index(len: usize, index: i64) -> Option<usize> {
    let i = if index < 0 { len as i64 + index } else { index };
    (0..len as i64).contains(&i).then_some(i as usize)
}

fn value_matches_str(v: &Value, s: &str) -> bool {
    match v {
        Value::String(x) => x == s,
        Value::Number(n) => n.to_string() == s,
        Value::Bool(b) => b.to_string() == s,
        _ => false,
    }
}

fn find_descendant<'a>(value: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    let is_match = value
        .as_object()
        .and_then(|o| o.get("id"))
        .is_some_and(|v| value_matches_str(v, id));
    if is_match {
        return Some(value);
    }
    match value {
        Value::Object(map) => map.values_mut().find_map(|v| find_descendant(v, id)),
        Value::Array(items) => items.iter_mut().find_map(|v| find_descendant(v, id)),
        _ => None,
    }
}

fn step<'a>(value: &'a mut Value, segment: &Segment, create: bool) -> Option<&'a mut Value> {
    match segment {
        Segment::Key(k) => {
            if create && value.is_null() {
                *value = Value::Object(Map::new());
            }
            let map = value.as_object_mut()?;
            if create && !map.contains_key(k) {
                map.insert(k.clone(), Value::Null);
            }
            map.get_mut(k)
        }
        Segment::Index(i) => {
            let items = value.as_array_mut()?;
            let i = resolve_index(items.len(), *i)?;
            items.get_mut(i)
        }
        Segment::Match(k, v) => value
            .as_array_mut()?
            .iter_mut()
            .find(|item| item.get(k).is_some_and(|x| value_matches_str(x, v))),
        Segment::Descendant(id) => match value {
            // Strictly below the current value: a record and its root node may share an id.
            Value::Object(map) => map.values_mut().find_map(|v| find_descendant(v, id)),
            Value::Array(items) => items.iter_mut().find_map(|v| find_descendant(v, id)),
            _ => None,
        },
        Segment::Append => None,
    }
}

fn navigate<'a>(
    mut value: &'a mut Value,
    segments: &[Segment],
    create: bool,
) -> Option<&'a mut Value> {
    for segment in segments {
        value = step(value, segment, create)?;
    }
    Some(value)
}

/// Recursively merge `patch` into `base`. Objects merge key by key; `null` removes a key;
/// anything else (including arrays) replaces.
pub fn deep_merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base), Value::Object(patch)) => {
            for (key, value) in patch {
                if value.is_null() {
                    base.shift_remove(key);
                } else if let Some(existing) = base.get_mut(key) {
                    deep_merge(existing, value);
                } else {
                    base.insert(key.clone(), value.clone());
                }
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

/// Does `record` contain everything in `filter` (recursively, by equality)?
pub fn matches_filter(record: &Value, filter: &Value) -> bool {
    match (record, filter) {
        (Value::Object(r), Value::Object(f)) => f
            .iter()
            .all(|(k, fv)| r.get(k).is_some_and(|rv| matches_filter(rv, fv))),
        (r, f) => r == f,
    }
}

/// `*`-wildcard match.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !text.starts_with(first) || !text[first.len()..].ends_with(last) {
        return false;
    }
    let mut rest = &text[first.len()..text.len() - last.len()];
    for part in &parts[1..parts.len() - 1] {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    true
}

/// Apply an in-place op (everything but `Copy`, and `Remove` without a path, which the
/// registry handles) to one record.
pub fn apply_to_record(record: &mut Value, patch: &Patch) -> Result<(), String> {
    let segments = parse_path(&patch.path);
    let need_value = || {
        patch
            .value
            .clone()
            .ok_or_else(|| "missing `value`".to_owned())
    };
    match patch.op {
        Op::Merge => {
            let target = navigate(record, &segments, false)
                .ok_or_else(|| format!("path `{}` not found", patch.path))?;
            deep_merge(target, &need_value()?);
        }
        Op::Set => {
            let value = need_value()?;
            if let Some((Segment::Append, parent)) = segments.split_last() {
                let array = navigate(record, parent, true)
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| format!("`{}` is not an array", patch.path))?;
                array.push(value);
            } else {
                let target = navigate(record, &segments, true)
                    .ok_or_else(|| format!("cannot create path `{}`", patch.path))?;
                *target = value;
            }
        }
        Op::Remove => {
            let (last, parent) = segments
                .split_last()
                .ok_or_else(|| "remove without a path removes the record".to_owned())?;
            let container = navigate(record, parent, false)
                .ok_or_else(|| format!("path `{}` not found", patch.path))?;
            let removed = match (container, last) {
                (Value::Object(map), Segment::Key(k)) => map.shift_remove(k).is_some(),
                (Value::Array(items), Segment::Index(i)) => resolve_index(items.len(), *i)
                    .map(|i| items.remove(i))
                    .is_some(),
                (Value::Array(items), Segment::Match(k, v)) => {
                    let before = items.len();
                    items.retain(|item| !item.get(k).is_some_and(|x| value_matches_str(x, v)));
                    items.len() != before
                }
                (container, Segment::Descendant(id)) => remove_descendant(container, id),
                _ => false,
            };
            if !removed {
                return Err(format!("nothing to remove at `{}`", patch.path));
            }
        }
        Op::Insert => {
            let value = need_value()?;
            let array = navigate(record, &segments, false)
                .and_then(Value::as_array_mut)
                .ok_or_else(|| format!("`{}` is not an array", patch.path))?;
            match patch.index {
                None => array.push(value),
                Some(i) => {
                    let len = array.len() as i64;
                    let at = if i < 0 { len + 1 + i } else { i }.clamp(0, len) as usize;
                    array.insert(at, value);
                }
            }
        }
        Op::Copy => return Err("copy is handled by the registry".into()),
    }
    Ok(())
}

/// Remove the first object with `id` anywhere below `value` (not `value` itself).
fn remove_descendant(value: &mut Value, id: &str) -> bool {
    let has_id = |v: &Value| {
        v.as_object()
            .and_then(|o| o.get("id"))
            .is_some_and(|x| value_matches_str(x, id))
    };
    match value {
        Value::Array(items) => {
            if let Some(i) = items.iter().position(has_id) {
                items.remove(i);
                return true;
            }
            items.iter_mut().any(|v| remove_descendant(v, id))
        }
        Value::Object(map) => {
            if let Some(key) = map.iter().find(|(_, v)| has_id(v)).map(|(k, _)| k.clone()) {
                map.shift_remove(&key);
                return true;
            }
            map.values_mut().any(|v| remove_descendant(v, id))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn patch(v: Value) -> Patch {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn globs() {
        assert!(glob_match("K-1*", "K-1 Booster"));
        assert!(glob_match("*Booster", "K-1 Booster"));
        assert!(glob_match("*-1*", "K-1 Booster"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("A*", "K-1"));
        assert!(glob_match("exact", "exact"));
    }

    #[test]
    fn merge_set_remove_insert() {
        let mut craft = json!({
            "id": "c",
            "stages": [
                { "name": "Booster", "engines": ["a", "a"] },
                { "name": "Payload", "structure_mass": 100 }
            ]
        });
        fn p(craft: &mut Value, v: Value) -> Result<(), String> {
            apply_to_record(craft, &patch(v))
        }
        p(&mut craft, json!({ "op": "merge", "collection": "craft", "target": "c", "path": "stages.name=Payload", "value": { "structure_mass": 200, "extra": true } })).unwrap();
        assert_eq!(craft["stages"][1]["structure_mass"], 200);
        p(&mut craft, json!({ "op": "set", "collection": "craft", "target": "c", "path": "stages.0.engines.-", "value": "b" })).unwrap();
        assert_eq!(craft["stages"][0]["engines"], json!(["a", "a", "b"]));
        p(&mut craft, json!({ "op": "set", "collection": "craft", "target": "c", "path": "meta.author", "value": "me" })).unwrap();
        assert_eq!(craft["meta"]["author"], "me");
        p(&mut craft, json!({ "op": "insert", "collection": "craft", "target": "c", "path": "stages", "index": 0, "value": { "name": "Kick" } })).unwrap();
        assert_eq!(craft["stages"][0]["name"], "Kick");
        p(&mut craft, json!({ "op": "remove", "collection": "craft", "target": "c", "path": "stages.name=Booster" })).unwrap();
        assert_eq!(craft["stages"].as_array().unwrap().len(), 2);
        p(&mut craft, json!({ "op": "remove", "collection": "craft", "target": "c", "path": "stages.-1.extra" })).unwrap();
        assert!(craft["stages"][1].get("extra").is_none());
        assert!(
            p(
                &mut craft,
                json!({ "op": "remove", "collection": "craft", "target": "c", "path": "nope" })
            )
            .is_err()
        );
    }

    #[test]
    fn merge_null_deletes() {
        let mut v = json!({ "a": 1, "b": { "c": 2, "d": 3 } });
        deep_merge(&mut v, &json!({ "a": null, "b": { "c": 5 } }));
        assert_eq!(v, json!({ "b": { "c": 5, "d": 3 } }));
    }

    #[test]
    fn descendant_paths_for_ui_trees() {
        let mut layout = json!({
            "id": "hud",
            "root": { "widget": "column", "children": [
                { "id": "readout", "widget": "column", "children": [ { "id": "alt", "widget": "label" } ] }
            ]}
        });
        apply_to_record(
            &mut layout,
            &patch(json!({
                "op": "insert", "collection": "layouts", "target": "hud",
                "path": "#readout.children", "value": { "id": "mine", "widget": "label" }
            })),
        )
        .unwrap();
        assert_eq!(layout["root"]["children"][0]["children"][1]["id"], "mine");
        apply_to_record(
            &mut layout,
            &patch(json!({
                "op": "merge", "collection": "layouts", "target": "hud",
                "path": "#alt", "value": { "class": "big" }
            })),
        )
        .unwrap();
        assert_eq!(layout["root"]["children"][0]["children"][0]["class"], "big");
        apply_to_record(
            &mut layout,
            &patch(json!({
                "op": "remove", "collection": "layouts", "target": "hud", "path": "#alt"
            })),
        )
        .unwrap();
        assert_eq!(layout["root"]["children"][0]["children"][0]["id"], "mine");
    }

    #[test]
    fn bracket_segments_allow_dots() {
        let mut keys = json!({ "id": "default", "bindings": [
            { "key": "Space", "action": "vessel.stage" },
            { "key": "KeyM", "action": "view.toggle_map" }
        ]});
        apply_to_record(
            &mut keys,
            &patch(json!({
                "op": "set", "collection": "keybindings", "target": "default",
                "path": "bindings[action=vessel.stage].key", "value": "Enter"
            })),
        )
        .unwrap();
        assert_eq!(keys["bindings"][0]["key"], "Enter");
        assert_eq!(split_path("a[0].b"), ["a", "0", "b"]);
    }

    #[test]
    fn descendant_skips_the_record_itself() {
        let mut layout = json!({
            "id": "toolbar",
            "root": { "id": "toolbar", "widget": "row", "children": [] }
        });
        apply_to_record(
            &mut layout,
            &patch(json!({
                "op": "insert", "collection": "layouts", "target": "toolbar",
                "path": "#toolbar.children", "value": { "widget": "button" }
            })),
        )
        .unwrap();
        assert_eq!(layout["root"]["children"][0]["widget"], "button");
    }

    #[test]
    fn filters() {
        let engine = json!({ "id": "e", "requires_ullage": true, "nested": { "x": 1, "y": 2 } });
        assert!(matches_filter(&engine, &json!({ "requires_ullage": true })));
        assert!(matches_filter(&engine, &json!({ "nested": { "x": 1 } })));
        assert!(!matches_filter(
            &engine,
            &json!({ "requires_ullage": false })
        ));
        assert!(!matches_filter(&engine, &json!({ "missing": 1 })));
    }
}
