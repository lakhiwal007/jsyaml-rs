//! Any input must produce documents or an `Error`, never a panic: in the napi addon a
//! panic becomes a JS exception at best, and a crashed process if it happens in a
//! destructor.
#![no_main]

use jsyaml::{ConstructorOptions, LoadOptions, Schema};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(input) = std::str::from_utf8(data) else { return };
    for schema in [Schema::core(), Schema::yaml11()] {
        let options = LoadOptions {
            constructor: ConstructorOptions { schema, ..ConstructorOptions::default() },
            ..LoadOptions::default()
        };
        if let Err(e) = jsyaml::load_all(input, &options) {
            // Error positions must be valid offsets.
            if let Some(p) = e.position() {
                assert!(p <= input.len() && input.is_char_boundary(p));
            }
        }
    }
});
