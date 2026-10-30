//! The data store UI layouts bind to.
//!
//! The game publishes values under dotted keys each frame (`vessel.altitude`,
//! `orbit.period`, ...). Layouts reference them in text templates (`{vessel.altitude}`),
//! visibility conditions, progress bars and repeated lists. Lists hold records, whose
//! fields can be lists again: `vessel.stages[2].name`, `vessel.sequences[0].items[1].kind`.

use std::collections::{BTreeMap, HashMap};

use bevy::prelude::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum UiValue {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<UiRecord>),
}

/// One item of a list: named fields.
pub type UiRecord = BTreeMap<String, UiValue>;

impl UiValue {
    /// Truthiness for visibility conditions.
    pub fn truthy(&self) -> bool {
        match self {
            UiValue::Null => false,
            UiValue::Bool(b) => *b,
            UiValue::Number(n) => *n != 0.0 && !n.is_nan(),
            UiValue::Text(s) => !s.is_empty(),
            UiValue::List(items) => !items.is_empty(),
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            UiValue::Number(n) => Some(*n),
            UiValue::Bool(b) => Some(f64::from(u8::from(*b))),
            UiValue::Text(s) => s.parse().ok(),
            _ => None,
        }
    }
}

macro_rules! from_number {
    ($($t:ty),*) => {$(
        impl From<$t> for UiValue {
            fn from(v: $t) -> Self {
                UiValue::Number(v as f64)
            }
        }
    )*};
}
from_number!(f64, f32, i32, i64, u32, u64, usize);

impl From<bool> for UiValue {
    fn from(v: bool) -> Self {
        UiValue::Bool(v)
    }
}

impl From<String> for UiValue {
    fn from(v: String) -> Self {
        UiValue::Text(v)
    }
}

impl From<&str> for UiValue {
    fn from(v: &str) -> Self {
        UiValue::Text(v.to_owned())
    }
}

impl<T: Into<UiValue>> From<Option<T>> for UiValue {
    fn from(v: Option<T>) -> Self {
        v.map_or(UiValue::Null, Into::into)
    }
}

impl From<Vec<UiRecord>> for UiValue {
    fn from(v: Vec<UiRecord>) -> Self {
        UiValue::List(v)
    }
}

/// Build a list record: `record([("name", "Booster".into()), ("dv", 4193.0.into())])`.
pub fn record<const N: usize>(fields: [(&str, UiValue); N]) -> UiRecord {
    fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

#[derive(Resource, Default, Debug)]
pub struct UiData {
    values: HashMap<String, UiValue>,
}

impl UiData {
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<UiValue>) {
        self.values.insert(key.into(), value.into());
    }

    pub fn remove(&mut self, key: &str) {
        self.values.remove(key);
    }

    /// Remove every key that starts with `prefix` (a closed layout copy's data).
    pub fn remove_prefix(&mut self, prefix: &str) {
        self.values.retain(|key, _| !key.starts_with(prefix));
    }

    /// Look up `key`, `list[3]`'s `field` (`list[3].field`), and so on into lists held by
    /// record fields (`list[3].items[0].name`). A bare `list[3]` is no value.
    pub fn get(&self, path: &str) -> Option<&UiValue> {
        let Some((base, mut rest)) = path.split_once('[') else {
            return self.values.get(path);
        };
        let mut value = self.values.get(base)?;
        loop {
            let (index, after) = rest.split_once(']')?;
            let UiValue::List(items) = value else {
                return None;
            };
            let item = items.get(index.parse::<usize>().ok()?)?;
            let field = after.strip_prefix('.')?;
            match field.split_once('[') {
                None => return item.get(field),
                Some((name, next)) => {
                    value = item.get(name)?;
                    rest = next;
                }
            }
        }
    }

    pub fn list_len(&self, key: &str) -> usize {
        match self.get(key) {
            Some(UiValue::List(items)) => items.len(),
            _ => 0,
        }
    }

    /// All keys, sorted (for the data inspector).
    pub fn keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self.values.keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    /// Evaluate a condition: a key (true when its value is truthy), negated with `!`, or a
    /// comparison `key == value` / `key != value` against the value's plain text.
    pub fn condition(&self, condition: &str) -> bool {
        for (operator, equal) in [("!=", false), ("==", true)] {
            if let Some((key, expected)) = condition.split_once(operator) {
                let actual = self.get(key.trim()).map_or(String::new(), |v| match v {
                    UiValue::Null => String::new(),
                    other => crate::format::plain(other),
                });
                let expected = expected.trim().trim_matches(['"', '\'']);
                return (actual == expected) == equal;
            }
        }
        let (negate, key) = match condition.trim().strip_prefix('!') {
            Some(key) => (true, key.trim()),
            None => (false, condition.trim()),
        };
        self.get(key).is_some_and(UiValue::truthy) != negate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_conditions() {
        let mut data = UiData::default();
        data.set("vessel.altitude", 1234.5);
        data.set("vessel.landed", false);
        data.set(
            "vessel.stages",
            vec![
                record([("name", "Booster".into())]),
                record([("name", "Upper".into())]),
            ],
        );
        assert_eq!(data.get("vessel.altitude"), Some(&UiValue::Number(1234.5)));
        assert_eq!(
            data.get("vessel.stages[1].name"),
            Some(&UiValue::Text("Upper".into()))
        );
        assert_eq!(data.get("vessel.stages[5].name"), None);
        assert!(data.condition("!vessel.landed"));
        assert!(data.condition("vessel.stages"));
        assert!(!data.condition("missing"));
        assert_eq!(data.list_len("vessel.stages"), 2);

        data.set("sas.mode", "prograde");
        data.set("stage", 2);
        assert!(data.condition("sas.mode == prograde"));
        assert!(data.condition("sas.mode=='prograde'"));
        assert!(!data.condition("sas.mode == retrograde"));
        assert!(data.condition("sas.mode != retrograde"));
        assert!(data.condition("stage == 2"));
        assert!(data.condition("vessel.stages[1].name == Upper"));
        assert!(data.condition("missing == "));
        assert!(!data.condition("missing != "));
    }

    #[test]
    fn lists_inside_records() {
        let mut data = UiData::default();
        let items = vec![
            record([("kind", "engines".into())]),
            record([("kind", "rcs".into())]),
        ];
        data.set(
            "seqs",
            vec![record([("n", 1.into())]), record([("items", items.into())])],
        );
        assert_eq!(data.list_len("seqs[1].items"), 2);
        assert_eq!(
            data.get("seqs[1].items[1].kind"),
            Some(&UiValue::Text("rcs".into()))
        );
        assert_eq!(data.list_len("seqs[0].items"), 0);
        assert_eq!(data.get("seqs[1].items[2].kind"), None);
        assert_eq!(
            data.get("seqs[1].items"),
            Some(&UiValue::List(vec![
                record([("kind", "engines".into())]),
                record([("kind", "rcs".into())]),
            ]))
        );
        assert_eq!(data.get("seqs[1]"), None);
    }
}
