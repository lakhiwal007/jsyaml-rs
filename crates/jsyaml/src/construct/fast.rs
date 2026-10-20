//! The fast path for plain data: events → the JSON text of the documents, in one pass,
//! with no intermediate values.
//!
//! It covers what most YAML in the wild is (mappings, sequences, strings, numbers,
//! booleans, nulls) and gives up — returns `None` — on anything else: aliases, `<<`
//! merges, values JSON can't carry (`NaN`, `±Infinity`, `-0`, dates, binaries, `Map`,
//! `Set`, ordered maps and pairs), collection keys, and every error. The caller then runs
//! the full constructor ([`super::construct`]) on the same events, which produces the
//! exact result or error. So the fast path never decides an outcome on its own.
//!
//! Its JSON is meant for `JSON.parse`, which reproduces the JS semantics the full
//! constructor emulates: integer-like keys move first, and a repeated key (allowed with
//! `json: true`) keeps its first position and takes the last value.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use super::scalars::{Resolved, js_number_to_string};
use super::{ConstructorOptions, ScalarOut, implicit_tags, resolve_scalar, tagname};
use crate::event::{Directive, Event};
use crate::schema::{Tag, TagKind};

/// Linear scans beat hashing for small mappings; index keys above this size.
const INDEX_THRESHOLD: usize = 8;

/// The keys of one mapping, for duplicate detection.
#[derive(Default)]
struct Keys<'s> {
    list: Vec<Cow<'s, str>>,
    index: Option<HashSet<Cow<'s, str>>>,
}

impl<'s> Keys<'s> {
    /// `false` if the key was already present.
    fn insert(&mut self, key: Cow<'s, str>) -> bool {
        if let Some(index) = &mut self.index {
            return index.insert(key);
        }
        if self.list.contains(&key) {
            return false;
        }
        if self.list.len() >= INDEX_THRESHOLD {
            let mut index: HashSet<Cow<'s, str>> = self.list.drain(..).collect();
            index.insert(key);
            self.index = Some(index);
        } else {
            self.list.push(key);
        }
        true
    }
}

enum Frame<'s> {
    Document,
    Sequence {
        first: bool,
    },
    Mapping {
        first: bool,
        expect_key: bool,
        keys: Keys<'s>,
    },
}

/// A value about to be written: a scalar's JSON text, or the start of a collection.
enum Item<'s> {
    Scalar(Resolved, Cow<'s, str>),
    EmptyCollection(Tag),
    Open(Tag),
}

struct Writer<'s> {
    out: String,
    frames: Vec<Frame<'s>>,
    json: bool,
}

impl<'s> Writer<'s> {
    /// Writes `item` where the innermost frame expects its next value or key.
    fn item(&mut self, item: Item<'s>, tag: Tag) -> Option<()> {
        let frame = self.frames.last_mut()?;
        match frame {
            Frame::Document => {}
            Frame::Sequence { first } => {
                if !*first {
                    self.out.push(',');
                }
                *first = false;
            }
            Frame::Mapping {
                first,
                expect_key,
                keys,
            } => {
                if *expect_key {
                    // Keys must be primitives; `<<` (merge) needs the full constructor.
                    let Item::Scalar(resolved, source) = item else {
                        return None;
                    };
                    if tag.name() == "tag:yaml.org,2002:merge" {
                        return None;
                    }
                    let key = key_string(&resolved, source)?;
                    if !self.json && !keys.insert(key.clone()) {
                        return None; // duplicated mapping key: let the full path report it
                    }
                    if !*first {
                        self.out.push(',');
                    }
                    *first = false;
                    *expect_key = false;
                    write_json_string(&mut self.out, &key);
                    self.out.push(':');
                    return Some(());
                }
                *expect_key = true;
            }
        }
        match item {
            Item::Scalar(resolved, source) => write_scalar(&mut self.out, &resolved, &source)?,
            Item::EmptyCollection(Tag::Seq) => self.out.push_str("[]"),
            Item::EmptyCollection(Tag::Map | Tag::LegacyMap) => self.out.push_str("{}"),
            Item::Open(Tag::Seq) => {
                self.out.push('[');
                self.frames.push(Frame::Sequence { first: true });
            }
            Item::Open(Tag::Map | Tag::LegacyMap) => {
                self.out.push('{');
                self.frames.push(Frame::Mapping {
                    first: true,
                    expect_key: true,
                    keys: Keys::default(),
                });
            }
            Item::EmptyCollection(_) | Item::Open(_) => return None,
        }
        Some(())
    }
}

/// `String(key)` for a primitive key; `None` for keys that are objects.
fn key_string<'s>(resolved: &Resolved, source: Cow<'s, str>) -> Option<Cow<'s, str>> {
    Some(match *resolved {
        Resolved::Str => source,
        Resolved::Merge => Cow::Borrowed("<<"),
        Resolved::Null => Cow::Borrowed("null"),
        Resolved::Bool(true) => Cow::Borrowed("true"),
        Resolved::Bool(false) => Cow::Borrowed("false"),
        Resolved::Number(n) => Cow::Owned(js_number_to_string(n)),
        Resolved::Binary(_) | Resolved::Date(_) => return None,
    })
}

fn write_scalar(out: &mut String, resolved: &Resolved, source: &str) -> Option<()> {
    match *resolved {
        Resolved::Str => write_json_string(out, source),
        Resolved::Merge => out.push_str("\"<<\""),
        Resolved::Null => out.push_str("null"),
        Resolved::Bool(b) => out.push_str(if b { "true" } else { "false" }),
        // JS's own formatting (what JSON.stringify writes): exact, and short for large or
        // tiny numbers, where Rust's `{}` would write every digit (1e300 → 301 chars).
        Resolved::Number(n) if n.is_finite() && !(n == 0.0 && n.is_sign_negative()) => {
            out.push_str(&js_number_to_string(n));
        }
        Resolved::Number(_) | Resolved::Binary(_) | Resolved::Date(_) => return None,
    }
    Some(())
}

/// A JSON string literal (`JSON.parse` accepts every other char, U+2028/9 included).
pub(crate) fn write_json_string(out: &mut String, s: &str) {
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

/// The documents of `events` as a JSON array, when the fast path covers all of them;
/// `None` otherwise (then use [`super::construct`], which also reports errors).
pub fn construct_json(
    input: &str,
    events: &[Event],
    options: &ConstructorOptions,
) -> Option<String> {
    let schema = &options.schema;
    schema.lookup(TagKind::Scalar, Tag::Str.name())?;
    let implicit = implicit_tags(schema);
    let mut tag_handlers: HashMap<String, String> = HashMap::new();
    let mut writer = Writer {
        out: String::with_capacity(input.len() + 16),
        frames: Vec::new(),
        json: options.json,
    };
    let mut documents = 0;
    writer.out.push('[');

    for event in events {
        match event {
            Event::Document(document) => {
                if documents > 0 {
                    writer.out.push(',');
                }
                documents += 1;
                tag_handlers.clear();
                for directive in &document.directives {
                    if let Directive::Tag { handle, prefix } = directive {
                        tag_handlers.insert(handle.clone(), prefix.clone());
                    }
                }
                writer.frames.push(Frame::Document);
            }
            Event::Scalar(scalar) => {
                let (out, tag) =
                    resolve_scalar(schema, &implicit, input, scalar, &tag_handlers, 0).ok()?;
                let item = match out {
                    ScalarOut::Value(resolved, source) => Item::Scalar(resolved, source),
                    ScalarOut::EmptyCollection => Item::EmptyCollection(tag),
                };
                writer.item(item, tag)?;
            }
            Event::Sequence(collection) | Event::Mapping(collection) => {
                let (kind, default) = if matches!(event, Event::Mapping(_)) {
                    (TagKind::Mapping, "tag:yaml.org,2002:map")
                } else {
                    (TagKind::Sequence, "tag:yaml.org,2002:seq")
                };
                let raw = collection.tag.map_or("", |span| span.slice(input));
                let tag = if raw.is_empty() || raw == "!" {
                    schema.lookup(kind, default)
                } else {
                    schema.lookup(kind, &tagname::tag_name_full(raw, &tag_handlers))
                }?;
                writer.item(Item::Open(tag), tag)?;
            }
            Event::Alias(_) => return None,
            Event::Pop => match writer.frames.pop()? {
                Frame::Document => {}
                Frame::Sequence { .. } => writer.out.push(']'),
                Frame::Mapping {
                    expect_key: false, ..
                } => return None,
                Frame::Mapping { .. } => writer.out.push('}'),
            },
        }
    }

    writer.out.push(']');
    Some(writer.out)
}
