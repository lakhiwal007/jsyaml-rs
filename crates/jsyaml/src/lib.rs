//! A YAML 1.2 loader ported line by line from [js-yaml] 5.x, so it accepts, rejects and
//! constructs exactly what js-yaml does.
//!
//! The pipeline mirrors upstream:
//!
//! | Stage | This crate | js-yaml |
//! |---|---|---|
//! | Text → events | [`parse_events`] | `src/parser/parser.ts`, `parser_scalar.ts` |
//! | Events → values | [`construct()`] | `src/parser/constructor.ts`, `src/tag/**` |
//! | Events → JSON text (plain data only) | [`construct_json`] | — |
//! | Both | [`load`], [`load_all`] | `src/load.ts` |
//!
//! Offsets in [`Event`]s and [`Error`]s are byte offsets into the input; the JS bindings
//! convert them to UTF-16 offsets with [`utf16::Utf16Cursor`].
//!
//! [js-yaml]: https://github.com/nodeca/js-yaml

pub mod construct;
pub mod error;
pub mod event;
pub mod parser;
pub mod schema;
pub mod utf16;
pub mod value;

pub use crate::construct::{ConstructorOptions, construct, construct_json};
pub use crate::error::Error;
pub use crate::event::{
    Chomping, CollectionEvent, CollectionStyle, Directive, DocumentEvent, Event, ScalarEvent,
    ScalarStyle, Span,
};
pub use crate::parser::{ParserOptions, get_scalar_value, parse_events};
pub use crate::schema::{Schema, Tag, TagKind};
pub use crate::value::{Document, Value};

/// `String(n)` for a JS number (ECMAScript `Number::toString`).
pub fn js_number_to_string(n: f64) -> String {
    construct::js_number_to_string(n)
}

/// Options of `load` / `loadAll`: the union of parser and constructor options, as upstream.
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Options for [`parse_events`].
    pub parser: ParserOptions,
    /// Options for [`construct()`].
    pub constructor: ConstructorOptions,
}

/// Port of `loadAll`: every document in the stream.
pub fn load_all(input: &str, options: &LoadOptions) -> Result<Vec<Document>, Error> {
    let events = parse_events(input, &options.parser)?;
    construct(input, &events, &options.constructor)
}

/// Port of `load`: exactly one document, with upstream's messages otherwise.
pub fn load(input: &str, options: &LoadOptions) -> Result<Document, Error> {
    let mut documents = load_all(input, options)?;
    match documents.len() {
        0 => Err(Error::without_position(
            "expected a document, but the input is empty",
        )),
        1 => Ok(documents.swap_remove(0)),
        _ => Err(Error::without_position(
            "expected a single document in the stream, but found more",
        )),
    }
}
