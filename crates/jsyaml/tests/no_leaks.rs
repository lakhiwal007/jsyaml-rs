//! Heap-accounting leak test. A tracking allocator counts live heap bytes; after each
//! load result is dropped, the live total must return exactly to its baseline.
//! Runs without the libtest harness so no other test allocates concurrently.

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn live_bytes() -> usize {
    dhat::HeapStats::get().curr_bytes
}

fn main() {
    let _profiler = dhat::Profiler::builder().testing().build();
    let inputs = [
        "a: &x [1, 2]\nb: *x\nc: {d: 'e', f: \"g\\n\"}\n",
        "--- !!set\n? a\n? b\n--- [1, 2\n",
        "key: [unterminated\n",
    ];
    let options = jsyaml::LoadOptions::default();
    let baseline = live_bytes();
    for input in inputs {
        for _ in 0..50 {
            drop(jsyaml::load_all(input, &options));
            drop(jsyaml::parse_events(input, &options.parser));
        }
        let live = live_bytes();
        assert_eq!(live, baseline, "leak after loading {input:?}");
    }
}
