//! Deterministic outputs and exact work/room thresholds for generated trees.
use openui_lang::{Library, ParseResult, parse, parse_json};
use openui_runtime::{Limits, Store, TreeInput, Value, evaluate_tree_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn threshold(p: &ParseResult, store: &Store, work: bool) -> usize {
    let (mut lo, mut hi) = (0, 1_000_000);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let limits = Limits::DEFAULT
            .with_work(if work { mid } else { 1_000_000 })
            .with_result_size(if work { 1_000_000 } else { mid });
        let result = evaluate_tree_with_limits(TreeInput::new(p), store, limits);
        if result.size_limited || result.depth_limited {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    lo
}
fn main() {
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}}}"#).unwrap(),
    );
    let mut seed = 0x736166657479u64;
    let count = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096);
    for i in 0..count {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let n = 1 + (seed as usize % 8);
        let list = (0..n).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
        let templates = [
            format!("@Each(L,i,i+{})", seed % 31),
            "@Each(L,i,L)".into(),
            "@Each(L,i,Box(i+$x))".into(),
            "[$x,$x+1,$x+2]".into(),
            "$x>0?L:[0]".into(),
            "@Count(L)".into(),
            format!("@Each(L,i,{{a:i,b:[i,i+1],s:\"s{}\"}})", seed % 127),
            "@Format($x)".into(),
        ];
        let source = format!(
            "$x={i}\nL=[{list}]\nroot=Box({},$x,Box(L))",
            templates[(seed >> 32) as usize % templates.len()]
        );
        let parsed = parse(&source, &library, None);
        for data in [false, true] {
            let mut p = parsed.clone();
            if data && let Some(root) = p.root.as_mut() {
                let mut fields = openui_runtime::JsObject::new();
                for (key, v) in root.props.iter() {
                    let v = if let Value::Ast(a) = v {
                        Value::from_json(&parse_json(&a.to_json()).unwrap())
                    } else {
                        v.clone()
                    };
                    fields.define(key, v);
                }
                root.props = fields;
            }
            let mut store = Store::new();
            let initialized = store.initialize(
                p.state_declarations
                    .iter()
                    .map(|(k, v)| (k.as_str(), Some(v))),
                [],
            );
            assert!(initialized.refused.is_empty());
            let result = evaluate_tree_with_limits(TreeInput::new(&p), &store, Limits::DEFAULT);
            println!(
                "{i}/{data}\t{}\t{}\t{}\t{}\t{}",
                threshold(&p, &store, true),
                threshold(&p, &store, false),
                result.size_limited,
                result.depth_limited,
                result.to_json().unwrap()
            );
        }
    }
}
