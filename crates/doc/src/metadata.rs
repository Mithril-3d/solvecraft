//! Design properties (#1015): the design's optional top-level `metadata` object.
//!
//! SolidCraft reads a few well-known, unprefixed text keys from it (project, part number,
//! revision, author, organization, description, licence) and keeps every other key exactly as
//! it was through open, edit and save. Other tools keep their own data under one reverse-DNS
//! key each (`com.example.tool`), holding an object.
//!
//! Reading never validates: a well-known key holding something other than text is kept as it
//! is and reads as absent, so a file another tool wrote always opens. Writing through
//! [`Metadata::set`] (the `document.metadata.set` command) checks the key, the value's kind and
//! its size, so nothing SolidCraft writes is malformed.
//!
//! The map is stored as is: typed fields would refuse a design another tool had written
//! `"author": 42` into. Everything under `metadata` is informational, never geometry, so keeping
//! unknown keys can't leave stale model data behind (unlike keeping unknown top-level fields).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{DocError, Result};

pub const PROJECT: &str = "project";
pub const PART_NUMBER: &str = "part_number";
pub const REVISION: &str = "revision";
pub const AUTHOR: &str = "author";
pub const ORGANIZATION: &str = "organization";
pub const DESCRIPTION: &str = "description";
pub const LICENSE: &str = "license";

/// SolidCraft's own keys, in the order the Design Properties dialog shows them. Every other
/// unprefixed key is reserved for SolidCraft too.
pub const WELL_KNOWN: [&str; 7] = [PROJECT, PART_NUMBER, REVISION, AUTHOR, ORGANIZATION, DESCRIPTION, LICENSE];

/// Longest key, in bytes.
pub const MAX_KEY_BYTES: usize = 128;
/// Longest well-known text value, in bytes.
pub const MAX_TEXT_BYTES: usize = 4096;
/// Largest namespaced value, as compact JSON.
pub const MAX_VALUE_BYTES: usize = 64 * 1024;
/// Deepest nesting of a namespaced value.
pub const MAX_VALUE_DEPTH: usize = 32;
/// Most keys, and largest object as compact JSON, that a change may grow the metadata to.
pub const MAX_KEYS: usize = 256;
pub const MAX_TOTAL_BYTES: usize = 256 * 1024;

/// The design's `metadata` object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Metadata(pub Map<String, Value>);

/// A key SolidCraft reads as a design property.
pub fn is_well_known(key: &str) -> bool {
    WELL_KNOWN.contains(&key)
}

/// A reverse-DNS key another tool keeps its data under: two or more dot-separated labels of
/// ASCII letters, digits, `-` and `_` (`com.example.tool`, `org.printfarm`).
pub fn is_namespaced(key: &str) -> bool {
    key.len() <= MAX_KEY_BYTES
        && key.split('.').count() >= 2
        && key.split('.').all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
}

fn depth(v: &Value) -> usize {
    // Iterative, so a hostile value can't overflow the stack here.
    let mut deepest = 0;
    let mut stack = vec![(v, 1usize)];
    while let Some((v, d)) = stack.pop() {
        deepest = deepest.max(d);
        if d > MAX_VALUE_DEPTH {
            break;
        }
        match v {
            Value::Array(a) => stack.extend(a.iter().map(|x| (x, d + 1))),
            Value::Object(m) => stack.extend(m.values().map(|x| (x, d + 1))),
            _ => {}
        }
    }
    deepest
}

fn json_len(v: &impl Serialize) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

fn invalid(msg: String) -> DocError {
    DocError::Invalid(msg)
}

impl Metadata {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    /// A text value: `None` when the key is missing, empty or holds something other than text.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key)?.as_str().filter(|s| !s.is_empty())
    }

    pub fn project(&self) -> Option<&str> {
        self.text(PROJECT)
    }
    pub fn part_number(&self) -> Option<&str> {
        self.text(PART_NUMBER)
    }
    pub fn revision(&self) -> Option<&str> {
        self.text(REVISION)
    }
    pub fn author(&self) -> Option<&str> {
        self.text(AUTHOR)
    }
    pub fn organization(&self) -> Option<&str> {
        self.text(ORGANIZATION)
    }
    pub fn description(&self) -> Option<&str> {
        self.text(DESCRIPTION)
    }
    pub fn license(&self) -> Option<&str> {
        self.text(LICENSE)
    }

    /// The keys SolidCraft doesn't read (other tools' data, or keys a newer SolidCraft added),
    /// kept as they are.
    pub fn others(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.0.iter().filter(|(k, _)| !is_well_known(k))
    }

    /// Set `key` to `value` after checking both; `null` (or empty text for a well-known key)
    /// removes it. Returns whether the metadata changed.
    pub fn set(&mut self, key: &str, value: Value) -> Result<bool> {
        let value = check(key, value)?;
        let Some(value) = value else { return Ok(self.remove(key)) };
        if self.0.get(key) == Some(&value) {
            return Ok(false);
        }
        let (keys, bytes) = (self.0.len(), json_len(&self.0));
        let old = self.0.insert(key.to_string(), value);
        // Growing past the limits is refused; a file already past them can still shrink.
        let (new_keys, new_bytes) = (self.0.len(), json_len(&self.0));
        if (new_keys > MAX_KEYS && new_keys > keys) || (new_bytes > MAX_TOTAL_BYTES && new_bytes > bytes) {
            match old {
                Some(old) => self.0.insert(key.to_string(), old),
                None => self.0.remove(key),
            };
            return Err(invalid(format!("design properties are limited to {MAX_KEYS} keys and {} KiB", MAX_TOTAL_BYTES / 1024)));
        }
        Ok(true)
    }

    /// Remove `key`, whatever it holds. Returns whether it was there.
    pub fn remove(&mut self, key: &str) -> bool {
        self.0.remove(key).is_some()
    }
}

/// `value` as `key` may hold it (`None`: remove the key), or why it can't.
fn check(key: &str, value: Value) -> Result<Option<Value>> {
    if is_well_known(key) {
        return match value {
            Value::Null => Ok(None),
            Value::String(s) if s.is_empty() => Ok(None),
            Value::String(s) => {
                if s.len() > MAX_TEXT_BYTES {
                    return Err(invalid(format!("`{key}` is longer than {MAX_TEXT_BYTES} bytes")));
                }
                if s.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r')) {
                    return Err(invalid(format!("`{key}` has a control character")));
                }
                Ok(Some(Value::String(s)))
            }
            _ => Err(invalid(format!("`{key}` takes text"))),
        };
    }
    if is_namespaced(key) {
        return match value {
            Value::Null => Ok(None),
            Value::Object(m) => {
                let v = Value::Object(m);
                if depth(&v) > MAX_VALUE_DEPTH {
                    return Err(invalid(format!("`{key}` is nested more than {MAX_VALUE_DEPTH} levels deep")));
                }
                if json_len(&v) > MAX_VALUE_BYTES {
                    return Err(invalid(format!("`{key}` is larger than {} KiB", MAX_VALUE_BYTES / 1024)));
                }
                Ok(Some(v))
            }
            _ => Err(invalid(format!("`{key}` must hold an object (a tool keeps its own data as an object under its reverse-DNS key)"))),
        };
    }
    let shown: String = key.chars().take(64).collect();
    Err(invalid(format!(
        "`{shown}` is not a design property: SolidCraft's are {}; another tool's data goes under a reverse-DNS key such as `com.example.tool`",
        WELL_KNOWN.join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn well_known_keys_read_as_text_and_ignore_other_kinds() {
        let m: Metadata = serde_json::from_value(json!({"author": "Jane Doe", "revision": 3, "project": "", "license": null})).unwrap();
        assert_eq!(m.author(), Some("Jane Doe"));
        assert_eq!(m.revision(), None, "a number is kept but is not a revision");
        assert_eq!(m.project(), None, "empty text reads as absent");
        assert_eq!(m.license(), None);
        assert_eq!(m.get("revision"), Some(&json!(3)), "kept as it was");
    }

    #[test]
    fn set_checks_keys_kinds_and_sizes() {
        let mut m = Metadata::default();
        assert!(m.set("author", json!("Jane")).unwrap());
        assert!(!m.set("author", json!("Jane")).unwrap(), "the same value is no change");
        assert!(m.set("com.acme.tool", json!({"shop_id": 88123, "tags": ["a", null, 1.5]})).unwrap());
        assert!(m.set("author", json!(42)).is_err(), "well-known keys take text");
        assert!(m.set("author", json!("a\u{7}b")).is_err(), "no control characters");
        assert!(m.set("description", json!("two\nlines\tand a tab")).unwrap());
        assert!(m.set("com.acme.tool", json!("text")).is_err(), "a tool's key holds an object");
        assert!(m.set("colour", json!("red")).is_err(), "unprefixed keys are reserved");
        assert!(m.set("", json!({})).is_err());
        for bad in ["com.", ".com", "com..x", "com.ex ample", "com.é", "single"] {
            assert!(m.set(bad, json!({})).is_err(), "{bad}");
        }
        assert!(m.set("author", json!("x".repeat(MAX_TEXT_BYTES + 1))).is_err());
        assert!(m.set("org.big", json!({"a": "x".repeat(MAX_VALUE_BYTES)})).is_err());
        let mut deep = json!({});
        for _ in 0..MAX_VALUE_DEPTH {
            deep = json!({ "a": deep });
        }
        assert!(m.set("org.deep", deep).is_err());
        assert_eq!(m.author(), Some("Jane"), "a refused value leaves the old one");
        // Null and empty text remove.
        assert!(m.set("author", json!("")).unwrap());
        assert!(m.set("com.acme.tool", Value::Null).unwrap());
        assert!(!m.set("com.acme.tool", Value::Null).unwrap());
        assert_eq!(m.0.keys().collect::<Vec<_>>(), ["description"]);
    }

    #[test]
    fn the_limits_refuse_growth_but_let_an_oversized_file_shrink() {
        let mut m = Metadata::default();
        for i in 0..MAX_KEYS {
            m.set(&format!("org.k{i}"), json!({})).unwrap();
        }
        assert!(m.set("org.one_more", json!({})).is_err());
        assert_eq!(m.0.len(), MAX_KEYS);
        // A file written past the limits by another tool still opens and can be cut down.
        let mut big = Metadata::default();
        for i in 0..MAX_KEYS + 10 {
            big.0.insert(format!("org.k{i}"), json!({}));
        }
        assert!(big.set("org.k0", Value::Null).unwrap());
        assert!(big.set("author", json!("Jane")).is_err(), "still over: no new keys");
        assert!(big.set("org.k1", json!({"replaced": true})).is_ok(), "the count doesn't grow");
    }
}
