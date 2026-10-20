//! Port of `src/parser/constructor.ts` and the built-in tags of `src/tag/**`:
//! [`Event`]s → [`Document`]s.
//!
//! JS values have identity (aliases share objects, `Map`/`Set` keys compare objects by
//! identity, `<<` merges look tags up by object), so collections, dates and binaries
//! live in an arena and values refer to them by node id. When a document is complete
//! it is turned into a [`Value`] tree, with nodes reached more than once (aliases,
//! cycles, merged values) moved to [`Document::shared`].
//!
//! Inputs the port can't reproduce exactly fail with [`Error::is_unsupported`]; the JS
//! bindings then rerun the load through upstream's own constructor. That covers
//! `"\uD800"`-style escapes (lone surrogates) and `legacyMapTag` keys that would be
//! stringified through `Date.prototype.toString` (time-zone dependent).

mod containers;
mod fast;
mod scalars;
mod tagname;

use std::borrow::Cow;
use std::collections::HashMap;

use self::containers::{KeySet, ObjectMap, SzMap, SzSet, js_string};
use self::scalars::Resolved;
use crate::error::Error;
use crate::event::{CollectionEvent, Event, ScalarEvent, ScalarStyle, Span};
use crate::parser::scalar_value_exact;
use crate::schema::{Schema, Tag, TagKind};
use crate::value::{Document, Value};

pub use self::fast::construct_json;
pub(crate) use self::scalars::js_number_to_string;

/// `ConstructorOptions` (the filename lives in the bindings, which format messages).
#[derive(Debug, Clone)]
pub struct ConstructorOptions {
    /// Tags to resolve with. Default: [`Schema::core`].
    pub schema: Schema,
    /// `JSON.parse` compatibility: duplicate keys override instead of failing.
    pub json: bool,
    /// Limit on keys processed by `<<` merges per load; `None` means unlimited (`-1`).
    pub max_total_merge_keys: Option<usize>,
    /// Limit on alias nodes per document; `None` means unlimited (`-1`).
    pub max_aliases: Option<usize>,
}

impl Default for ConstructorOptions {
    fn default() -> Self {
        ConstructorOptions {
            schema: Schema::core(),
            json: false,
            max_total_merge_keys: Some(10_000),
            max_aliases: None,
        }
    }
}

const MERGE_TAG_NAME: &str = "tag:yaml.org,2002:merge";

/// Index of a node in the arena.
pub(crate) type NodeId = u32;

const NOT_SHARED: u32 = u32::MAX;

/// A value under construction: primitives inline, objects by arena id.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum V {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Node(NodeId),
}

/// An object-like JS value.
#[derive(Debug)]
pub(crate) enum Node {
    /// An array: `seqTag`, `pairsTag`, a finalized `omapTag`.
    Seq(Vec<V>),
    /// The `omapTag` carrier `{ list, seen }`.
    Omap {
        list: Vec<V>,
        seen: SzSet,
    },
    /// `{}`: `mapTag` and `legacyMapTag`.
    Object(ObjectMap),
    /// `Map`: `realMapTag`.
    Map(SzMap),
    /// `Set`: `setTag`.
    Set(KeySet),
    Binary(Vec<u8>),
    Date(f64),
    /// Moved out while converting to [`Value`].
    Taken,
}

#[derive(Debug)]
struct Anchor {
    value: V,
    tag: Tag,
    is_value_final: bool,
}

#[derive(Debug)]
enum Frame {
    Document {
        value: Option<V>,
    },
    Sequence {
        node: NodeId,
        tag: Tag,
        anchor: Option<usize>,
    },
    Mapping {
        node: NodeId,
        tag: Tag,
        anchor: Option<usize>,
        key: Option<V>,
        key_position: usize,
        key_is_merge: bool,
        // Keys brought in by a merge that an explicit pair may still override.
        overridable: Option<SzSet>,
    },
}

type Result<T> = std::result::Result<T, Error>;

struct State<'a> {
    source: &'a str,
    schema: &'a Schema,
    json: bool,
    max_total_merge_keys: Option<usize>,
    max_aliases: Option<usize>,
    position: usize,
    nodes: Vec<Node>,
    frames: Vec<Frame>,
    anchor_names: HashMap<&'a str, usize>,
    anchors: Vec<Anchor>,
    // Mapping tag each sequence element was built with, keyed by node. Needed by `<<`
    // merge, which sees only the finished element values.
    node_tags: HashMap<NodeId, Tag>,
    tag_handlers: HashMap<String, String>,
    total_merge_keys: usize,
    alias_count: usize,
    /// The schema's implicit scalar tags in order, with their `implicitFirstChars`.
    implicit: ImplicitTags,
}

/// What a scalar event constructs.
pub(crate) enum ScalarOut<'s> {
    /// A resolved scalar; the text is the scalar's decoded source.
    Value(Resolved, Cow<'s, str>),
    /// An empty node with a collection tag (`!!map`, `!!seq`): an empty collection.
    EmptyCollection,
}

/// The first chars an implicit tag accepts (`implicitFirstChars`) as a byte table; every
/// listed char is ASCII, and the empty string (an empty scalar) is entry 0.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FirstChars {
    any: bool,
    table: [bool; 128],
}

impl FirstChars {
    fn new(chars: Option<&[&str]>) -> Self {
        let mut table = [false; 128];
        let Some(chars) = chars else {
            return FirstChars { any: true, table };
        };
        for c in chars {
            table[c.as_bytes().first().map_or(0, |&b| usize::from(b))] = true;
        }
        FirstChars { any: false, table }
    }

    fn accepts(&self, source: &str) -> bool {
        self.any
            || match source.as_bytes().first() {
                None => self.table[0],
                Some(&b) => self.table.get(usize::from(b)).copied().unwrap_or(false),
            }
    }
}

/// The schema's implicit scalar tags in order, with their `implicitFirstChars`.
pub(crate) type ImplicitTags = Vec<(Tag, FirstChars)>;

pub(crate) fn implicit_tags(schema: &Schema) -> ImplicitTags {
    schema
        .tags()
        .iter()
        .filter(|&&tag| scalars::is_implicit(tag))
        .map(|&tag| (tag, FirstChars::new(scalars::implicit_first_chars(tag))))
        .collect()
}

/// `constructScalar` minus building the value: shared by both constructors.
pub(crate) fn resolve_scalar<'s>(
    schema: &Schema,
    implicit: &[(Tag, FirstChars)],
    source_text: &'s str,
    event: &ScalarEvent,
    tag_handlers: &HashMap<String, String>,
    position: usize,
) -> Result<(ScalarOut<'s>, Tag)> {
    let fail = |message: String| Err(Error::new(message, position));
    let Some(source) = scalar_value_exact(source_text, event) else {
        return Err(Error::unsupported(
            "a scalar escape decodes to a lone surrogate",
        ));
    };
    let raw_tag = event.tag.map_or("", |span| span.slice(source_text));

    if !raw_tag.is_empty() {
        if raw_tag == "!" {
            return Ok((ScalarOut::Value(Resolved::Str, source), Tag::Str));
        }

        let tag_name = tagname::tag_name_full(raw_tag, tag_handlers);
        if let Some(tag) = schema.lookup(TagKind::Scalar, &tag_name) {
            return match scalars::resolve(tag, &source, true) {
                Some(resolved) => Ok((ScalarOut::Value(resolved, source), tag)),
                None => fail(format!(
                    "cannot resolve a node with !<{tag_name}> explicit tag"
                )),
            };
        }

        // An empty node carrying a collection tag (`!!map`, `!!seq`) arrives as a
        // scalar event: resolve it by the tag's kind into an empty collection.
        let collection = schema
            .lookup(TagKind::Mapping, &tag_name)
            .or_else(|| schema.lookup(TagKind::Sequence, &tag_name));
        if let Some(tag) = collection {
            if !source.is_empty() {
                return fail(format!(
                    "cannot resolve a node with !<{tag_name}> explicit tag"
                ));
            }
            return Ok((ScalarOut::EmptyCollection, tag));
        }

        return fail(format!("unknown scalar tag !<{tag_name}>"));
    }

    if event.style == ScalarStyle::Plain {
        // resolveImplicitScalarTag
        for (tag, first_chars) in implicit {
            let tag = *tag;
            if !first_chars.accepts(&source) {
                continue;
            }
            if let Some(resolved) = scalars::resolve(tag, &source, false) {
                return Ok((ScalarOut::Value(resolved, source), tag));
            }
        }
    }

    Ok((ScalarOut::Value(Resolved::Str, source), Tag::Str))
}

fn event_position(event: &Event) -> usize {
    match event {
        Event::Scalar(s) => s.tag.or(s.anchor).or(s.value).map_or(0, |span| span.start),
        Event::Sequence(c) | Event::Mapping(c) => {
            c.tag.or(c.anchor).map_or(c.start, |span| span.start)
        }
        Event::Alias(span) => span.start,
        Event::Document(_) | Event::Pop => 0,
    }
}

impl<'a> State<'a> {
    fn fail<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(Error::new(message, self.position))
    }

    fn slice(&self, span: Option<Span>) -> &'a str {
        span.map_or("", |s| s.slice(self.source))
    }

    fn alloc(&mut self, node: Node) -> V {
        let id = NodeId::try_from(self.nodes.len()).unwrap_or(NodeId::MAX);
        self.nodes.push(node);
        V::Node(id)
    }

    fn lookup(&self, kind: TagKind, name: &str) -> Option<Tag> {
        self.schema.lookup(kind, name)
    }

    /// `tag.create(tagName)`
    fn create(&mut self, tag: Tag) -> V {
        let node = match tag {
            Tag::Omap => Node::Omap {
                list: Vec::new(),
                seen: SzSet::default(),
            },
            Tag::RealMap => Node::Map(SzMap::default()),
            Tag::Set => Node::Set(KeySet::default()),
            Tag::Map | Tag::LegacyMap => Node::Object(ObjectMap::default()),
            _ => Node::Seq(Vec::new()),
        };
        self.alloc(node)
    }

    /// `tag.finalize(carrier)` for tags without `carrierIsResult` (only `omapTag`).
    fn finalize(&mut self, tag: Tag, value: V) -> V {
        if tag == Tag::Omap {
            if let V::Node(id) = value {
                let node = &mut self.nodes[id as usize];
                if let Node::Omap { list, .. } = node {
                    // The carrier is unreachable (its anchor isn't final), so the list can
                    // take its place.
                    *node = Node::Seq(std::mem::take(list));
                }
            }
        }
        value
    }

    fn construct_scalar(&mut self, event: &ScalarEvent) -> Result<(V, Tag)> {
        let (out, tag) = resolve_scalar(
            self.schema,
            &self.implicit,
            self.source,
            event,
            &self.tag_handlers,
            self.position,
        )?;
        Ok(match out {
            ScalarOut::Value(Resolved::Str, source) => (V::Str(source.into_owned()), tag),
            ScalarOut::Value(resolved, _) => (self.resolved(resolved), tag),
            ScalarOut::EmptyCollection => {
                let carrier = self.create(tag);
                (self.finalize(tag, carrier), tag)
            }
        })
    }

    /// A resolved scalar other than a string.
    fn resolved(&mut self, resolved: Resolved) -> V {
        match resolved {
            Resolved::Null => V::Null,
            Resolved::Bool(b) => V::Bool(b),
            Resolved::Number(n) => V::Num(n),
            Resolved::Str => V::Str(String::new()),
            Resolved::Merge => V::Str("<<".into()),
            Resolved::Binary(bytes) => self.alloc(Node::Binary(bytes)),
            Resolved::Date(ms) => self.alloc(Node::Date(ms)),
        }
    }

    fn collection_tag_name(&self, event: &CollectionEvent, default: &str) -> String {
        match self.slice(event.tag) {
            "" | "!" => default.to_owned(),
            raw => tagname::tag_name_full(raw, &self.tag_handlers),
        }
    }

    fn charge_merge_work(&mut self) -> Result<()> {
        self.total_merge_keys += 1;
        if let Some(max) = self.max_total_merge_keys {
            if self.total_merge_keys > max {
                return self.fail(format!("merge keys exceeded maxTotalMergeKeys ({max})"));
            }
        }
        Ok(())
    }

    // --- the mapping tags' has / addPair / keys / get ---

    fn map_has(&self, tag: Tag, target: NodeId, key: &V) -> Result<bool> {
        Ok(match &self.nodes[target as usize] {
            Node::Object(object) => match tag {
                Tag::LegacyMap => match self.legacy_key(key)? {
                    Some(k) => object.contains(&k),
                    None => false,
                },
                _ => match key {
                    V::Node(_) => false,
                    V::Str(k) => object.contains(k),
                    _ => object.contains(&js_string(key)),
                },
            },
            Node::Map(map) => map.contains(key),
            Node::Set(set) => set.contains(key),
            _ => false,
        })
    }

    /// `addPair`; `Err(message)` for upstream's returned error strings.
    fn map_add_pair(
        &mut self,
        tag: Tag,
        target: NodeId,
        key: V,
        value: V,
    ) -> Result<std::result::Result<(), &'static str>> {
        if matches!(self.nodes[target as usize], Node::Object(_)) {
            let normalized = if tag == Tag::LegacyMap {
                match self.legacy_key(&key)? {
                    Some(k) => k,
                    None => return Ok(Err("nested arrays are not supported inside keys")),
                }
            } else {
                match key {
                    V::Node(_) => return Ok(Err("object-based map does not support complex keys")),
                    V::Str(k) => k,
                    other => js_string(&other),
                }
            };
            if let Node::Object(object) = &mut self.nodes[target as usize] {
                object.insert(normalized, value);
            }
            return Ok(Ok(()));
        }
        match &mut self.nodes[target as usize] {
            Node::Map(map) => map.insert(key, value),
            Node::Set(set) => {
                if value != V::Null {
                    return Ok(Err("cannot resolve a set item"));
                }
                set.insert(key);
            }
            _ => {}
        }
        Ok(Ok(()))
    }

    fn map_keys(&self, source: NodeId) -> Vec<V> {
        match &self.nodes[source as usize] {
            Node::Object(object) => object.js_keys().map(|k| V::Str(k.to_owned())).collect(),
            Node::Map(map) => map.keys().cloned().collect(),
            Node::Set(set) => set.iter().cloned().collect(),
            _ => Vec::new(),
        }
    }

    fn map_get(&self, source: NodeId, key: &V) -> Result<V> {
        Ok(match &self.nodes[source as usize] {
            Node::Object(object) => object.get(&self.js_key(key)?).cloned().unwrap_or(V::Null),
            Node::Map(map) => map.get(key).cloned().unwrap_or(V::Null),
            _ => V::Null,
        })
    }

    /// `String(key)` for a primitive.
    fn js_key(&self, key: &V) -> Result<String> {
        match key {
            V::Node(id) => self
                .node_to_string(*id)
                .ok_or_else(Self::date_string_unsupported),
            _ => Ok(js_string(key)),
        }
    }

    fn date_string_unsupported() -> Error {
        Error::unsupported("a Date would be stringified with the local time zone")
    }

    /// `String(object)` for the objects `legacyMapTag` may stringify; `None` for a Date.
    fn node_to_string(&self, id: NodeId) -> Option<String> {
        Some(match &self.nodes[id as usize] {
            Node::Seq(items) => {
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    parts.push(match item {
                        V::Null => String::new(),
                        V::Node(inner) => self.node_to_string(*inner)?,
                        other => js_string(other),
                    });
                }
                parts.join(",")
            }
            Node::Object(_) | Node::Omap { .. } => "[object Object]".to_owned(),
            Node::Map(_) => "[object Map]".to_owned(),
            Node::Set(_) => "[object Set]".to_owned(),
            Node::Binary(bytes) => bytes
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(","),
            Node::Date(_) | Node::Taken => return None,
        })
    }

    /// `normalizeKey` of `legacyMapTag`: `Ok(None)` for nested arrays.
    fn legacy_key(&self, key: &V) -> Result<Option<String>> {
        let V::Node(id) = key else {
            return Ok(Some(js_string(key)));
        };
        match &self.nodes[*id as usize] {
            Node::Seq(items) => {
                let mut parts = Vec::with_capacity(items.len());
                for item in items {
                    parts.push(match item {
                        V::Null => String::new(),
                        V::Node(inner) => match &self.nodes[*inner as usize] {
                            Node::Seq(_) => return Ok(None),
                            Node::Object(_) => "[object Object]".to_owned(),
                            _ => self
                                .node_to_string(*inner)
                                .ok_or_else(Self::date_string_unsupported)?,
                        },
                        other => js_string(other),
                    });
                }
                Ok(Some(parts.join(",")))
            }
            _ => self
                .node_to_string(*id)
                .map(Some)
                .ok_or_else(Self::date_string_unsupported),
        }
    }

    // --- the sequence tags' addItem ---

    fn add_item(
        &mut self,
        tag: Tag,
        target: NodeId,
        item: V,
    ) -> std::result::Result<(), &'static str> {
        match tag {
            Tag::Pairs => {
                let pair = match &item {
                    V::Node(id) => match &self.nodes[*id as usize] {
                        Node::Map(map) if map.len() == 1 => {
                            map.iter().next().map(|(k, v)| vec![k.clone(), v.clone()])
                        }
                        Node::Object(object) if object.len() == 1 => object
                            .iter()
                            .next()
                            .map(|(k, v)| vec![V::Str(k.clone()), v.clone()]),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(pair) = pair else {
                    return Err("cannot resolve a pairs item");
                };
                let pair = self.alloc(Node::Seq(pair));
                if let Node::Seq(list) = &mut self.nodes[target as usize] {
                    list.push(pair);
                }
            }
            Tag::Omap => {
                let key = match &item {
                    V::Node(id) => match &self.nodes[*id as usize] {
                        Node::Map(map) if map.len() == 1 => map.keys().next().cloned(),
                        Node::Object(object) if object.len() == 1 => {
                            object.iter().next().map(|(k, _)| V::Str(k.clone()))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let Some(key) = key else {
                    return Err("cannot resolve an ordered map item");
                };
                if let Node::Omap { list, seen } = &mut self.nodes[target as usize] {
                    if seen.contains(&key) {
                        return Err("duplicate key in ordered map");
                    }
                    seen.insert(&key);
                    list.push(item);
                }
            }
            _ => {
                if let Node::Seq(list) = &mut self.nodes[target as usize] {
                    list.push(item);
                }
            }
        }
        Ok(())
    }

    // --- merges and values ---

    fn merge_keys(&mut self, frame: usize, source: NodeId) -> Result<()> {
        // Count the source mapping itself to bound sequences of empty mappings.
        self.charge_merge_work()?;

        let Frame::Mapping {
            node: target, tag, ..
        } = self.frames[frame]
        else {
            return Ok(());
        };
        for source_key in self.map_keys(source) {
            self.charge_merge_work()?;

            if self.map_has(tag, target, &source_key)? {
                continue;
            }

            let value = self.map_get(source, &source_key)?;
            if let Err(message) = self.map_add_pair(tag, target, source_key.clone(), value)? {
                return self.fail(message);
            }

            if let Frame::Mapping { overridable, .. } = &mut self.frames[frame] {
                overridable
                    .get_or_insert_with(SzSet::default)
                    .insert(&source_key);
            }
        }
        Ok(())
    }

    fn merge_source(&mut self, frame: usize, source: &V, source_tag: Tag) -> Result<()> {
        if let Frame::Mapping { key_position, .. } = self.frames[frame] {
            self.position = key_position;
        }
        let unacceptable = "cannot merge mappings; the provided source object is unacceptable";

        match (source, source_tag.kind()) {
            (V::Node(id), TagKind::Mapping) => self.merge_keys(frame, *id),
            (V::Node(id), TagKind::Sequence)
                if matches!(self.nodes[*id as usize], Node::Seq(_)) =>
            {
                let Node::Seq(elements) = &self.nodes[*id as usize] else {
                    return Ok(());
                };
                // A hard cap on top of the merge budget; not exposed as an option upstream.
                if elements.len() > 100 {
                    return self.fail("abnormal merge sequence size");
                }
                for element in elements.clone() {
                    let mapping = match element {
                        V::Node(element_id) if self.node_tags.contains_key(&element_id) => {
                            element_id
                        }
                        _ => return self.fail(unacceptable),
                    };
                    self.merge_keys(frame, mapping)?;
                }
                Ok(())
            }
            _ => self.fail(unacceptable),
        }
    }

    fn add_mapping_value(&mut self, frame: usize, key: V, value: V, value_tag: Tag) -> Result<()> {
        let Frame::Mapping {
            node,
            tag,
            key_position,
            key_is_merge,
            ..
        } = self.frames[frame]
        else {
            return Ok(());
        };
        self.position = key_position;

        // `<<` is intercepted before dedup, so a repeated merge key is allowed.
        if key_is_merge {
            return self.merge_source(frame, &value, value_tag);
        }

        let overridden = match &self.frames[frame] {
            Frame::Mapping {
                overridable: Some(set),
                ..
            } => set.contains(&key),
            _ => false,
        };
        if !self.json && !overridden && self.map_has(tag, node, &key)? {
            return self.fail("duplicated mapping key");
        }

        // Upstream deletes the key from `overridable` after `addPair`; an `addPair`
        // failure aborts the load either way, so deleting first is equivalent.
        if let Frame::Mapping {
            overridable: Some(set),
            ..
        } = &mut self.frames[frame]
        {
            set.remove(&key);
        }
        if let Err(message) = self.map_add_pair(tag, node, key, value)? {
            return self.fail(message);
        }
        Ok(())
    }

    fn add_value(&mut self, value: V, tag: Tag) -> Result<()> {
        let index = self.frames.len() - 1;
        let position = self.position;
        match &mut self.frames[index] {
            Frame::Document { value: slot } => {
                *slot = Some(value);
                Ok(())
            }
            Frame::Sequence {
                node,
                tag: sequence_tag,
                ..
            } => {
                let (node, sequence_tag) = (*node, *sequence_tag);
                // Any element may later be folded in by a `<<` merge, which by then can't
                // tell what built it.
                if tag.kind() == TagKind::Mapping {
                    if let V::Node(id) = value {
                        self.node_tags.insert(id, tag);
                    }
                }
                match self.add_item(sequence_tag, node, value) {
                    Ok(()) => Ok(()),
                    Err(message) => self.fail(message),
                }
            }
            Frame::Mapping {
                key,
                key_position,
                key_is_merge,
                ..
            } => {
                if let Some(key) = key.take() {
                    return self.add_mapping_value(index, key, value, tag);
                }
                *key = Some(value);
                *key_position = position;
                *key_is_merge = tag.name() == MERGE_TAG_NAME;
                Ok(())
            }
        }
    }

    fn store_anchor(
        &mut self,
        anchor: Option<Span>,
        value: &V,
        tag: Tag,
        is_value_final: bool,
    ) -> Option<usize> {
        let span = anchor?;
        let index = self.anchors.len();
        self.anchors.push(Anchor {
            value: value.clone(),
            tag,
            is_value_final,
        });
        self.anchor_names.insert(span.slice(self.source), index);
        Some(index)
    }

    fn run(&mut self, events: &[Event]) -> Result<Vec<Document>> {
        let mut documents = Vec::new();

        for event in events {
            self.position = event_position(event);

            match event {
                Event::Document(document) => {
                    self.anchor_names.clear();
                    self.anchors.clear();
                    self.node_tags.clear();
                    self.alias_count = 0;
                    self.tag_handlers.clear();
                    for directive in &document.directives {
                        if let crate::event::Directive::Tag { handle, prefix } = directive {
                            self.tag_handlers.insert(handle.clone(), prefix.clone());
                        }
                    }
                    self.frames.push(Frame::Document { value: None });
                }

                Event::Scalar(scalar) => {
                    let (value, tag) = self.construct_scalar(scalar)?;
                    self.store_anchor(scalar.anchor, &value, tag, true);
                    self.add_value(value, tag)?;
                }

                Event::Sequence(collection) | Event::Mapping(collection) => {
                    let mapping = matches!(event, Event::Mapping(_));
                    let (kind, default) = if mapping {
                        (TagKind::Mapping, "tag:yaml.org,2002:map")
                    } else {
                        (TagKind::Sequence, "tag:yaml.org,2002:seq")
                    };
                    let tag_name = self.collection_tag_name(collection, default);
                    let Some(tag) = self.lookup(kind, &tag_name) else {
                        let what = if mapping { "mapping" } else { "sequence" };
                        return self.fail(format!("unknown {what} tag !<{tag_name}>"));
                    };

                    let value = self.create(tag);
                    let anchor =
                        self.store_anchor(collection.anchor, &value, tag, tag != Tag::Omap);
                    let V::Node(node) = value else {
                        unreachable!("create returns a node")
                    };
                    self.frames.push(if mapping {
                        Frame::Mapping {
                            node,
                            tag,
                            anchor,
                            key: None,
                            key_position: self.position,
                            key_is_merge: false,
                            overridable: None,
                        }
                    } else {
                        Frame::Sequence { node, tag, anchor }
                    });
                }

                Event::Alias(span) => {
                    self.alias_count += 1;
                    if let Some(max) = self.max_aliases {
                        if self.alias_count > max {
                            return self.fail(format!("aliases exceeded maxAliases ({max})"));
                        }
                    }

                    let name = span.slice(self.source);
                    let Some(&index) = self.anchor_names.get(name) else {
                        return self.fail(format!("unidentified alias \"{name}\""));
                    };
                    let anchor = &self.anchors[index];
                    if !anchor.is_value_final {
                        let tag_name = anchor.tag.name();
                        return self.fail(format!(
                            "recursive alias \"{name}\" is not supported for tag {tag_name} because it uses finalize()"
                        ));
                    }
                    let (value, tag) = (anchor.value.clone(), anchor.tag);
                    self.add_value(value, tag)?;
                }

                Event::Pop => match self.frames.pop() {
                    Some(Frame::Document { value }) => {
                        let root = value.unwrap_or(V::Null);
                        documents.push(self.finish_document(root));
                        // Nothing in the next document can reach this one's nodes.
                        self.nodes.clear();
                        self.anchor_names.clear();
                        self.anchors.clear();
                        self.node_tags.clear();
                    }
                    Some(Frame::Mapping {
                        key: Some(_),
                        key_position,
                        ..
                    }) => {
                        self.position = key_position;
                        return self.fail("incomplete mapping pair in event stream");
                    }
                    Some(
                        Frame::Mapping {
                            node, tag, anchor, ..
                        }
                        | Frame::Sequence { node, tag, anchor },
                    ) => {
                        let value = self.finalize(tag, V::Node(node));
                        if let Some(index) = anchor {
                            self.anchors[index].value = value.clone();
                            self.anchors[index].is_value_final = true;
                        }
                        self.add_value(value, tag)?;
                    }
                    None => return self.fail("unbalanced event stream"),
                },
            }
        }

        Ok(documents)
    }

    // --- arena → Value ---

    /// Builds a document's value; nodes reachable more than once become shared.
    fn finish_document(&mut self, root: V) -> Document {
        // Only an alias can make a node reachable twice (merges copy values out of the
        // source, which is itself reachable only through an alias). Without one, every
        // node is reached once and there is nothing to count.
        if self.alias_count == 0 {
            let root = self.build(root, &[]);
            return Document {
                root,
                shared: Vec::new(),
            };
        }

        // Count references with an explicit stack: alias chains can be arbitrarily long.
        let mut refs = vec![0u32; self.nodes.len()];
        let mut order: Vec<NodeId> = Vec::new();
        let mut stack: Vec<NodeId> = Vec::new();
        let push = |v: &V, refs: &mut [u32], stack: &mut Vec<NodeId>| {
            if let V::Node(id) = v {
                let count = &mut refs[*id as usize];
                *count += 1;
                if *count == 1 {
                    stack.push(*id);
                }
            }
        };
        push(&root, &mut refs, &mut stack);
        while let Some(id) = stack.pop() {
            order.push(id);
            match &self.nodes[id as usize] {
                Node::Seq(items) | Node::Set(KeySet { items, .. }) => {
                    for item in items {
                        push(item, &mut refs, &mut stack);
                    }
                }
                Node::Object(object) => {
                    for (_, item) in object.iter() {
                        push(item, &mut refs, &mut stack);
                    }
                }
                Node::Map(map) => {
                    for (k, item) in map.iter() {
                        push(k, &mut refs, &mut stack);
                        push(item, &mut refs, &mut stack);
                    }
                }
                _ => {}
            }
        }

        // shared_index[node] is its index in Document::shared, or NOT_SHARED.
        let mut shared_index = vec![NOT_SHARED; self.nodes.len()];
        let mut shared_ids = Vec::new();
        for &id in &order {
            if refs[id as usize] > 1 {
                shared_index[id as usize] = u32::try_from(shared_ids.len()).unwrap_or(NOT_SHARED);
                shared_ids.push(id);
            }
        }

        let root = self.build(root, &shared_index);
        let shared = shared_ids
            .into_iter()
            .map(|id| self.build_node(id, &shared_index))
            .collect();
        Document { root, shared }
    }

    fn build(&mut self, v: V, shared: &[u32]) -> Value {
        match v {
            V::Null => Value::Null,
            V::Bool(b) => Value::Bool(b),
            V::Num(n) => Value::Number(n),
            V::Str(s) => Value::String(s),
            V::Node(id) => match shared.get(id as usize) {
                Some(&index) if index != NOT_SHARED => Value::Shared(index as usize),
                _ => self.build_node(id, shared),
            },
        }
    }

    fn build_node(&mut self, id: NodeId, shared: &[u32]) -> Value {
        match std::mem::replace(&mut self.nodes[id as usize], Node::Taken) {
            Node::Seq(items) | Node::Omap { list: items, .. } => {
                Value::Sequence(items.into_iter().map(|v| self.build(v, shared)).collect())
            }
            Node::Object(object) => Value::Object(
                object
                    .into_entries()
                    .map(|(k, v)| (k, self.build(v, shared)))
                    .collect(),
            ),
            Node::Map(map) => Value::Map(
                map.into_entries()
                    .map(|(k, v)| (self.build(k, shared), self.build(v, shared)))
                    .collect(),
            ),
            Node::Set(set) => Value::Set(
                set.items
                    .into_iter()
                    .map(|v| self.build(v, shared))
                    .collect(),
            ),
            Node::Binary(bytes) => Value::Binary(bytes),
            Node::Date(ms) => Value::Date(ms),
            Node::Taken => Value::Null,
        }
    }
}

/// Port of `constructFromEvents`.
pub fn construct(
    input: &str,
    events: &[Event],
    options: &ConstructorOptions,
) -> std::result::Result<Vec<Document>, Error> {
    if options
        .schema
        .lookup(TagKind::Scalar, Tag::Str.name())
        .is_none()
    {
        return Err(Error::without_position(
            "schema does not define the default scalar tag (tag:yaml.org,2002:str)",
        ));
    }
    let mut state = State {
        source: input,
        schema: &options.schema,
        json: options.json,
        max_total_merge_keys: options.max_total_merge_keys,
        max_aliases: options.max_aliases,
        position: 0,
        nodes: Vec::new(),
        frames: Vec::new(),
        anchor_names: HashMap::new(),
        anchors: Vec::new(),
        node_tags: HashMap::new(),
        tag_handlers: HashMap::new(),
        total_merge_keys: 0,
        alias_count: 0,
        implicit: implicit_tags(&options.schema),
    };
    state.run(events)
}
