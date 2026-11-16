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
