//! Native runner for `bench/run.mjs`: `jsyaml-bench <file> <profile> <min-time-ms>` prints
//! `{"samples":[ms...]}`. It measures the Rust crate alone (YAML → Rust values), which no
//! JS caller gets: it is the ceiling for the bindings, not a competitor.

use std::time::Instant;

use jsyaml::{ConstructorOptions, LoadOptions, Schema, load_all};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [file, profile, min_time] = args.as_slice() else {
        return Err("usage: jsyaml-bench <file> <profile> <min-time-ms>".into());
    };
    let input = std::fs::read_to_string(file)?;
    let min_time: f64 = min_time.parse()?;
    let constructor = match profile.as_str() {
        "yaml11-merge" => ConstructorOptions {
            schema: Schema::yaml11(),
            max_total_merge_keys: None,
            ..ConstructorOptions::default()
        },
        _ => ConstructorOptions::default(),
    };
    let options = LoadOptions {
        constructor,
        ..LoadOptions::default()
    };

    // Warm-up and batch size, mirroring runners/js.mjs.
    let start = Instant::now();
    drop(load_all(&input, &options)?);
    let per_call = start.elapsed().as_secs_f64() * 1000.0;
    #[allow(clippy::cast_sign_loss)] // clamped to >= 1
    let batch = (5.0 / per_call.max(1e-3)).ceil().max(1.0) as usize;

    let mut samples = Vec::new();
    let mut total = 0.0;
    while (total < min_time || samples.len() < 10) && samples.len() < 500 {
        let start = Instant::now();
        for _ in 0..batch {
            drop(load_all(&input, &options)?);
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        samples.push(ms / batch as f64);
        total += ms;
    }
    println!("{}", json!({ "samples": samples }));
    Ok(())
}
