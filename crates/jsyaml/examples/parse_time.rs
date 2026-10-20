//! `parse_time <file>...`: median time of `parse_events`, `construct` (events → values) and
//! the JSON transport encoding, per file. A quick profiling aid; use `just bench-compare`
//! for real numbers.

use std::time::Instant;

fn median(mut f: impl FnMut()) -> f64 {
    let mut times: Vec<f64> = (0..7)
        .map(|_| {
            let start = Instant::now();
            f();
            start.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[3]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for file in std::env::args().skip(1) {
        let input = std::fs::read_to_string(&file)?;
        let options = jsyaml::LoadOptions::default();
        let events = jsyaml::parse_events(&input, &options.parser)?;
        let parse = median(|| {
            drop(jsyaml::parse_events(&input, &options.parser));
        });
        let construct = median(|| {
            drop(jsyaml::construct(&input, &events, &options.constructor));
        });
        let fast = jsyaml::construct_json(&input, &events, &options.constructor).is_some();
        let json = median(|| {
            drop(jsyaml::construct_json(
                &input,
                &events,
                &options.constructor,
            ));
        });
        println!(
            "{file}: parse {parse:.1} ms, construct {construct:.1} ms, construct_json {json:.1} ms (applies: {fast}), {} events",
            events.len()
        );
    }
    Ok(())
}
