//! The JSON fast path must agree with the full constructor: whenever `construct_json`
//! produces JSON, `construct` succeeds on the same events, shares nothing, and builds the
//! same data.
#![no_main]

use jsyaml::{ConstructorOptions, LoadOptions, Schema};
use libfuzzer_sys::fuzz_target;
use serde_json::Value as Json;

/// Numbers as f64 everywhere: the fast path writes `1`, serde writes `1.0`.
fn canonical(value: &Json) -> Json {
    match value {
        Json::Number(n) => Json::from(n.as_f64().unwrap_or(f64::NAN)),
        Json::Array(items) => items.iter().map(canonical).collect(),
        Json::Object(map) => Json::Object(map.iter().map(|(k, v)| (k.clone(), canonical(v))).collect()),
        other => other.clone(),
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    let options = LoadOptions::default();
    let Ok(events) = jsyaml::parse_events(input, &options.parser) else { return };
    for schema in [Schema::core(), Schema::json(), Schema::yaml11()] {
        let constructor = ConstructorOptions { schema, ..ConstructorOptions::default() };
        let Some(fast) = jsyaml::construct_json(input, &events, &constructor) else { continue };
        let documents = jsyaml::construct(input, &events, &constructor)
            .unwrap_or_else(|e| panic!("fast path succeeded but construct failed: {e}"));
        assert!(documents.iter().all(|d| d.shared.is_empty()), "fast path on shared values");
        let fast: Json = serde_json::from_str(&fast).expect("fast path wrote invalid JSON");
        let slow = serde_json::to_value(&documents).expect("serialize documents");
        assert_eq!(canonical(&fast), canonical(&slow), "fast path differs for {input:?}");
    }
});
