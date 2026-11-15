//! Eager temporary arrays can be larger than the final result allowance.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{
    LibraryFunctions, Limits, Store, TreeInput, Value, evaluate_tree_with_limits,
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}
#[test]
fn an_index_chosen_after_all_callbacks_observes_an_uncut_temporary_array() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Make":{"params":{"type":"object","properties":{}},"returns":{"type":"string"}},"Pick":{"params":{"type":"object","properties":{}},"returns":{"type":"number"}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    for (n, room) in [
        (32, 8192),
        (64, 12288),
        (128, 20480),
        (256, 36864),
        (1024, 135168),
        (32768, 8192),
    ] {
        let leaf = room / 4;
        let values = (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let text = format!("root=Box(@Each([{values}],i,@Make())[@Pick()])");
        let p = parse(&text, &library, None);
        let calls = AtomicUsize::new(0);
        let picks = AtomicUsize::new(0);
        let state = Mutex::new((0x3141592653589793u64, Vec::new()));
        let mut functions = LibraryFunctions::new(&schema["functions"]);
        functions
            .register("Make", |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                let mut state = state.lock().unwrap();
                let mut payload = String::with_capacity(leaf);
                for _ in 0..leaf {
                    state.0 ^= state.0 << 13;
                    state.0 ^= state.0 >> 7;
                    state.0 ^= state.0 << 17;
                    payload.push((b'a' + (state.0 % 26) as u8) as char);
                }
                state.1.push(digest(&payload));
                Ok(Some(Value::Str(payload)))
            })
            .unwrap();
        functions
            .register("Pick", |_| {
                assert_eq!(calls.load(Ordering::Relaxed), n);
                picks.fetch_add(1, Ordering::Relaxed);
                Ok(Some(Value::Num((n / 2) as f64)))
            })
            .unwrap();
        let store = Store::new();
        test_alloc::reset_peak();
        let baseline = test_alloc::peak();
        let r = evaluate_tree_with_limits(
            TreeInput::new(&p)
                .with_library(&library)
                .with_functions(&functions),
            &store,
            Limits::DEFAULT
                .with_result_size(room)
                .with_work(4_000_000_000),
        );
        let peak = test_alloc::peak() - baseline;
        assert!(
            !r.size_limited && !r.depth_limited,
            "{n}: {}",
            r.to_json().unwrap()
        );
        assert!(r.errors.is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), n);
        assert_eq!(picks.load(Ordering::Relaxed), 1);
        let Some(Value::Element(root)) = &r.root else {
            panic!("element")
        };
        let Some(Value::Str(selected)) = root.props.get("a") else {
            panic!("selected payload")
        };
        assert_eq!(digest(selected), state.lock().unwrap().1[n / 2]);
        // Record the witness, without requiring an inefficient minimum peak:
        // a future compact representation must still pass the semantic pins.
        println!(
            "n={n} room={room} input_text={} program_json={} opaque_produced={} peak={peak} output={}",
            text.len(),
            p.to_json().len(),
            n * leaf,
            r.to_json().unwrap().len()
        );
    }
}
