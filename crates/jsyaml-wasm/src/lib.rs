//! WebAssembly build of the loader, for platforms without a prebuilt native addon and for
//! browsers. Same JS API and results as `jsyaml-node` (both use `jsyaml-tape`).

use js_sys::{Array, Float64Array, Int32Array, Object, Reflect, Uint8Array, Uint32Array};
use jsyaml_tape::{Failure, Loaded, Options};
use wasm_bindgen::prelude::*;

fn get(options: &JsValue, key: &str) -> JsValue {
    if options.is_object() {
        Reflect::get(options, &key.into()).unwrap_or(JsValue::UNDEFINED)
    } else {
        JsValue::UNDEFINED
    }
}

fn options(js: &JsValue) -> Options {
    let tags = get(js, "tags");
    Options {
        tags: if Array::is_array(&tags) {
            Array::from(&tags)
                .iter()
                .filter_map(|t| t.as_string())
                .collect()
        } else {
            Vec::new()
        },
        json: get(js, "json").as_bool(),
        max_total_merge_keys: get(js, "maxTotalMergeKeys").as_f64(),
        max_aliases: get(js, "maxAliases").as_f64(),
        max_depth: get(js, "maxDepth").as_f64(),
    }
}

fn object(fields: &[(&str, JsValue)]) -> JsValue {
    let out = Object::new();
    for (key, value) in fields {
        // Setting a property on a fresh plain object can't fail.
        let _ = Reflect::set(&out, &(*key).into(), value);
    }
    out.into()
}

fn failure(f: Failure) -> JsValue {
    let position = f
        .position
        .map_or(JsValue::NULL, |p| JsValue::from(p as f64));
    // Rerun with js-yaml's constructor (see `jsyaml_tape::Failure::fallback`).
    let fallback = if f.fallback {
        JsValue::TRUE
    } else {
        JsValue::UNDEFINED
    };
    object(&[
        ("errorReason", f.reason.into()),
        ("errorPosition", position),
        ("fallback", fallback),
    ])
}

/// `loadAll` with built-in tags only. Use `load` / `loadAll` of `index.cjs` instead.
#[wasm_bindgen(js_name = loadRaw)]
pub fn load_raw(source: &str, options_js: &JsValue) -> JsValue {
    match jsyaml_tape::load(source, &options(options_js)) {
        Ok(Loaded::Json(json)) => object(&[("json", json.into())]),
        Ok(Loaded::Tape(tape)) => object(&[
            ("ops", Uint32Array::from(tape.ops.as_slice()).into()),
            (
                "numbers",
                Float64Array::from(tape.numbers.as_slice()).into(),
            ),
            ("strings", tape.strings.into()),
            ("bytes", Uint8Array::from(tape.bytes.as_slice()).into()),
        ]),
        Err(f) => failure(f),
    }
}

/// `parseEvents`. Use `parseEvents` of `index.cjs` instead.
#[wasm_bindgen(js_name = eventsRaw)]
pub fn events_raw(source: &str, options_js: &JsValue) -> JsValue {
    match jsyaml_tape::events(source, &options(options_js)) {
        Ok(tape) => object(&[
            ("records", Int32Array::from(tape.records.as_slice()).into()),
            ("directives", tape.directives.into()),
        ]),
        Err(f) => failure(f),
    }
}
