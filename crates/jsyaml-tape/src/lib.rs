//! Shared layer of the JavaScript bindings (`jsyaml-node`, `jsyaml-wasm`).
//!
//! Bindings read the JS options into [`Options`] and call [`load`] or [`events`]; the
//! result goes to `js/decode.cjs`, which turns it into what js-yaml would return.
//! Keeping this in one crate guarantees both packages produce identical results.
//!
//! Building JS objects one N-API call at a time is what makes native addons slow, so
//! results cross the boundary in bulk, in one of three forms:
//!
//! - [`Loaded::Json`]: when every value is JSON-representable, a JSON string for V8's
//!   `JSON.parse`, the fastest way to build plain objects;
//! - [`Loaded::Tape`]: otherwise ([`Map`]s, `Set`s, `Date`s, `Uint8Array`s, `NaN`, `-0`,
//!   objects shared through aliases), a [`ValueTape`];
//! - [`EventTape`]: just the parser's events, for schemas with custom JS tags; upstream's
//!   own `constructFromEvents` builds the values.
//!
//! [`Map`]: jsyaml::Value::Map

#![warn(missing_docs)]

mod events;
mod json;
mod options;
mod values;

pub use crate::events::{EVENT_STRIDE, EventTape};
pub use crate::options::Options;
pub use crate::values::{ValueTape, op};

use jsyaml::utf16::Utf16Cursor;

/// The documents of one `loadAll` call, in transport form.
#[derive(Debug)]
pub enum Loaded {
    /// A JSON array of the documents.
    Json(String),
    /// Everything else.
    Tape(ValueTape),
}

/// A load error for `YAMLException.throwAt(source, position, reason)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Upstream's `reason`.
    pub reason: String,
    /// UTF-16 offset into the source, if the error has a mark.
    pub position: Option<usize>,
    /// Not a YAML error: the input needs js-yaml's own constructor
    /// ([`jsyaml::Error::is_unsupported`]); `decode.cjs` reruns the load that way.
    pub fallback: bool,
}

impl Failure {
    fn new(input: &str, error: &jsyaml::Error) -> Self {
        Failure {
            reason: error.reason().to_owned(),
            position: error
                .position()
                .map(|p| Utf16Cursor::new(input).to_utf16(p)),
            fallback: error.is_unsupported(),
        }
    }

    fn options(reason: String) -> Self {
        Failure {
            reason,
            position: None,
            fallback: false,
        }
    }
}

/// `loadAll(input, options)` with a schema of built-in tags only.
pub fn load(input: &str, options: &Options) -> Result<Loaded, Failure> {
    let options = options.to_load_options().map_err(Failure::options)?;
    let fail = |e: jsyaml::Error| Failure::new(input, &e);
    let events = jsyaml::parse_events(input, &options.parser).map_err(fail)?;
    // Plain data streams straight to JSON; everything else (and every error) goes
    // through the full constructor.
    if let Some(json) = jsyaml::construct_json(input, &events, &options.constructor) {
        return Ok(Loaded::Json(json));
    }
    let documents = jsyaml::construct(input, &events, &options.constructor).map_err(fail)?;
    Ok(match json::encode(&documents) {
        Some(json) => Loaded::Json(json),
        None => Loaded::Tape(ValueTape::encode(&documents)),
    })
}

/// `parseEvents(input, options)`, with offsets converted to UTF-16.
pub fn events(input: &str, options: &Options) -> Result<EventTape, Failure> {
    let options = options.to_load_options().map_err(Failure::options)?;
    let events =
        jsyaml::parse_events(input, &options.parser).map_err(|e| Failure::new(input, &e))?;
    Ok(EventTape::encode(input, &events))
}
