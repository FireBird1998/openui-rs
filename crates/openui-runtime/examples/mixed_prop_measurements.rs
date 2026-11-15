//! Measure exact work and output hashes for elements mixed with ordinary data.
#[path = "../tests/support/mixed.rs"]
#[allow(dead_code)]
mod mixed;
use openui_lang::{Library, parse};
use openui_runtime::{Limits, TreeInput, evaluate_tree_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "cases".into());
    if mode == "cases" {
        for (source, n) in [
            ("root=Box(($n>2?[$el,$s]:0))", 1000),
            ("root=Box(($n>2?[$el,$s]:0))", 10_000),
            ("root=Box(($n>2?[$el,$s]:0))", 100_000),
            ("root=Box(@Grow([$s,$el]))", 100_000),
            ("root=Box(@Two($s,$el))", 100_000),
            ("v=@Grow([$s,$el])\nroot=Box(v,v)", 100_000),
            ("root=Box(($n>2?[$marker,$data.s]:@Throw()))", 200),
        ] {
            for data in [false, true] {
                println!(
                    "{n}\t{data}\t{}\t{source:?}",
                    mixed::minimum(source, n, data)
                );
            }
        }
    } else {
        let count = std::env::args()
            .nth(2)
            .and_then(|s| s.parse().ok())
            .unwrap_or(4096);
        let j = mixed::schema();
        let l = Library::from_json_schema(&j);
        let f = mixed::functions(&j);
        for seed in 0..count {
            let source = mixed::source(seed);
            for data in [false, true] {
                let p = parse(&source, &l, None);
                let p = if data { mixed::shaped(p) } else { p };
                assert!(p.root.is_some() || p.root_expression.is_some(), "{source}");
                let s = mixed::supplied(1000, data);
                let input = TreeInput::new(&p).with_library(&l).with_functions(&f);
                let (mut lo, mut hi) = (0, 20_000_000);
                while lo < hi {
                    let mid = lo + (hi - lo) / 2;
                    let r = evaluate_tree_with_limits(
                        input.clone(),
                        &s,
                        Limits::DEFAULT.with_work(mid).with_result_size(64_000_000),
                    );
                    if r.size_limited || r.depth_limited {
                        lo = mid + 1
                    } else {
                        hi = mid
                    }
                }
                let r = evaluate_tree_with_limits(
                    input,
                    &s,
                    Limits::DEFAULT
                        .with_work(20_000_000)
                        .with_result_size(64_000_000),
                );
                assert!(!r.size_limited && !r.depth_limited, "{source}");
                let json = r.to_json().unwrap();
                let hash = json.bytes().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
                });
                println!(
                    "{seed}\t{data}\t{lo}\t{}\t{hash:016x}\t{source:?}",
                    json.len()
                );
            }
        }
    }
}
