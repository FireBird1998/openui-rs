//! Exact generated work pins and the independently archived main budget.
#[path = "support/reuse.rs"]
#[allow(dead_code)]
mod reuse;
use openui_lang::{Library, parse};
use openui_runtime::{Limits, TreeInput, evaluate_tree_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(1024 * 1024 * 1024);
#[test]
fn generated_function_member_marker_and_loop_work_never_exceeds_main() {
    let j = reuse::schema();
    let l = Library::from_json_schema(&j);
    let f = reuse::functions(&j);
    let (s, refs) = reuse::supplied(1000);
    // main work was measured from a dedicated git archive db80c86 target.
    // 204 deterministic text/data configurations retained from the larger run.
    for line in include_str!("support/reuse-work.tsv").lines() {
        let row = line.split('\t').collect::<Vec<_>>();
        let seed = row[0].parse::<usize>().unwrap();
        let data = row[1] == "true";
        let main = row[2].parse::<usize>().unwrap();
        let work = row[3].parse::<usize>().unwrap();
        let source = reuse::source(seed);
        let p = parse(&source, &l, None);
        let p = if data { reuse::shaped(p) } else { p };
        assert!(work <= main, "seed={seed} work={work} main={main}");
        for (units, cut) in [(work, false), (work - 1, true), (main, false)] {
            let r = evaluate_tree_with_limits(
                TreeInput::new(&p)
                    .with_library(&l)
                    .with_functions(&f)
                    .with_reference_values(&refs),
                &s,
                Limits::DEFAULT
                    .with_result_size(128_000_000)
                    .with_work(units),
            );
            assert_eq!(
                r.size_limited, cut,
                "seed={seed} data={data} units={units} source={source}"
            );
            assert!(!r.depth_limited);
        }
    }
}

#[test]
fn an_uncached_depth_check_reads_structure_without_reading_text_bytes() {
    use openui_runtime::{EvaluationSession, Value};
    use std::sync::Arc;
    for form in 0..3 {
        let mut value = match form {
            0 => Value::Str("x".repeat(100_000)),
            1 => Value::Ast(Arc::new(openui_lang::Ast::Str("x".repeat(100_000)))),
            _ => {
                let mut o = openui_lang::JsObject::new();
                o.define("k", Value::Str("Str".into()));
                o.define("v", Value::Str("x".repeat(100_000)));
                Value::Obj(Arc::new(o))
            }
        };
        for _ in 0..16 {
            value = Value::Arr(Arc::new(vec![value]));
        }
        let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(1024));
        session.with_evaluator(|eval| {
            assert!(eval.check_depth(&value));
            assert!(eval.charge_work(900));
        });
        assert!(!session.flags().size_limited && !session.flags().depth_limited);
    }
}
