//! Measures parser stack use per nesting level, to size `HARD_DEPTH_LIMIT`.
//!
//! `stack_probe <flow|seq|map> <depth>` parses a nested input on a 1 MiB stack: exit 0 if
//! it fits, abort on overflow. Binary-search the depth from a shell, e.g.
//! `cargo build --release --example stack_probe` and loop over depths.

#![allow(clippy::unwrap_used)]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args[2].parse().unwrap();
    let input = match args[1].as_str() {
        "flow" => "[".repeat(n),
        "seq" => (0..n)
            .map(|i| format!("{}- ", " ".repeat(i * 2)))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => (0..n)
            .map(|i| format!("{}k:", " ".repeat(i)))
            .collect::<Vec<_>>()
            .join("\n"),
    };
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            drop(jsyaml::parse_events(
                &input,
                &jsyaml::ParserOptions {
                    max_depth: usize::MAX,
                },
            ));
        })
        .unwrap()
        .join()
        .unwrap();
}
