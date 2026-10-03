//! Developer-only TOML owner for release checkout version injection.
use serde_json::{Map, Number, Value as Json};
use std::io::{self, Read};
use toml_edit::{DocumentMut, Item, Table, Value};

fn table_json(table: &Table) -> Result<Json, String> {
    let mut result = Map::new();
    for (key, item) in table.iter() {
        if !item.is_none() {
            result.insert(key.to_owned(), item_json(item)?);
        }
    }
    Ok(Json::Object(result))
}

fn item_json(item: &Item) -> Result<Json, String> {
    match item {
        Item::Value(value) => value_json(value),
        Item::Table(table) => table_json(table),
        Item::ArrayOfTables(tables) => tables.iter().map(table_json).collect(),
        Item::None => Err("Missing TOML item".into()),
    }
}

fn value_json(value: &Value) -> Result<Json, String> {
    Ok(match value {
        Value::String(v) => Json::String(v.value().clone()),
        Value::Integer(v) => Json::Number((*v.value()).into()),
        Value::Float(v) => {
            Json::Number(Number::from_f64(*v.value()).ok_or("Non-finite TOML float")?)
        }
        Value::Boolean(v) => Json::Bool(*v.value()),
        Value::Datetime(v) => Json::String(v.value().to_string()),
        Value::Array(values) => values.iter().map(value_json).collect::<Result<_, _>>()?,
        Value::InlineTable(table) => {
            let mut result = Map::new();
            for (key, value) in table.iter() {
                result.insert(key.to_owned(), value_json(value)?);
            }
            Json::Object(result)
        }
    })
}

fn edit(source: &str, section: &str, old: &str, new: &str) -> Result<String, String> {
    let mut document = source.parse::<DocumentMut>().map_err(|e| e.to_string())?;
    let target = match section {
        "workspace.package" => document
            .get_mut("workspace")
            .and_then(|v| v.get_mut("package"))
            .and_then(|v| v.get_mut("version")),
        "package" => document
            .get_mut("package")
            .and_then(|v| v.get_mut("version")),
        _ => return Err("Unsupported Cargo version section".into()),
    }
    .and_then(Item::as_value_mut)
    .ok_or("Missing Cargo version declaration")?;
    if target.as_str() != Some(old) {
        return Err("Cargo version differs from captured source".into());
    }
    let decor = target.decor().clone();
    *target = Value::from(new);
    *target.decor_mut() = decor;
    Ok(document.to_string())
}

fn run(args: &[String], source: &str) -> Result<String, String> {
    match args {
        [action] if action == "parse" => {
            let document = source.parse::<DocumentMut>().map_err(|e| e.to_string())?;
            table_json(document.as_table()).map(|value| value.to_string())
        }
        [action, section, old, new] if action == "edit" => edit(source, section, old, new),
        _ => Err("Usage: release-version parse | edit <package|workspace.package> <old> <new>; TOML on stdin".into()),
    }
}

fn main() {
    let result = (|| {
        let mut source = String::new();
        io::stdin()
            .take(64 * 1024 * 1024 + 1)
            .read_to_string(&mut source)
            .map_err(|e| e.to_string())?;
        if source.len() > 64 * 1024 * 1024 {
            return Err("TOML input exceeds limit".into());
        }
        run(&std::env::args().skip(1).collect::<Vec<_>>(), &source)
    })();
    match result {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_comments_spacing_and_unrelated_fields() {
        let source =
            "# owner\n[workspace.package]\nversion  =  '5.0.0-dev' # keep\nedition = \"2024\"\n";
        assert_eq!(
            edit(source, "workspace.package", "5.0.0-dev", "5.0.0-alpha.9").unwrap(),
            source.replace("'5.0.0-dev'", "\"5.0.0-alpha.9\"")
        );
    }

    #[test]
    fn parses_cargo_tables_arrays_and_inline_values() {
        let args = ["parse".into()];
        let result = run(&args, "version = 4\n[[package]]\nname = \"local\"\nversion = \"1.0.0\"\ndependencies = [\"core 1.0.0\"]\nmetadata = { enabled = true }\n").unwrap();
        let json: Json = serde_json::from_str(&result).unwrap();
        assert_eq!(json["package"][0]["metadata"]["enabled"], true);
        assert_eq!(json["package"][0]["dependencies"][0], "core 1.0.0");
    }

    #[test]
    fn rejects_missing_mismatched_or_duplicate_versions() {
        for source in [
            "[package]\nname = \"local\"\n",
            "[package]\nversion = \"2.0.0\"\n",
            "[package]\nversion = \"1.0.0\"\nversion = \"1.0.0\"\n",
            "[package]\nversion.workspace = true\n",
        ] {
            assert!(edit(source, "package", "1.0.0", "1.0.1").is_err());
        }
    }
}
