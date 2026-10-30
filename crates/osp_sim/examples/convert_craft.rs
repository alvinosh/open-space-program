//! Converts craft files written as stacks of stages into plain parts (see
//! `osp_sim::staged`), in place. Files already written as parts are left alone, unless
//! `--format` is given: then they (and assembly files, lists of assemblies) are written
//! again in the same style, so a craft built by hand or by a script as plain JSON reads
//! like the rest. `--fit-fairings` also fits each craft's fairings to what they hold
//! (`CraftDef::fit_fairings`). `--json` prints each file as JSON instead, for such scripts.
//! The comment at the top of a file is kept; comments inside it are not.
//!
//! ```sh
//! cargo run -p osp_sim --example convert_craft --release -- \
//!     [--format | --fit-fairings | --json] <files...>
//! ```

use std::path::Path;

use osp_sim::{
    data::{
        AssemblyDef, BatteryDef, CraftDef, EngineDef, LiveryDef, MaterialDef, PartCatalog, PodDef,
        RcsDef, ReactionWheelDef, ResourceDef, StagedCraftDef, TankTypeDef,
    },
    staged::convert,
};
use serde_json::Value;

/// Widest a line is written, characters.
const WIDTH: usize = 92;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let mut diagnostics = Vec::new();
    let catalog = PartCatalog {
        engines: registry.all::<EngineDef>("engines", &mut diagnostics),
        resources: registry.all::<ResourceDef>("resources", &mut diagnostics),
        materials: registry.all::<MaterialDef>("materials", &mut diagnostics),
        tank_types: registry.all::<TankTypeDef>("tank_types", &mut diagnostics),
        reaction_wheels: registry.all::<ReactionWheelDef>("reaction_wheels", &mut diagnostics),
        rcs: registry.all::<RcsDef>("rcs", &mut diagnostics),
        pods: registry.all::<PodDef>("pods", &mut diagnostics),
        batteries: registry.all::<BatteryDef>("batteries", &mut diagnostics),
        liveries: registry.all::<LiveryDef>("liveries", &mut diagnostics),
    };
    let mut failed = 0;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let (fit, json) = (flag("--fit-fairings"), flag("--json"));
    let format = fit || flag("--format");
    for path in args.into_iter().filter(|a| !a.starts_with("--")) {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let value: Value = json5::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"));
        let header: String = text
            .lines()
            .take_while(|l| l.trim_start().starts_with("//"))
            .map(|l| format!("{l}\n"))
            .collect();
        if json {
            println!("{value}");
            continue;
        }
        if value.get("stages").is_none() {
            if format {
                // Read as what it is, so that it is checked and written the way it loads.
                let typed = if value.is_array() {
                    let list: Vec<AssemblyDef> =
                        serde_json::from_value(value).unwrap_or_else(|e| panic!("{path}: {e}"));
                    serde_json::to_value(list)
                } else {
                    let mut craft: CraftDef =
                        serde_json::from_value(value).unwrap_or_else(|e| panic!("{path}: {e}"));
                    if fit {
                        match craft.fit_fairings(&catalog) {
                            Ok(panels) if !panels.is_empty() => {
                                println!("{path}: fitted {}", panels.join(", "));
                            }
                            Ok(_) => {}
                            Err(e) => {
                                eprintln!("{path}: {e}");
                                failed += 1;
                            }
                        }
                    }
                    serde_json::to_value(craft)
                };
                let mut out = header;
                write(&typed.unwrap(), 0, &mut out);
                out.push('\n');
                std::fs::write(&path, out).unwrap_or_else(|e| panic!("{path}: {e}"));
                println!("formatted {path}");
            }
            continue;
        }
        let staged: StagedCraftDef =
            serde_json::from_value(value).unwrap_or_else(|e| panic!("{path}: {e}"));
        match convert(&staged, &catalog) {
            Ok(craft) => {
                let mut out = header;
                write(&serde_json::to_value(&craft).unwrap(), 0, &mut out);
                out.push('\n');
                std::fs::write(&path, out).unwrap_or_else(|e| panic!("{path}: {e}"));
                println!("converted {path}");
            }
            Err(e) => {
                eprintln!("{path}: {e}");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        std::process::exit(1);
    }
}

/// `value` as JSON5, nested `indent` deep: on one line where it fits (and is not the craft
/// or its list of parts), otherwise one entry a line.
fn write(value: &Value, indent: usize, out: &mut String) {
    write_at(value, indent, indent, false, out);
}

/// `value` written from column `column`, its entries indented from `indent`.
fn write_at(value: &Value, indent: usize, column: usize, spread: bool, out: &mut String) {
    let line = inline(value);
    // The comma after it counts too.
    let fits = !spread && column + line.len() < WIDTH;
    match value {
        Value::Object(map) if !fits || map.contains_key("parts") => {
            out.push_str("{\n");
            for (key, item) in map {
                let key = key_text(key);
                out.push_str(&" ".repeat(indent + 2));
                out.push_str(&key);
                out.push_str(": ");
                let column = indent + 2 + key.len() + 2;
                write_at(item, indent + 2, column, key == "parts", out);
                out.push_str(",\n");
            }
            out.push_str(&" ".repeat(indent));
            out.push('}');
        }
        Value::Array(items) if !fits => {
            out.push_str("[\n");
            for item in items {
                out.push_str(&" ".repeat(indent + 2));
                write(item, indent + 2, out);
                out.push_str(",\n");
            }
            out.push_str(&" ".repeat(indent));
            out.push(']');
        }
        _ => out.push_str(&line),
    }
}

/// `value` as JSON5 on one line.
fn inline(value: &Value) -> String {
    match value {
        Value::Object(map) if map.is_empty() => "{}".into(),
        Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}: {}", key_text(k), inline(v)))
                .collect();
            format!("{{ {} }}", entries.join(", "))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(inline).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Number(n) => match n.as_f64() {
            Some(x) if x.fract() == 0.0 && x.abs() < 1e15 => format!("{}", x as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

fn key_text(key: &str) -> String {
    let bare = key
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if bare {
        key.into()
    } else {
        serde_json::to_string(key).unwrap()
    }
}
