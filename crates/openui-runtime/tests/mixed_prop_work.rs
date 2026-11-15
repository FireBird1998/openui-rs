//! Rebuilding an element does not charge its untouched neighboring data twice.
#[path = "support/mixed.rs"]
#[allow(dead_code)]
mod mixed;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
#[test]
fn rebuilding_an_element_preserves_the_work_for_neighboring_text() {
    // Main's native threshold is 3*N+485; both columns were measured from
    // git archive db80c86 using the same schema, callbacks and supplied data.
    for n in [1000, 10_000, 100_000] {
        for data in [false, true] {
            let work = mixed::minimum("root=Box(($n>2?[$el,$s]:0))", n, data);
            assert_eq!(work, 3 * n + if data { 525 } else { 440 });
            assert!(work <= 3 * n + if data { 639 } else { 485 });
        }
    }
}

#[test]
fn mixed_prop_work_matches_the_recorded_program_budgets() {
    use openui_lang::{Library, parse};
    use openui_runtime::{Limits, TreeInput, evaluate_tree_with_limits};
    let j = mixed::schema();
    let l = Library::from_json_schema(&j);
    let f = mixed::functions(&j);
    // Columns: generator seed, data form, archived db80c86 work, current work.
    // Each row is an exact fit and a one-unit-below cut.
    for line in include_str!("support/mixed-work.tsv").lines() {
        let row = line.split('\t').collect::<Vec<_>>();
        let seed = row[0].parse::<usize>().unwrap();
        let data = row[1] == "true";
        let main = row[2].parse::<usize>().unwrap();
        let work = row[3].parse::<usize>().unwrap();
        let source = mixed::source(seed);
        let p = parse(&source, &l, None);
        let p = if data { mixed::shaped(p) } else { p };
        let s = mixed::supplied(1000, data);
        assert!(work <= main, "{source}");
        for (units, cut) in [(work, false), (work - 1, true), (main, false)] {
            let r = evaluate_tree_with_limits(
                TreeInput::new(&p).with_library(&l).with_functions(&f),
                &s,
                Limits::DEFAULT
                    .with_work(units)
                    .with_result_size(64_000_000),
            );
            assert_eq!(
                r.size_limited, cut,
                "seed={seed} data={data} work={units} {source}"
            );
            assert!(!r.depth_limited);
        }
        // Reusing a projection's measured bytes must still reserve its entire
        // retained output. Check exact room boundaries independently of work.
        let run = |room| {
            evaluate_tree_with_limits(
                TreeInput::new(&p).with_library(&l).with_functions(&f),
                &s,
                Limits::DEFAULT.with_work(20_000_000).with_result_size(room),
            )
        };
        let (mut lo, mut hi) = (0, 64_000_000);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let r = run(mid);
            if r.size_limited || r.depth_limited {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let r = run(lo);
        assert!(!r.size_limited && !r.depth_limited);
        assert!(
            root_bytes(&r.root) <= lo,
            "seed={seed} data={data} room={lo}"
        );
        assert!(run(lo - 1).size_limited);
    }
}
fn root_bytes(root: &Option<openui_runtime::Value>) -> usize {
    use openui_lang::{Element, JsObject, ParseResult};
    let mut props = JsObject::new();
    props.define("v", root.clone().unwrap_or(openui_runtime::Value::Null));
    let json = ParseResult {
        root: Some(Element {
            type_name: "".into(),
            props,
            partial: false,
            has_dynamic_props: false,
            statement_id: None,
        }),
        ..ParseResult::default()
    }
    .to_json();
    let start = json.find("\"v\":").unwrap() + 4;
    json.rfind("},\"partial\":false,\"hasDynamicProps\":false},\"meta\":")
        .unwrap()
        - start
}
