//! The JSON fast path: documents as a JSON array, or `None` if anything in them has no
//! JSON form (see [`crate::Loaded`]).

use std::fmt::Write as _;

use jsyaml::{Document, Value};

pub(crate) fn encode(documents: &[Document]) -> Option<String> {
    let mut out = String::with_capacity(64);
    out.push('[');
    for (i, doc) in documents.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        value(&mut out, &doc.root)?;
    }
    out.push(']');
    Some(out)
}

fn value(out: &mut String, v: &Value) -> Option<()> {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        // JS's own number formatting, as JSON.stringify writes it.
        Value::Number(n) if n.is_finite() && !(*n == 0.0 && n.is_sign_negative()) => {
            out.push_str(&jsyaml::js_number_to_string(*n));
        }
        Value::String(s) => string(out, s),
        Value::Sequence(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(entries) => {
            out.push('{');
            for (i, (k, item)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                string(out, k);
                out.push(':');
                value(out, item)?;
            }
            out.push('}');
        }
        Value::Number(_)
        | Value::Map(_)
        | Value::Set(_)
        | Value::Binary(_)
        | Value::Date(_)
        | Value::Shared(_) => return None,
    }
    Some(())
}

fn string(out: &mut String, s: &str) {
    out.push('"');
    let mut start = 0;
    for (i, b) in s.bytes().enumerate() {
        let escape = match b {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0..=0x1F => "",
            _ => continue,
        };
        out.push_str(&s[start..i]);
        if escape.is_empty() {
            let _ = write!(out, "\\u{b:04x}");
        } else {
            out.push_str(escape);
        }
        start = i + 1;
    }
    out.push_str(&s[start..]);
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(root: Value) -> Document {
        Document {
            root,
            shared: Vec::new(),
        }
    }

    #[test]
    fn round_trips_through_serde_json() {
        let root = Value::Object(vec![
            (
                "a\"\n\u{1}é😀".into(),
                Value::Sequence(vec![Value::Number(1.5), Value::Number(1e21)]),
            ),
            ("b".into(), Value::Null),
            ("c".into(), Value::Bool(true)),
        ]);
        let json = encode(&[doc(root), doc(Value::Number(-3.0))]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["a\"\n\u{1}é😀"][1].as_f64(), Some(1e21));
        assert_eq!(parsed[1].as_f64(), Some(-3.0));
    }

    #[test]
    fn rejects_values_without_a_json_form() {
        for v in [
            Value::Number(f64::NAN),
            Value::Number(-0.0),
            Value::Date(0.0),
            Value::Shared(0),
        ] {
            assert_eq!(encode(&[doc(Value::Sequence(vec![v]))]), None);
        }
    }
}
