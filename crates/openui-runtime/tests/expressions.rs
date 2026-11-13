//! Public expression evaluation: pinned upstream outputs and runtime extension
//! points. The store, tree evaluation and action semantics belong to later tickets.

use std::cell::Cell;
use std::sync::Arc;

use openui_lang::{JsObject, Library, free_json, parse, parse_expression, parse_json, tokenize};
use openui_runtime::{
    Ast, EvaluationResult, EvaluationScope, ExpressionEvaluator, Value, evaluate_expression,
};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);

fn ast(source: &str) -> Ast {
    parse_expression(&tokenize(source))
}

fn value(json: &str) -> Value {
    let json = parse_json(json).unwrap();
    let result = Value::from_json(&json);
    free_json(json);
    result
}

fn property(value: &Value, name: &str) -> Option<Value> {
    if let Value::Obj(o) = value {
        o.get(name).cloned()
    } else {
        None
    }
}

struct Scope {
    state: Value,
    references: Value,
    statements: Value,
    extra: Value,
}
impl EvaluationScope for Scope {
    fn state(&self, name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        property(&self.state, name)
    }
    fn reference(&self, name: &str, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        if let Some(Value::Str(source)) = &property(&self.statements, name) {
            evaluator.evaluate(&ast(source))
        } else {
            property(&self.references, name)
        }
    }
    fn extra(&self, name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        property(&self.extra, name)
    }
}

fn whole(result: EvaluationResult) -> Option<Value> {
    assert!(!result.size_limited && !result.depth_limited, "{result:?}");
    result.value
}

#[test]
fn matches_the_pinned_upstream_evaluator_with_supplied_names() {
    let cases = parse_json(include_str!("expressions.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let scope = Scope {
            state: Value::from_json(&case["state"]),
            references: Value::from_json(&case["references"]),
            statements: Value::from_json(&case["statements"]),
            extra: Value::from_json(&case["extra"]),
        };
        let mut expression = ast(case["expression"].as_str().unwrap());
        if let Some(name) = case["inlineRuntimeRef"].as_str() {
            let Ast::Comp { args, .. } = &mut expression else {
                panic!("Each");
            };
            Arc::make_mut(args)[2] = Ast::RuntimeRef {
                n: name.into(),
                ref_type: openui_lang::RefType::Query,
            };
        }
        let expected = case["defined"]
            .as_bool()
            .unwrap()
            .then(|| Value::from_json(&case["value"]));
        assert_eq!(
            whole(evaluate_expression(&expression, &scope)),
            expected,
            "{}",
            case["expression"]
        );
    }
    free_json(cases);
}

#[test]
fn identities_are_shared_across_names_and_nested_values() {
    let shared = value("{\"n\":1}");
    let mut state = JsObject::new();
    state.define("$left", shared.clone());
    state.define("$right", shared.clone());
    let scope = Scope {
        state: Value::Obj(Arc::new(state)),
        references: value("{}"),
        statements: Value::Null,
        extra: value("{}"),
    };
    assert_eq!(
        whole(evaluate_expression(&ast("$left == $right"), &scope)),
        Some(Value::Bool(true))
    );
}

// This is an embedder hook used to exercise the seam, not the runtime's action
// implementation. Set's real validation and plan shape are later work.
struct Calls {
    count: Cell<usize>,
}
impl EvaluationScope for Calls {
    fn state(&self, name: &str, _evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        (name == "$hp").then_some(Value::Num(5.0))
    }
    fn call(&self, call: &Ast, evaluator: &ExpressionEvaluator<'_>) -> Option<Value> {
        self.count.set(self.count.get() + 1);
        let Ast::Comp {
            name,
            args,
            mapped_props,
            ..
        } = call
        else {
            return None;
        };
        match name.as_str() {
            "Set" => args.get(1).map(|n| Value::Ast(Arc::new(n.clone()))),
            "Twice" => evaluator.evaluate(&args[0]).and_then(|v| {
                if let Value::Num(n) = v {
                    Some(Value::Num(n * 2.0))
                } else {
                    None
                }
            }),
            "Inspect" => {
                assert!(
                    mapped_props
                        .as_ref()
                        .is_some_and(|p| p.get("value").is_some())
                );
                evaluator.evaluate(&args[0])
            }
            _ => None,
        }
    }
}

#[test]
fn each_substitutes_before_a_call_can_defer_its_expression() {
    let calls = Calls {
        count: Cell::new(0),
    };
    let result = whole(evaluate_expression(
        &ast("@Each([{id: 2}, {id: 7}], \"row\", @Set($hp, $hp - row.id))"),
        &calls,
    ))
    .unwrap();
    let Value::Arr(items) = &result else {
        panic!("expected a list");
    };
    assert_eq!(items.len(), 2);
    for (expression, expected) in items.iter().zip([3.0, -2.0]) {
        let Value::Ast(expression) = expression else {
            panic!("expected a deferred expression");
        };
        assert_eq!(
            whole(evaluate_expression(expression, &calls)),
            Some(Value::Num(expected))
        );
    }
    assert_eq!(calls.count.get(), 2);
    assert_eq!(
        whole(evaluate_expression(&ast("@Twice(@Sum([1, 2]))"), &calls)),
        Some(Value::Num(6.0))
    );
    assert_eq!(
        whole(evaluate_expression(&ast("false && @Twice(1)"), &calls)),
        Some(Value::Bool(false))
    );
    assert_eq!(calls.count.get(), 3);
}

#[test]
fn parsed_named_props_reach_the_hook_untouched() {
    let json = parse_json(r#"{"$defs":{"Text":{"properties":{"value":{}},"required":["value"]}},"functions":{"Inspect":{"params":{"properties":{"value":{}},"required":["value"]}}}}"#).unwrap();
    let library = Library::from_json_schema(&json);
    free_json(json);
    let parsed = parse("root = Text(@Inspect($hp))", &library, None);
    let root = parsed.root.as_ref().unwrap();
    let Value::Ast(expression) = root.props.get("value").unwrap() else {
        panic!("expected an expression");
    };
    assert_eq!(
        whole(evaluate_expression(
            expression,
            &Calls {
                count: Cell::new(0)
            }
        )),
        Some(Value::Num(5.0))
    );
}

#[test]
fn binding_markers_keep_their_expression_for_a_later_write() {
    let marker = whole(evaluate_expression(&ast("$hp = $value * 2"), &())).unwrap();
    let Value::Ast(ref expression) = property(&marker, "expr").unwrap() else {
        panic!("expected an expression");
    };
    let scope = Scope {
        state: value("{}"),
        references: value("{}"),
        statements: Value::Null,
        extra: value(r#"{"$value":9}"#),
    };
    assert_eq!(
        whole(evaluate_expression(expression, &scope)),
        Some(Value::Num(18.0))
    );
    assert_eq!(
        expression.to_json(),
        r#"{"k":"BinOp","op":"*","left":{"k":"StateRef","n":"$value"},"right":{"k":"Num","v":2}}"#
    );
}

#[test]
fn unknown_calls_are_null_and_builtins_do_not_reach_the_hook() {
    let calls = Calls {
        count: Cell::new(0),
    };
    assert_eq!(
        whole(evaluate_expression(&ast("@Unknown(1)"), &())),
        Some(Value::Null)
    );
    assert_eq!(
        whole(evaluate_expression(&ast("@Sum([2, 3])"), &calls)),
        Some(Value::Num(5.0))
    );
    assert_eq!(calls.count.get(), 0);
}

#[test]
fn runtime_calls_can_return_shared_elements_and_plans_without_losing_identity() {
    struct Returned(Value);
    impl EvaluationScope for Returned {
        fn call(&self, _: &Ast, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(self.0.clone())
        }
    }
    let mut props = JsObject::new();
    props.define("value", Value::Ast(Arc::new(ast("$hp"))));
    let element = Arc::new(openui_lang::Element {
        type_name: "Text".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    });
    let result = whole(evaluate_expression(
        &ast("@Element()"),
        &Returned(Value::Element(element.clone())),
    ))
    .unwrap();
    let Value::Element(got) = &result else {
        panic!("expected a typed element");
    };
    assert!(Arc::ptr_eq(got, &element));
    // The same property is needed for the runtime to keep an ownership mark
    // for a custom step in a side table, without exposing it as forgeable data.
    let plan = value(r#"{"steps":[{"type":"set","target":"$hp"}]}"#);
    let scope = Returned(plan.clone());
    let result = whole(evaluate_expression(&ast("@Plan()"), &scope)).unwrap();
    let (Value::Obj(got), Value::Obj(original)) = (&result, &plan) else {
        panic!("expected a plan");
    };
    assert!(Arc::ptr_eq(got, original));
}
