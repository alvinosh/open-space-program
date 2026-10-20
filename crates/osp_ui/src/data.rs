//! The data store UI layouts bind to.
//!
//! The game publishes values under dotted keys each frame (`vessel.altitude`,
//! `orbit.period`, ...). Layouts reference them in text templates (`{vessel.altitude}`),
//! visibility conditions, progress bars and repeated lists. Lists hold records:
//! `vessel.stages[2].name`.

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

    /// Look up `key`, `list[3]` or `list[3].field`.
    pub fn get(&self, path: &str) -> Option<&UiValue> {
        let Some((base, rest)) = path.split_once('[') else {
            return self.values.get(path);
        };
        let (index, field) = rest.split_once(']')?;
        let index: usize = index.parse().ok()?;
        let UiValue::List(items) = self.values.get(base)? else {
            return None;
        };
        let item = items.get(index)?;
        match field.strip_prefix('.') {
            Some(field) => item.get(field),
            None if field.is_empty() => None,
            None => None,
        }
    }

    pub fn list_len(&self, key: &str) -> usize {
        match self.values.get(key) {
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
}
