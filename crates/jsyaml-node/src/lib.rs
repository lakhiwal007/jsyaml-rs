//! Native Node.js addon built with NAPI-RS. The public API is `js/index.cjs`, a drop-in
//! for `require('js-yaml')`:
//!
//! ```js
//! const yaml = require('jsyaml-node');
//! const config = yaml.load(fs.readFileSync('config.yml', 'utf8'));
//! ```
//!
//! Loading runs natively; results cross into JS in bulk (a JSON string, or one packed
//! buffer) and the shared `decode.cjs` rebuilds js-yaml's values.
//! Errors are returned, not thrown, so JS can raise js-yaml's own `YAMLException`.

use jsyaml_tape::{EventTape, Failure, Loaded, Options};
use napi::Env;
use napi::bindgen_prelude::{BufferSlice, Object};
use napi_derive::napi;

/// The `LoadOptions` the native path supports, as prepared by `decode.cjs`.
#[napi(object)]
#[derive(Debug, Default)]
pub struct NativeOptions {
    /// Export names of the schema's built-in tags; empty for `CORE_SCHEMA`.
    pub tags: Option<Vec<String>>,
    pub json: Option<bool>,
    pub max_total_merge_keys: Option<f64>,
    pub max_aliases: Option<f64>,
    pub max_depth: Option<f64>,
}

impl From<NativeOptions> for Options {
    fn from(o: NativeOptions) -> Self {
        Options {
            tags: o.tags.unwrap_or_default(),
            json: o.json,
            max_total_merge_keys: o.max_total_merge_keys,
            max_aliases: o.max_aliases,
            max_depth: o.max_depth,
        }
    }
}

fn position(failure: &Failure) -> Option<f64> {
    failure.position.map(|p| p as f64)
}

/// Sets the `error*` / `fallback` fields of a result object.
fn set_failure(object: &mut Object<'_>, failure: Failure) -> napi::Result<()> {
    object.set("errorPosition", position(&failure))?;
    object.set("errorReason", failure.reason)?;
    if failure.fallback {
        object.set("fallback", true)?;
    }
    Ok(())
}

/// Packs `f64`s, then `u32`/`i32`s, then bytes into one buffer, in native byte order
/// (the `f64`s first keep them 8-byte aligned). `decode.cjs` takes typed-array views of it.
fn pack(numbers: &[f64], words: impl ExactSizeIterator<Item = [u8; 4]>, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(numbers.len() * 8 + words.len() * 4 + bytes.len());
    for n in numbers {
        out.extend_from_slice(&n.to_ne_bytes());
    }
    for w in words {
        out.extend_from_slice(&w);
    }
    out.extend_from_slice(bytes);
    out
}

/// `loadAll` with built-in tags only. Use `load` / `loadAll` of `index.cjs` instead.
///
/// Returns `{ json }`, or the value tape as `{ buf, numbersLength, opsLength,
/// bytesLength, strings }`, or `{ errorReason, errorPosition, fallback? }`. The tape is
/// copied into a Node-owned buffer: V8 tracks and frees that memory itself (handing over
/// Rust-owned memory as external `ArrayBuffer`s leaked).
#[napi(catch_unwind)]
#[allow(clippy::needless_pass_by_value)] // N-API hands us owned values
pub fn load_raw(
    env: &Env,
    source: String,
    options: Option<NativeOptions>,
) -> napi::Result<Object<'_>> {
    let mut object = Object::new(env)?;
    match jsyaml_tape::load(&source, &options.unwrap_or_default().into()) {
        Ok(Loaded::Json(json)) => object.set("json", json)?,
        Ok(Loaded::Tape(tape)) => {
            let buf = pack(
                &tape.numbers,
                tape.ops.iter().map(|w| w.to_ne_bytes()),
                &tape.bytes,
            );
            object.set("buf", &BufferSlice::copy_from(env, buf)?)?;
            object.set("numbersLength", tape.numbers.len() as f64)?;
            object.set("opsLength", tape.ops.len() as f64)?;
            object.set("bytesLength", tape.bytes.len() as f64)?;
            object.set("strings", tape.strings)?;
        }
        Err(failure) => set_failure(&mut object, failure)?,
    }
    Ok(object)
}

/// `parseEvents`. Use `parseEvents` of `index.cjs` instead.
///
/// Returns `{ buf, recordsLength, directives }` or `{ errorReason, errorPosition }`.
#[napi(catch_unwind)]
#[allow(clippy::needless_pass_by_value)]
pub fn events_raw(
    env: &Env,
    source: String,
    options: Option<NativeOptions>,
) -> napi::Result<Object<'_>> {
    let mut object = Object::new(env)?;
    match jsyaml_tape::events(&source, &options.unwrap_or_default().into()) {
        Ok(EventTape {
            records,
            directives,
        }) => {
            let buf = pack(&[], records.iter().map(|r| r.to_ne_bytes()), &[]);
            object.set("buf", &BufferSlice::copy_from(env, buf)?)?;
            object.set("recordsLength", records.len() as f64)?;
            object.set("directives", directives)?;
        }
        Err(failure) => set_failure(&mut object, failure)?,
    }
    Ok(object)
}
