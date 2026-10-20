//! Text templates and value formatters.
//!
//! A template is literal text with bindings: `"Alt {vessel.altitude|distance}"`.
//! `{key}` prints a value with the default format, `{key|name}` or `{key|name:arg}` runs a
//! registered formatter, and `{{` / `}}` print literal braces.
//!
//! Built-in formatters: `fixed:N`, `int`, `percent[:N]`, `distance`, `speed`, `mass`,
//! `force`, `torque`, `pressure`, `duration`, `countdown`, `degrees[:N]`, `sci[:N]`,
//! `upper`, `lower`, `yesno`.
//! The game (or a plugin) can register more with [`Formatters::register`].

use std::{collections::HashMap, fmt::Write};

use bevy::prelude::*;

use crate::data::{UiData, UiValue};

pub type Formatter = fn(&UiValue, Option<&str>) -> String;

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Literal(String),
    Binding {
        key: String,
        format: Option<String>,
        arg: Option<String>,
    },
}

/// A parsed text template.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Template {
    pieces: Vec<Piece>,
}

impl Template {
    pub fn parse(source: &str) -> Self {
        let mut pieces = Vec::new();
        let mut literal = String::new();
        let mut chars = source.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    literal.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    literal.push('}');
                }
                '{' => {
                    let mut inner = String::new();
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                        inner.push(c);
                    }
                    if !literal.is_empty() {
                        pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                    }
                    let (key, spec) = match inner.split_once('|') {
                        Some((k, s)) => (k.trim(), Some(s.trim())),
                        None => (inner.trim(), None),
                    };
                    let (format, arg) = match spec.map(|s| s.split_once(':').unwrap_or((s, ""))) {
                        Some((f, a)) => (Some(f.to_owned()), (!a.is_empty()).then(|| a.to_owned())),
                        None => (None, None),
                    };
                    pieces.push(Piece::Binding {
                        key: key.to_owned(),
                        format,
                        arg,
                    });
                }
                c => literal.push(c),
            }
        }
        if !literal.is_empty() {
            pieces.push(Piece::Literal(literal));
        }
        Self { pieces }
    }

    /// True if the template has no bindings (its text never changes).
    pub fn is_static(&self) -> bool {
        self.pieces.iter().all(|p| matches!(p, Piece::Literal(_)))
    }

    pub fn render(&self, data: &UiData, formatters: &Formatters) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Literal(s) => out.push_str(s),
                Piece::Binding { key, format, arg } => {
                    let value = data.get(key).cloned().unwrap_or_default();
                    out.push_str(&formatters.apply(format.as_deref(), &value, arg.as_deref()));
                }
            }
        }
        out
    }
}

#[derive(Resource, Clone)]
pub struct Formatters {
    map: HashMap<String, Formatter>,
}

impl Default for Formatters {
    fn default() -> Self {
        let mut f = Self {
            map: HashMap::new(),
        };
        f.register("fixed", fixed);
        f.register("int", |v, _| number(v, |n| format!("{n:.0}")));
        f.register("percent", |v, a| {
            let digits = a.and_then(|a| a.parse().ok()).unwrap_or(0);
            number(v, |n| format!("{:.*}%", digits, n * 100.0))
        });
        f.register("distance", |v, _| number(v, distance));
        f.register("speed", |v, _| {
            number(v, |n| {
                if n.abs() >= 1e4 {
                    format!("{:.2} km/s", n / 1e3)
                } else {
                    format!("{n:.1} m/s")
                }
            })
        });
        f.register("mass", |v, _| {
            number(v, |n| scaled(n, &[(1e3, "t"), (1.0, "kg")], 2))
        });
        f.register("force", |v, _| {
            number(v, |n| scaled(n, &[(1e6, "MN"), (1e3, "kN"), (1.0, "N")], 1))
        });
        f.register("torque", |v, _| {
            number(v, |n| {
                scaled(n, &[(1e6, "MN m"), (1e3, "kN m"), (1.0, "N m")], 1)
            })
        });
        f.register("pressure", |v, _| {
            number(v, |n| scaled(n, &[(1e3, "kPa"), (1.0, "Pa")], 2))
        });
        f.register("duration", |v, _| number(v, duration));
        // Time until something: T-00:05:00 before it, T+00:00:12 after.
        f.register("countdown", |v, _| {
            number(v, |n| {
                let sign = if n < 0.0 { "T+" } else { "T-" };
                format!("{sign}{}", duration(n.abs()))
            })
        });
        f.register("degrees", |v, a| {
            let digits = a.and_then(|a| a.parse().ok()).unwrap_or(2);
            number(v, |n| format!("{:.*} deg", digits, n))
        });
        f.register("sci", |v, a| {
            let digits = a.and_then(|a| a.parse().ok()).unwrap_or(3);
            number(v, |n| format!("{:.*e}", digits, n))
        });
        f.register("upper", |v, _| plain(v).to_uppercase());
        f.register("lower", |v, _| plain(v).to_lowercase());
        f.register("yesno", |v, _| {
            if v.truthy() { "yes" } else { "no" }.to_owned()
        });
        f
    }
}

impl Formatters {
    pub fn register(&mut self, name: &str, formatter: Formatter) {
        self.map.insert(name.to_owned(), formatter);
    }

    pub fn apply(&self, name: Option<&str>, value: &UiValue, arg: Option<&str>) -> String {
        match name {
            None => plain(value),
            Some(name) => match self.map.get(name) {
                Some(f) => f(value, arg),
                None => format!("<?{name}>"),
            },
        }
    }
}

/// Default formatting: integers without decimals, other numbers with two.
pub fn plain(value: &UiValue) -> String {
    match value {
        UiValue::Null => "-".into(),
        UiValue::Bool(b) => b.to_string(),
        UiValue::Number(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{n:.0}"),
        UiValue::Number(n) => format!("{n:.2}"),
        UiValue::Text(s) => s.clone(),
        UiValue::List(items) => format!("[{} items]", items.len()),
    }
}

fn number(value: &UiValue, f: impl Fn(f64) -> String) -> String {
    match value.as_number() {
        Some(n) if n.is_finite() => f(n),
        Some(n) if n.is_nan() => "-".into(),
        Some(n) => if n > 0.0 { "inf" } else { "-inf" }.into(),
        None => plain(value),
    }
}

fn fixed(value: &UiValue, arg: Option<&str>) -> String {
    let digits = arg.and_then(|a| a.parse().ok()).unwrap_or(2);
    number(value, |n| format!("{:.*}", digits, n))
}

fn scaled(n: f64, units: &[(f64, &str)], digits: usize) -> String {
    let (scale, unit) = units
        .iter()
        .find(|(scale, _)| n.abs() >= *scale)
        .unwrap_or(units.last().unwrap());
    format!("{:.*} {unit}", digits, n / scale)
}

pub fn distance(m: f64) -> String {
    const AU: f64 = 1.495_978_707e11;
    let a = m.abs();
    if a < 1e4 {
        format!("{m:.0} m")
    } else if a < 1e7 {
        format!("{:.2} km", m / 1e3)
    } else if a < 0.1 * AU {
        format!("{:.0} km", m / 1e3)
    } else {
        format!("{:.3} AU", m / AU)
    }
}

pub fn duration(seconds: f64) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let total = seconds.abs() as u64;
    let (y, d) = (total / 31_557_600, total % 31_557_600 / 86_400);
    let (h, m, s) = (total / 3_600 % 24, total / 60 % 60, total % 60);
    let mut out = String::from(sign);
    if y > 0 {
        let _ = write!(out, "{y}y ");
    }
    if y > 0 || d > 0 {
        let _ = write!(out, "{d}d ");
    }
    let _ = write!(out, "{h:02}:{m:02}:{s:02}");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_render_with_formatters() {
        let mut data = UiData::default();
        data.set("a", 12_345.678);
        data.set("name", "Pathfinder");
        data.set("frac", 0.4567);
        let f = Formatters::default();
        let t = Template::parse(
            "{name}: {a|distance}, {a|fixed:1}, {frac|percent:1} {{literal}} {missing}",
        );
        assert_eq!(
            t.render(&data, &f),
            "Pathfinder: 12.35 km, 12345.7, 45.7% {literal} -"
        );
        assert!(Template::parse("static").is_static());
        assert_eq!(Template::parse("{a|nope}").render(&data, &f), "<?nope>");
    }

    #[test]
    fn unit_formatters() {
        assert_eq!(duration(90_061.0), "1d 01:01:01");
        assert_eq!(distance(384_400_000.0), "384400 km");
        assert_eq!(
            scaled(7_459_000.0, &[(1e6, "MN"), (1e3, "kN"), (1.0, "N")], 1),
            "7.5 MN"
        );
    }
}
