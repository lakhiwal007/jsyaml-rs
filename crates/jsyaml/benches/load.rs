use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use jsyaml::{LoadOptions, ParserOptions, load_all, parse_events};

/// A k8s-style manifest: nested block mappings, sequences, quoted and plain scalars.
fn manifest(copies: usize) -> String {
    let doc = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: web\n  labels: {app: web, tier: \"frontend\"}\nspec:\n  replicas: 3\n  template:\n    spec:\n      containers:\n        - name: web\n          image: 'nginx:1.27'\n          ports: [80, 443]\n          env:\n            - {name: MODE, value: production}\n";
    let mut out = String::new();
    for _ in 0..copies {
        out.push_str("---\n");
        out.push_str(doc);
    }
    out
}

fn bench(c: &mut Criterion) {
    let input = manifest(2_000);
    let mut group = c.benchmark_group("manifest");
    group.throughput(Throughput::Bytes(input.len() as u64));
    group.bench_function("parse_events", |b| {
        b.iter(|| parse_events(black_box(&input), &ParserOptions::default()));
    });
    group.bench_function("load_all", |b| {
        b.iter(|| load_all(black_box(&input), &LoadOptions::default()));
    });
    group.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
