//! Reference rules that affect parser flags, name scope and validation.
use openui_lang::{Ast, Library, Value, parse};
use serde_json::json;

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

#[test]
fn element_flags_use_nullish_fallback_and_javascript_truthiness() {
    let lib = Library::from_json_schema(&json!({"$defs":{"Box":{"properties":{"a":{}}}}}));
    for flag in [
        None,
        Some("null"),
        Some("true"),
        Some("false"),
        Some("1"),
        Some("0"),
        Some("\"yes\""),
        Some("\"\""),
        Some("[]"),
        Some("{}"),
    ] {
        for dynamic in [false, true] {
            let own = flag
                .map(|v| format!(",hasDynamicProps:{v}"))
                .unwrap_or_default();
            let props = if dynamic {
                "{a:{k:\"StateRef\",n:\"$x\"}}"
            } else {
                "{}"
            };
            let text = format!(
                "root=Box({{type:\"element\",typeName:\"Box\",partial:false{own},props:{props}}})\n$x=7"
            );
            let result = parse(&text, &lib, None);
            let expected = match flag {
                None | Some("null") => dynamic,
                Some("false" | "0" | "\"\"") => false,
                _ => true,
            };
            assert_eq!(result.root.unwrap().has_dynamic_props, expected, "{text}");
        }
    }
}

#[test]
fn a_declared_each_disables_iterator_scope_in_values_and_expressions() {
    for kind in ["functions", "actions"] {
        let mut schema = json!({"$defs":{"Box":{"properties":{"a":{},"b":{}}}}});
        schema[kind] = json!({"Each":{"params":{"properties":{"x":{}}}}});
        let lib = Library::from_json_schema(&schema);
        let result = parse("root=Box(@Each(0,x,x),true?@Each(0,x,x):null)", &lib, None);
        assert_eq!(result.meta.unresolved, ["x", "x", "x", "x"]);
        assert_eq!(
            result
                .meta
                .errors
                .iter()
                .filter(|e| e.code.as_str() == "excess-args")
                .count(),
            2
        );
        for prop in ["a", "b"] {
            let Value::Ast(ast) = result.root.as_ref().unwrap().props.get(prop).unwrap() else {
                panic!("call lost")
            };
            let node = match ast.as_ref() {
                Ast::Ternary { then, .. } => then.as_ref(),
                node => node,
            };
            let Ast::Comp {
                args, mapped_props, ..
            } = node
            else {
                panic!("call lost")
            };
            assert!(matches!(&args[1],Ast::Ph(n) if n=="x"));
            assert!(mapped_props.as_ref().unwrap().has_own("x"));
        }
    }
}

#[test]
fn inherited_schema_properties_validate_inert_object_and_function_values() {
    let names = [
        "__proto__",
        "constructor",
        "toString",
        "hasOwnProperty",
        "__defineGetter__",
        "__defineSetter__",
        "__lookupGetter__",
        "__lookupSetter__",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "toLocaleString",
        "valueOf",
    ];
    for name in names {
        for required in [false, true] {
            let mut sub = json!({"type":"object","properties":{name:{"type":"string"}}});
            if required {
                sub["required"] = json!([name]);
            }
            let lib = Library::from_json_schema(&json!({"$defs":{"Box":{"properties":{"a":sub}}}}));
            let r = parse("root=Box({})", &lib, None);
            assert_eq!(r.meta.errors.len(), 1, "{name}: {:?}", r.meta.errors);
            assert_eq!(
                r.meta.errors[0].message,
                format!(
                    "field \"/a/{name}\" expects string but got {}",
                    if name == "__proto__" {
                        "object"
                    } else {
                        "function"
                    }
                )
            );
            assert_eq!(r.root.unwrap().props.has_own("a"), !required, "{name}");
        }
        let lib = Library::from_json_schema(
            &json!({"$defs":{"Box":{"properties":{"a":{"type":"object","required":[name]}}}}}),
        );
        let r = parse("root=Box({})", &lib, None);
        assert!(r.meta.errors.is_empty(), "required inherited {name}");
        assert!(r.root.unwrap().props.has_own("a"));
    }
}

#[test]
fn the_inert_object_prototype_has_a_null_parent() {
    let lib = Library::from_json_schema(&json!({"$defs":{"Box":{"properties":{"a":{
        "type":"object","properties":{"__proto__":{
            "type":"object","required":["__proto__"],
            "properties":{"__proto__":{"type":"string"}}
        }}
    }}}}}));
    let r = parse("root=Box({})", &lib, None);
    assert_eq!(r.meta.errors.len(), 1);
    assert_eq!(
        r.meta.errors[0].message,
        "required field \"/a/__proto__/__proto__\" cannot be null"
    );
    assert!(r.root.unwrap().props.has_own("a"));
}

#[test]
fn a_null_prototype_removes_inherited_required_properties() {
    for name in [
        "__proto__",
        "constructor",
        "toString",
        "hasOwnProperty",
        "__defineGetter__",
        "__defineSetter__",
        "__lookupGetter__",
        "__lookupSetter__",
        "isPrototypeOf",
        "propertyIsEnumerable",
        "toLocaleString",
        "valueOf",
    ] {
        let lib = Library::from_json_schema(&json!({"$defs":{"Box":{"properties":{"a":{
            "type":"object","required":[name],"properties":{name:{"type":"string"}}
        }}}}}));
        let result = parse("root=Box({__proto__:null})", &lib, None);
        assert_eq!(result.meta.errors.len(), 1, "{name}");
        assert_eq!(
            result.meta.errors[0].message,
            format!("missing required field \"/a/{name}\"")
        );
        assert!(!result.root.unwrap().props.has_own("a"));
    }
    let lib = Library::from_json_schema(&json!({"$defs":{"Box":{"properties":{"a":{}}}}}));
    let result = parse("root=Box({__proto__:null,__proto__:\"own\"})", &lib, None);
    let Value::Obj(obj) = result.root.as_ref().unwrap().props.get("a").unwrap() else {
        panic!("object lost")
    };
    assert_eq!(obj.get("__proto__"), Some(&Value::Str("own".into())));
}

#[test]
fn literal_call_arguments_use_the_same_inert_prototype_assignment() {
    let lib = Library::from_json_schema(&json!({
        "$defs":{"Box":{"properties":{"a":{}}}},
        "functions":{"Probe":{"params":{"properties":{"value":{
            "type":"object","required":["constructor"],
            "properties":{"constructor":{"type":"string"}}
        }}}}}
    }));
    let result = parse("root=Box(@Probe({__proto__:null}))", &lib, None);
    assert_eq!(result.meta.errors.len(), 1);
    assert_eq!(
        result.meta.errors[0].message,
        "missing required field \"/value/constructor\""
    );
}

#[test]
fn validation_spread_restores_ordinary_inheritance() {
    let lib = Library::from_json_schema(&json!({"$defs":{"Box":{"properties":{"a":{
        "type":"object","properties":{"x":{"type":"number","default":1}}
    }}}}}));
    let result = parse("root=Box({__proto__:null,x:\"bad\"})", &lib, None);
    let Value::Obj(obj) = result.root.as_ref().unwrap().props.get("a").unwrap() else {
        panic!("object lost")
    };
    assert_eq!(obj.get("x"), Some(&Value::Num(1.0)));
    assert!(obj.has("constructor"));
    assert!(!obj.has_own("constructor"));
}

#[test]
fn discarded_prototype_assignments_keep_value_equality_and_debug() {
    let lib = Library::from_json_schema(&json!({"$defs":{"Box":{"properties":{"a":{}}}}}));
    let ordinary = parse("root=Box({})", &lib, None).root.unwrap();
    let assigned = parse("root=Box({__proto__:$v})", &lib, None).root.unwrap();
    let a = ordinary.props.get("a").unwrap();
    let b = assigned.props.get("a").unwrap();
    assert_eq!(a, b);
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
    assert!(!assigned.has_dynamic_props);
}

/// Generated by reference/parser-call-results.ts with the pinned compileSchema and parse.
fn call_results() -> impl Iterator<Item = serde_json::Value> {
    include_str!("../../../conformance/golden/parser-call-results.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

#[test]
fn empty_ref_names_fall_through_in_required_field_signatures() {
    let mut cases = 0;
    for row in call_results().filter(|row| row["program"] == "root = Box()") {
        let library = Library::from_json_schema(&row["schema"]);
        let result = parse(row["program"].as_str().unwrap(), &library, None);
        assert_eq!(
            result.to_json(),
            row["expected"].as_str().unwrap(),
            "{}",
            row["name"]
        );
        cases += 1;
    }
    assert_eq!(cases, 4);
}

#[test]
fn nested_action_keeps_its_mapped_props_and_all_errors_under_the_result_floor() {
    let row = call_results()
        .find(|row| row["name"] == "nested-action")
        .unwrap();
    let library = std::sync::Arc::new(Library::from_json_schema(&row["schema"]));
    let program = row["program"].as_str().unwrap();
    let expected = row["expected"].as_str().unwrap();
    assert_eq!((program.len(), expected.len()), (287, 10_697));
    let result = parse(program, &library, None);
    assert_eq!(result.meta.errors.len(), 50);
    assert!(!result.size_limited && !result.depth_limited);
    assert_eq!(result.to_json(), expected);
    let mut stream = openui_lang::StreamParser::new(library.clone(), None);
    for chunk in program.as_bytes().chunks(11) {
        stream.push(std::str::from_utf8(chunk).unwrap());
    }
    let finished = stream.finish();
    assert!(!finished.size_limited && !finished.depth_limited);
    assert_eq!(finished.to_json(), expected);

    let cut = openui_lang::parse_with_limits(
        program,
        &library,
        None,
        openui_lang::Limits::DEFAULT.with_result_size(4096),
    );
    assert!(cut.size_limited && !cut.depth_limited);
}

#[test]
fn an_invalid_action_does_not_materialize_its_unused_parameter_default() {
    let lib = Library::from_json_schema(&json!({
        "$defs": {"Box": {"properties": {"v": {}}}},
        "actions": {"Send": {"params": {"properties": {
            "a": {"type": "string", "default": "x".repeat(65536)},
            "b": {"type": "string"}
        }}}}
    }));
    let result = parse("root = Box(@Send(1, 2))", &lib, None);
    assert_eq!(result.meta.errors.len(), 2);
    assert!(!result.size_limited);
}

#[test]
fn cached_invalid_action_requires_room_for_its_temporary_arguments() {
    use openui_lang::{Limits, StreamParser};
    use std::sync::Arc;

    let lib = Arc::new(Library::from_json_schema(&json!({
        "$defs": {"Box": {"properties": {"v": {}}}},
        "actions": {"Send": {"params": {"properties": {"v": {"type": "string"}}}}}
    })));
    let limits = Limits::DEFAULT.with_result_size(3400);
    let values = vec!["0"; 64].join(",");
    let base = format!("a = @Send([{values}])\nb = a\nroot = Box(b)\n");
    let edit = format!("root = Box([\"{}\", b])\n", "x".repeat(1000));
    let mut warm = StreamParser::with_limits(lib.clone(), None, limits);
    assert!(!warm.push(&base).size_limited);
    let mut fresh = StreamParser::with_limits(lib, None, limits);
    let expected = fresh.push(&(base + &edit));
    assert!(expected.size_limited);
    let replay = warm.push(&edit);
    assert_eq!(replay.size_limited, expected.size_limited);
    assert_eq!(replay.to_json(), expected.to_json());
}
