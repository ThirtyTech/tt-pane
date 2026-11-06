//! Tolerate settings written by another build, such as a branch with newer
//! actions or options. Unrecognized entries are skipped with a warning when
//! loading, and written back untouched when saving.
use super::{Binding, Config};
use toml::{Table, Value};

/// Settings older builds wrote that this build still migrates on read.
const LEGACY_KEYS: &[&str] = &["focus.include_obscured"];

fn schema() -> Table {
    Table::try_from(Config::default()).expect("default configuration serializes")
}

fn binding_label(entry: &Value) -> String {
    entry
        .get("action")
        .and_then(Value::as_str)
        .map_or_else(|| "entry".to_owned(), |action| format!("`{action}`"))
}

/// Whether `key = value` alone deserializes on top of an otherwise empty config.
fn accepted(key: &str, value: &Value) -> Result<(), toml::de::Error> {
    let mut probe = Table::new();
    probe.insert("version".into(), Value::Integer(1));
    probe.insert("bindings".into(), Value::Array(Vec::new()));
    probe.insert(key.into(), value.clone());
    Value::Table(probe).try_into::<Config>().map(|_| ())
}

/// Remove everything this build cannot read, describing each removal.
pub(super) fn sanitize(mut raw: Table, warnings: &mut Vec<String>) -> Table {
    let schema = schema();
    raw.retain(|key, value| {
        let Some(known) = schema.get(key) else {
            warnings.push(format!("Ignoring unknown setting `{key}`"));
            return false;
        };
        if let (Value::Table(section), Value::Table(known)) = (value, known) {
            section.retain(|name, _| {
                let path = format!("{key}.{name}");
                let ok = known.contains_key(name) || LEGACY_KEYS.contains(&path.as_str());
                if !ok {
                    warnings.push(format!("Ignoring unknown setting `{path}`"));
                }
                ok
            });
        }
        true
    });
    if let Some(Value::Array(bindings)) = raw.get_mut("bindings") {
        bindings.retain(|entry| match entry.clone().try_into::<Binding>() {
            Ok(_) => true,
            Err(error) => {
                warnings.push(format!(
                    "Ignoring binding {}: {}",
                    binding_label(entry),
                    error.message()
                ));
                false
            }
        });
    }
    // Unsupported values (e.g. a newer enum variant) fall back to defaults.
    let keys: Vec<String> = raw
        .keys()
        .filter(|key| !matches!(key.as_str(), "version" | "bindings"))
        .cloned()
        .collect();
    for key in keys {
        let value = raw.get_mut(&key).unwrap();
        if accepted(&key, value).is_ok() {
            continue;
        }
        if let Value::Table(section) = value {
            section.retain(|name, item| {
                let single = Value::Table(Table::from_iter([(name.to_owned(), item.clone())]));
                match accepted(&key, &single) {
                    Ok(()) => true,
                    Err(error) => {
                        warnings.push(format!(
                            "Using the default for `{key}.{name}`: {}",
                            error.message()
                        ));
                        false
                    }
                }
            });
        }
        if let Err(error) = accepted(&key, value) {
            warnings.push(format!(
                "Using the default for `{key}`: {}",
                error.message()
            ));
            raw.remove(&key);
        }
    }
    raw
}

/// Carry entries another build wrote, which this build ignored on load.
pub(super) fn preserve_foreign(existing: &str, ours: &mut Table) {
    let Ok(existing) = existing.parse::<Table>() else {
        return;
    };
    let schema = schema();
    for (key, value) in existing {
        match (schema.get(&key), value) {
            (None, value) => {
                ours.insert(key, value);
            }
            (Some(Value::Table(known)), Value::Table(section)) => {
                if let Some(Value::Table(target)) = ours.get_mut(&key) {
                    for (name, item) in section {
                        let legacy = LEGACY_KEYS.contains(&format!("{key}.{name}").as_str());
                        if !known.contains_key(&name) && !legacy {
                            target.insert(name, item);
                        }
                    }
                }
            }
            (Some(Value::Array(_)), Value::Array(entries)) if key == "bindings" => {
                if let Some(Value::Array(target)) = ours.get_mut("bindings") {
                    target.extend(
                        entries
                            .into_iter()
                            .filter(|entry| entry.clone().try_into::<Binding>().is_err()),
                    );
                }
            }
            _ => {}
        }
    }
}
