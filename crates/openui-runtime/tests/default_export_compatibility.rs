//! Implicit exports preserve the evaluation's limits; explicit export limits opt in.
use openui_lang::{Library, parse, parse_json, parse_with_limits};
use openui_runtime::{BindingProps, Limits, Store, TreeInput, evaluate_tree_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{},"d":{}}}}}"#;
fn export_limits() -> Limits {
    Limits::DEFAULT
        .with_work(usize::MAX)
        .with_depth(usize::MAX)
        .with_result_size(usize::MAX)
}
#[test]
fn default_tree_export_keeps_the_envelope_and_raw_cut_props() {
    let library = Library::from_json_schema(&parse_json(SCHEMA).unwrap());
    let list = (0..2000)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let k = (8 * 1024 * 1024 - 140_000 - 200) / 2000 - 3;
    for (pad, bytes, cut) in [(470, 8_388_616, false), (4000, 8_392_146, true)] {
        let source = format!(
            "root = Box(@Each([{list}], i, \"{}\"), \"{}\", \"{}\")",
            "x".repeat(k),
            "f".repeat(140_000),
            "p".repeat(pad)
        );
        let p = parse(&source, &library, None);
        let tree = evaluate_tree_with_limits(
            TreeInput::new(&p).with_library(&library),
            &Store::new(),
            Limits::DEFAULT,
        );
        assert_eq!((tree.size_limited, tree.depth_limited), (cut, false));
        let expected = tree.to_json_with_limits(export_limits()).unwrap();
        assert_eq!(expected.len(), bytes);
        assert_eq!(tree.to_json().unwrap(), expected);
    }
}
#[test]
fn default_tree_export_keeps_a_raised_evaluation_depth() {
    let library = Library::from_json_schema(&parse_json(SCHEMA).unwrap());
    let source = format!("root=Box({}1{})", "[".repeat(1000), "]".repeat(1000));
    let limits = Limits::DEFAULT.with_depth(4000);
    let p = parse_with_limits(&source, &library, None, limits);
    let tree = evaluate_tree_with_limits(TreeInput::new(&p), &Store::new(), limits);
    assert!(!tree.size_limited && !tree.depth_limited);
    assert_eq!(
        tree.to_json().unwrap(),
        tree.to_json_with_limits(export_limits()).unwrap()
    );
}
#[test]
fn default_store_snapshot_exports_even_when_empty_state_costs_zero() {
    for ceiling in [0, 1, 2] {
        let store = Store::with_limits(Limits::DEFAULT.with_result_size(ceiling));
        assert_eq!(store.snapshot_json().unwrap(), "{}");
        assert_eq!(store.version(), 0);
    }
}
fn spec(components: usize, props: usize) -> openui_runtime::Value {
    let mut cs = Vec::new();
    let mut defs = Vec::new();
    for c in 0..components {
        let names = (0..props)
            .map(|p| format!("p{}{p}", "n".repeat(20)))
            .collect::<Vec<_>>();
        let signature = names
            .iter()
            .map(|p| format!("{p}?: $binding<string>"))
            .collect::<Vec<_>>()
            .join(", ");
        cs.push(format!(r#""C{c}":{{"signature":"C{c}({signature})"}}"#));
        let ps = names
            .iter()
            .map(|p| format!(r#""{p}":{{}}"#))
            .collect::<Vec<_>>()
            .join(",");
        defs.push(format!(r#""C{c}":{{"properties":{{{ps}}}}}"#));
    }
    let source = format!(
        r#"{{"components":{{{}}},"schema":{{"$defs":{{{}}}}}}}"#,
        cs.join(","),
        defs.join(",")
    );
    openui_runtime::Value::from_json(&parse_json(&source).unwrap())
}
#[test]
fn ordinary_prompt_specs_use_linear_admission_instead_of_a_quadratic_estimate() {
    for (components, props) in [(1, 1200), (11, 300), (60, 150), (100, 100), (200, 50)] {
        let spec = spec(components, props);
        let bytes = openui_runtime::value_to_json_with_limits(&spec, export_limits())
            .unwrap()
            .len();
        for bindings in [
            BindingProps::from_prompt_spec(&spec).unwrap(),
            BindingProps::from_prompt_spec_with_limits(
                &spec,
                Limits::DEFAULT.with_work(bytes.saturating_mul(8)),
            )
            .unwrap(),
        ] {
            assert_eq!(bindings.undecided().count(), 0);
            for c in 0..components {
                for p in 0..props {
                    assert!(
                        bindings.contains(&format!("C{c}"), &format!("p{}{p}", "n".repeat(20)))
                    );
                }
            }
        }
    }
}
#[test]
fn default_binding_names_do_not_acquire_an_escaped_json_quota() {
    let mut bindings = BindingProps::new();
    let name = "\u{1}".repeat(2_000_000);
    bindings.insert(&name, "a").unwrap();
    assert!(bindings.contains(&name, "a"));
    let definitions = parse_json("{}").unwrap();
    let mut functions = openui_runtime::LibraryFunctions::new(&definitions);
    functions.register(&name, |_| Ok(None)).unwrap();
}
