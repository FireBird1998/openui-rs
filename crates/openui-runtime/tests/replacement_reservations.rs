//! Replacement reservations survive nesting, re-entry, unwind and cuts.
use openui_lang::{
    Ast, Element, EvaluationScope, EvaluationSession, ExpressionEvaluator, ExpressionValue,
    JsObject, Limits, ParseResult, Value,
};
use std::{
    cell::Cell,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
fn element(width: usize) -> Value {
    let mut props = JsObject::new();
    props.define("a", Value::Str("x".repeat(width)));
    Value::Element(Arc::new(Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    }))
}
fn bytes(value: &Value) -> usize {
    let mut props = JsObject::new();
    props.define("v", value.clone());
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
struct Scope {
    mode: usize,
    calls: Cell<usize>,
    nested: Cell<bool>,
}
impl EvaluationScope for Scope {
    fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        Some(element(10))
    }
    fn wants_each_item(&self, v: &ExpressionValue<'_>, _: bool) -> bool {
        v.property("type").as_str() == Some("element")
    }
    fn call(&self, call: &Ast, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        if matches!(call,Ast::Comp{name,..} if name=="Panic") {
            eval.reserve_result(17);
            panic!("caught embedder panic");
        }
        eval.evaluate(&Ast::Num(7.))
    }
    fn each_item(&self, _: Option<&Value>, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        let n = self.calls.get();
        self.calls.set(n + 1);
        if n == 1 && !self.nested.replace(true) {
            match self.mode {
                0 => {
                    let ast =
                        openui_lang::parse_expression(&openui_lang::tokenize("@Each([1,2],y,$el)"));
                    eval.evaluate(&ast);
                }
                1 => {
                    let call = openui_lang::parse_expression(&openui_lang::tokenize("@Reenter()"));
                    eval.evaluate(&call);
                }
                2 => {
                    let call = openui_lang::parse_expression(&openui_lang::tokenize("@Panic()"));
                    let caught = catch_unwind(AssertUnwindSafe(|| eval.evaluate(&call)));
                    assert!(caught.is_err());
                }
                _ => {
                    eval.reserve_result(1_000_000);
                }
            }
        }
        // The callback has deliberately reserved none of this result itself.
        Some(element(if n.is_multiple_of(2) { 1000 } else { 1 }))
    }
}
#[test]
fn retained_bytes_never_exceed_charged_room_after_adversarial_replacements() {
    for mode in 0..4 {
        for room in [1024, 4096, 16_384] {
            let scope = Scope {
                mode,
                calls: Cell::new(0),
                nested: Cell::new(false),
            };
            let session = EvaluationSession::with_limits(
                &scope,
                1024,
                Limits::DEFAULT.with_work(10_000_000).with_result_size(room),
            );
            let ast =
                openui_lang::parse_expression(&openui_lang::tokenize("@Each([1,2,3,4,5,6],x,$el)"));
            let result = session.evaluate(&ast);
            let retained = result.value.as_ref().map_or(0, bytes);
            let mut unused = 0;
            session.with_evaluator(|eval| {
                while eval.reserve_result(1) {
                    unused += 1;
                }
            });
            let charged = room - unused;
            assert!(
                retained <= charged,
                "mode={mode} room={room} retained={retained} charged={charged}"
            );
        }
    }
}
#[test]
fn retained_intermediate_supplied_values_keep_their_output_reservation() {
    let scope = Scope {
        mode: 1,
        calls: Cell::new(0),
        nested: Cell::new(false),
    };
    let session = EvaluationSession::with_limits(
        &scope,
        1024,
        Limits::DEFAULT.with_work(10_000).with_result_size(1000),
    );
    let result = session
        .with_evaluator(|eval| {
            eval.with_intermediate_results(|| {
                (eval.evaluate_result(&Ast::StateRef("$el".into())), false)
            })
        })
        .unwrap();
    let retained = bytes(result.value.as_ref().unwrap());
    let mut unused = 0;
    session.with_evaluator(|eval| {
        while eval.reserve_result(1) {
            unused += 1;
        }
    });
    assert!(retained <= 1000 - unused);
}
#[test]
fn measured_callback_bodies_recheck_depth_without_a_name_cache() {
    struct Memo {
        body: Value,
        prime: bool,
    }
    impl Memo {
        fn run(&self, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
            fn nested(eval: &ExpressionEvaluator<'_>, body: &Value, levels: usize) {
                if levels == 0 {
                    eval.reserve_value(body);
                } else {
                    eval.with_level(|| nested(eval, body, levels - 1));
                }
            }
            let owner = self.body.clone();
            if self.prime {
                eval.reserve_value(&owner);
            }
            nested(eval, &owner, 20);
            Some(Value::Null)
        }
    }
    impl EvaluationScope for Memo {
        fn call(&self, _: &Ast, e: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.run(e)
        }
        fn wants_unmapped_call(&self, _: &Value, name: &str) -> bool {
            name == "Host"
        }
        fn call_value(&self, _: &Value, e: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.run(e)
        }
    }
    let mut body = Value::Null;
    for _ in 0..40 {
        body = Value::Arr(Arc::new(vec![body]));
    }
    for prime in [false, true] {
        for data in [false, true] {
            let scope = Memo {
                body: body.clone(),
                prime,
            };
            let session =
                EvaluationSession::with_limits(&scope, 1024, Limits::DEFAULT.with_depth(32));
            if data {
                let mut node = JsObject::new();
                node.define("k", Value::Str("Comp".into()));
                node.define("name", Value::Str("Host".into()));
                node.define("args", Value::Arr(Arc::new(vec![])));
                session.with_evaluator(|eval| eval.evaluate_value(&Value::Obj(Arc::new(node))));
            } else {
                session.evaluate(&openui_lang::parse_expression(&openui_lang::tokenize(
                    "@Host()",
                )));
            }
            assert!(session.flags().depth_limited, "prime={prime} data={data}");
        }
    }
}
