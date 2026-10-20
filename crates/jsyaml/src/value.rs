//! Constructed values: one variant per JS type the built-in tags produce.

/// A loaded value. Mapping keys keep insertion order; JS key reordering (integer-like
/// keys first) happens naturally when the bindings build the object in JS.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Value {
    /// `null`
    #[default]
    Null,
    /// `true` / `false`
    Bool(bool),
    /// Every YAML number is a JS `number`, including `-0`, `NaN` and the infinities.
    Number(f64),
    /// `!!str`
    String(String),
    /// `!!seq`, `!!omap` and `!!pairs` (pairs become 2-element sequences).
    Sequence(Vec<Value>),
    /// `mapTag` / `legacyMapTag`: a plain `{}` with stringified keys. With the `json`
    /// option a duplicate key overwrites the value in place, keeping the first position.
    Object(Vec<(String, Value)>),
    /// `realMapTag`: a JS `Map` with keys of any type.
    Map(Vec<(Value, Value)>),
    /// `!!set`: a JS `Set`.
    Set(Vec<Value>),
    /// `!!binary`: a `Uint8Array`.
    Binary(Vec<u8>),
    /// `!!timestamp`: a `Date`, as milliseconds since the Unix epoch.
    Date(f64),
    /// A reference to [`Document::shared`]`[i]`. Used for every occurrence of an anchored
    /// node that JS would share by identity (collections, `Date`, `Uint8Array`) and is
    /// aliased at least once; anchored scalars are copied instead.
    Shared(usize),
}

impl Value {
    /// The JS `typeof`-level name, used in messages and tests.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Sequence(_) => "array",
            Value::Object(_) => "object",
            Value::Map(_) => "Map",
            Value::Set(_) => "Set",
            Value::Binary(_) => "Uint8Array",
            Value::Date(_) => "Date",
            Value::Shared(_) => "shared",
        }
    }
}

/// One loaded document.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    /// The document's value.
    pub root: Value,
    /// Nodes referenced by [`Value::Shared`]. A shared node may reference earlier ones
    /// (or itself, for recursive aliases inside a collection).
    pub shared: Vec<Value>,
}

#[cfg(feature = "serde")]
mod serde_impl {
    //! Serializes to plain JSON-like data for the CLI. Not lossless: `Map` keys are
    //! stringified, `Date` becomes milliseconds, `Binary` a byte array, and `Shared`
    //! nodes are expanded (recursive ones become `null`).
    use super::{Document, Value};
    use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};

    struct Node<'a> {
        doc: &'a Document,
        value: &'a Value,
        depth: usize,
    }

    impl Node<'_> {
        fn child<'b>(&'b self, value: &'b Value) -> Node<'b> {
            Node {
                doc: self.doc,
                value,
                depth: self.depth + 1,
            }
        }
    }

    impl Serialize for Node<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            match self.value {
                Value::Null => s.serialize_unit(),
                Value::Bool(b) => s.serialize_bool(*b),
                Value::Number(n) | Value::Date(n) => s.serialize_f64(*n),
                Value::String(v) => s.serialize_str(v),
                Value::Binary(bytes) => s.collect_seq(bytes),
                Value::Sequence(items) | Value::Set(items) => {
                    let mut seq = s.serialize_seq(Some(items.len()))?;
                    for item in items {
                        seq.serialize_element(&self.child(item))?;
                    }
                    seq.end()
                }
                Value::Object(entries) => {
                    let mut map = s.serialize_map(Some(entries.len()))?;
                    for (k, v) in entries {
                        map.serialize_entry(k, &self.child(v))?;
                    }
                    map.end()
                }
                Value::Map(entries) => {
                    let mut map = s.serialize_map(Some(entries.len()))?;
                    for (k, v) in entries {
                        let key = serde_json_free_key(k);
                        map.serialize_entry(&key, &self.child(v))?;
                    }
                    map.end()
                }
                Value::Shared(i) => match self.doc.shared.get(*i) {
                    // Cycles: a shared node can't be nested deeper than the document has nodes.
                    Some(v) if self.depth <= self.doc.shared.len() + 64 => {
                        self.child(v).serialize(s)
                    }
                    _ => s.serialize_unit(),
                },
            }
        }
    }

    fn serde_json_free_key(key: &Value) -> String {
        match key {
            Value::String(v) => v.clone(),
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            other => format!("[{}]", other.type_name()),
        }
    }

    impl Serialize for Document {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            Node {
                doc: self,
                value: &self.root,
                depth: 0,
            }
            .serialize(s)
        }
    }
}
