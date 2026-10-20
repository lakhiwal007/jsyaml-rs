//! Parser events, mirroring `src/parser/events.ts`. Upstream's `-1` ("absent") ranges are
//! `None` here, and offsets are bytes rather than UTF-16 code units.

/// A half-open byte range `start..end` into the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

impl Span {
    /// The text this span covers.
    pub fn slice(self, input: &str) -> &str {
        &input[self.start..self.end]
    }
}

/// `SCALAR_STYLE`; discriminants match upstream so they can cross into JS unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ScalarStyle {
    /// `foo`
    Plain = 1,
    /// `'foo'`
    SingleQuoted = 2,
    /// `"foo"`
    DoubleQuoted = 3,
    /// `|`
    LiteralBlock = 4,
    /// `>`
    FoldedBlock = 5,
}

/// `COLLECTION_STYLE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CollectionStyle {
    /// Indentation-based.
    Block = 1,
    /// `[...]` or `{...}`.
    Flow = 2,
}

/// `CHOMPING_MODE` of a block scalar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Chomping {
    /// Single trailing newline.
    Clip = 1,
    /// `-`: no trailing newline.
    Strip = 2,
    /// `+`: all trailing newlines.
    Keep = 3,
}

/// `DocumentDirective`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// `%YAML 1.2`
    Yaml {
        /// The version text, e.g. `1.2`.
        version: String,
    },
    /// `%TAG !e! tag:example.com,2000:`
    Tag {
        /// The handle, e.g. `!e!`.
        handle: String,
        /// The prefix it expands to.
        prefix: String,
    },
}

/// `DocumentEvent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentEvent {
    /// The document started with `---`.
    pub explicit_start: bool,
    /// The document ended with `...`.
    pub explicit_end: bool,
    /// Directives preceding the document.
    pub directives: Vec<Directive>,
}

/// `SequenceEvent` / `MappingEvent` (they have identical fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollectionEvent {
    /// Offset of the collection's first token.
    pub start: usize,
    /// The anchor name, without `&`.
    pub anchor: Option<Span>,
    /// The raw tag, e.g. `!!set`.
    pub tag: Option<Span>,
    /// Block or flow.
    pub style: CollectionStyle,
}

/// `ScalarEvent`. Decode its text with [`crate::get_scalar_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarEvent {
    /// The raw scalar text (inside the quotes for quoted styles); `None` for an empty node.
    pub value: Option<Span>,
    /// The anchor name, without `&`.
    pub anchor: Option<Span>,
    /// The raw tag.
    pub tag: Option<Span>,
    /// Presentation style.
    pub style: ScalarStyle,
    /// Chomping indicator (block scalars).
    pub chomping: Chomping,
    /// Content indentation of a block scalar; `-1` when there is none (other styles, or
    /// a block with no content line).
    pub indent: i32,
    /// The decoded value equals the raw `value` slice: no escapes, folding or chomping.
    pub fast: bool,
}

/// `Event`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Opens a document; closed by [`Event::Pop`].
    Document(DocumentEvent),
    /// Opens a sequence; closed by [`Event::Pop`].
    Sequence(CollectionEvent),
    /// Opens a mapping; closed by [`Event::Pop`].
    Mapping(CollectionEvent),
    /// A scalar node.
    Scalar(ScalarEvent),
    /// `*name`: the span covers `name`.
    Alias(Span),
    /// Closes the most recently opened document, sequence or mapping.
    Pop,
}

impl Event {
    /// Upstream's `EVENT_ID` of this event.
    pub fn id(&self) -> u8 {
        match self {
            Event::Document(_) => 1,
            Event::Sequence(_) => 2,
            Event::Mapping(_) => 3,
            Event::Scalar(_) => 4,
            Event::Alias(_) => 5,
            Event::Pop => 6,
        }
    }
}
