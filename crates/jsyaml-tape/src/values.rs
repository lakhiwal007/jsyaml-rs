//! The value tape: documents as a flat `u32` program that `js/decode.cjs` runs.
//!
//! Layout of `ops`:
//!
//! ```text
//! document count
//! per document: shared count N, N op offsets (one per shared value), root value
//! shared values (anywhere after the offsets table; reached through their offset)
//! ```
//!
//! A value is an [`op`] code followed by its operands. Containers carry their length;
//! strings are UTF-16 `(start, length)` into `strings`; numbers and dates index
//! `numbers`; binaries are byte `(start, length)` into `bytes`. Shared values are decoded
//! on first reference and registered before their children, so cycles resolve.

use jsyaml::{Document, Value};

/// Op codes; mirrored in `js/decode.cjs`.
#[allow(missing_docs)]
pub mod op {
    pub const NULL: u32 = 0;
    pub const TRUE: u32 = 1;
    pub const FALSE: u32 = 2;
    /// `index` into `numbers`
    pub const NUMBER: u32 = 3;
    /// `start, length` (UTF-16) into `strings`
    pub const STRING: u32 = 4;
    /// `length`, then `length` values
    pub const ARRAY: u32 = 5;
    /// `length`, then `length` × (key `start, length`, value)
    pub const OBJECT: u32 = 6;
    /// `length`, then `length` × (key value, value)
    pub const MAP: u32 = 7;
    /// `length`, then `length` values
    pub const SET: u32 = 8;
    /// `start, length` into `bytes`
    pub const BINARY: u32 = 9;
    /// `index` into `numbers` (milliseconds since the epoch)
    pub const DATE: u32 = 10;
    /// `index` into the document's shared table
    pub const SHARED: u32 = 11;
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct ValueTape {
    /// The program.
    pub ops: Vec<u32>,
    /// Number and date payloads.
    pub numbers: Vec<f64>,
    /// All strings, concatenated.
    pub strings: String,
    /// All binaries, concatenated.
    pub bytes: Vec<u8>,
    strings_utf16_len: usize,
}

impl ValueTape {
    pub(crate) fn encode(documents: &[Document]) -> Self {
        let mut tape = ValueTape::default();
        tape.ops.push(len(documents.len()));
        for doc in documents {
            tape.ops.push(len(doc.shared.len()));
            let table = tape.ops.len();
            tape.ops.resize(table + doc.shared.len(), 0);
            tape.value(&doc.root);
            for (i, shared) in doc.shared.iter().enumerate() {
                tape.ops[table + i] = len(tape.ops.len());
                tape.value(shared);
            }
        }
        tape
    }

    fn value(&mut self, v: &Value) {
        match v {
            Value::Null => self.ops.push(op::NULL),
            Value::Bool(true) => self.ops.push(op::TRUE),
            Value::Bool(false) => self.ops.push(op::FALSE),
            Value::Number(n) => self.number(op::NUMBER, *n),
            Value::Date(ms) => self.number(op::DATE, *ms),
            Value::String(s) => {
                self.ops.push(op::STRING);
                self.string(s);
            }
            Value::Sequence(items) | Value::Set(items) => {
                let code = if matches!(v, Value::Set(_)) {
                    op::SET
                } else {
                    op::ARRAY
                };
                self.ops.extend([code, len(items.len())]);
                for item in items {
                    self.value(item);
                }
            }
            Value::Object(entries) => {
                self.ops.extend([op::OBJECT, len(entries.len())]);
                for (k, item) in entries {
                    self.string(k);
                    self.value(item);
                }
            }
            Value::Map(entries) => {
                self.ops.extend([op::MAP, len(entries.len())]);
                for (k, item) in entries {
                    self.value(k);
                    self.value(item);
                }
            }
            Value::Binary(b) => {
                self.ops
                    .extend([op::BINARY, len(self.bytes.len()), len(b.len())]);
                self.bytes.extend_from_slice(b);
            }
            Value::Shared(i) => self.ops.extend([op::SHARED, len(*i)]),
        }
    }

    fn number(&mut self, code: u32, n: f64) {
        self.ops.extend([code, len(self.numbers.len())]);
        self.numbers.push(n);
    }

    fn string(&mut self, s: &str) {
        let units = s.encode_utf16().count();
        self.ops.extend([len(self.strings_utf16_len), len(units)]);
        self.strings.push_str(s);
        self.strings_utf16_len += units;
    }
}

/// Inputs are capped well below 4 GiB by V8's string limit, so every count fits.
fn len(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_shared_table_after_root() {
        let doc = Document {
            root: Value::Sequence(vec![Value::Shared(0), Value::String("é".into())]),
            shared: vec![Value::Sequence(vec![Value::Shared(0)])],
        };
        let tape = ValueTape::encode(&[doc]);
        #[rustfmt::skip]
        assert_eq!(tape.ops, [
            1,                                  // one document
            1, 10,                              // one shared value, at op 10
            op::ARRAY, 2, op::SHARED, 0, op::STRING, 0, 1,
            op::ARRAY, 1, op::SHARED, 0,        // shared[0] = [shared[0]]
        ]);
        assert_eq!(tape.strings, "é");
    }
}
