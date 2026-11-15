//! Compare deterministic public evaluation work and peak allocations across revisions.
#[path = "../tests/support/reuse.rs"]
#[allow(dead_code)]
mod reuse;
use openui_lang::{Library, parse};
use openui_runtime::{Limits, Store, TreeInput, Value, evaluate_tree_with_limits, run_action};
use std::{
    io::{BufWriter, Write},
    sync::Arc,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);
fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "costs".into());
    let mut j = reuse::schema();
    if mode == "actions-typed" || mode == "actions-returns" {
        j["functions"]["Id"]["params"]["type"] = openui_lang::parse_json("\"object\"").unwrap();
    }
    if mode == "actions-returns" {
        j["functions"]["Id"]["returns"] = openui_lang::parse_json("{}").unwrap();
    }
    let l = Library::from_json_schema(&j);
    let f = reuse::functions(&j);
    let mut out = BufWriter::new(std::io::stdout().lock());
    if mode == "costs" {
        let count = std::env::args()
            .nth(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(4096);
        let (s, refs) = reuse::supplied(1000);
        for seed in 0..count {
            let src = reuse::source(seed);
            for data in [false, true] {
                let p = parse(&src, &l, None);
                let p = if data { reuse::shaped(p) } else { p };
                let input = TreeInput::new(&p)
                    .with_library(&l)
                    .with_functions(&f)
                    .with_reference_values(&refs);
                let run = |units| {
                    evaluate_tree_with_limits(
                        input.clone(),
                        &s,
                        Limits::DEFAULT
                            .with_result_size(128_000_000)
                            .with_work(units),
                    )
                };
                let (mut lo, mut hi) = (0, 64_000_000);
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let r = run(mid);
                    if r.size_limited || r.depth_limited {
                        lo = mid + 1
                    } else {
                        hi = mid
                    }
                }
                test_alloc::reset_peak();
                let before = test_alloc::peak();
                let r = run(1_000_000_000);
                let peak = test_alloc::peak() - before;
                let json = r.to_json().unwrap();
                let bytes = json.len();
                let hash = json.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
                });
                writeln!(
                    out,
                    "{seed}\t{data}\t{lo}\t{peak}\t{}\t{bytes}\t{hash:016x}\t{src:?}",
                    r.size_limited || r.depth_limited
                )
                .unwrap();
            }
        }
    } else if mode == "loops" {
        let (mut s, _) = reuse::supplied(1000);
        for family in ["element", "array"] {
            for items in [1, 3] {
                for levels in 1..=8 {
                    s.set(
                        "$rows",
                        Some(Value::Arr(Arc::new(vec![Value::Num(1.); items]))),
                    )
                    .unwrap();
                    let mut b = "Box($s)".to_owned();
                    for n in 0..levels {
                        b = format!(
                            "@Each($rows,i{n},{})",
                            if family == "element" {
                                format!("Box({b})")
                            } else {
                                b
                            }
                        );
                    }
                    let src = format!("root=Box({b})");
                    let p = parse(&src, &l, None);
                    for room in [65536, 1_048_576, 4_100_000, 128_000_000] {
                        test_alloc::reset_peak();
                        let before = test_alloc::peak();
                        let r = evaluate_tree_with_limits(
                            TreeInput::new(&p),
                            &s,
                            Limits::DEFAULT
                                .with_work(1_000_000_000)
                                .with_result_size(room),
                        );
                        let peak = test_alloc::peak() - before;
                        writeln!(
                            out,
                            "{family}\t{items}\t{levels}\t{room}\t{peak}\t{}\t{}",
                            r.size_limited,
                            r.to_json().unwrap().len()
                        )
                        .unwrap();
                    }
                }
            }
        }
    } else if mode.starts_with("actions") {
        for body in [
            "[$d]",
            "@Id($d)",
            "[$d,$d]",
            "[$d,[$d]]",
            "[[$d],$d]",
            "[$d,$alias]",
            "@Id($alias)",
        ] {
            for data in [false, true] {
                let p = parse(&format!("root=Box(Action([@Set($y,{body})]))"), &l, None);
                let p = if data { reuse::shaped(p) } else { p };
                let tree = evaluate_tree_with_limits(
                    TreeInput::new(&p).with_library(&l).with_functions(&f),
                    &Store::new(),
                    Limits::DEFAULT,
                );
                let Some(Value::Element(root)) = &tree.root else {
                    continue;
                };
                for n in 730..=770 {
                    let mut s = Store::new();
                    let d = reuse::deep(n, false);
                    s.set("$d", Some(d.clone())).unwrap();
                    s.set("$alias", Some(d)).unwrap();
                    let r = run_action(
                        TreeInput::new(&p).with_library(&l).with_functions(&f),
                        &mut s,
                        root.props.get("a"),
                        "",
                        None,
                    );
                    writeln!(
                        out,
                        "{body}\t{data}\t{n}\t{}\t{}",
                        r.depth_limited, r.size_limited
                    )
                    .unwrap();
                }
            }
        }
    }
}
