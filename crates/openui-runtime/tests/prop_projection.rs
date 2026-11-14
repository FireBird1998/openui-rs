//! Pre-projection replacement preserves output accounting and markerless work.
use openui_lang::{Library, free_json, parse, parse_json};
use openui_runtime::{Limits, Store, TreeInput, evaluate_tree_with_limits};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);

#[test]
fn string_props_keep_the_work_needed_on_main() {
    let schema = parse_json(r#"{"$defs":{"Box":{"properties":{"children":{}}},"Field":{"properties":{"value":{},"display":{}}}}}"#).unwrap();
    let library = Library::from_json_schema(&schema);
    free_json(schema);
    for (count, length, expected) in [(11, 60_000, 781_431), (1, 400_000, 1_200_241)] {
        let root = format!("Box([{}])", vec!["Field(null,$a)"; count].join(","));
        let parsed = parse(
            &format!("root={root}\n$a=\"{}\"", "x".repeat(length)),
            &library,
            None,
        );
        let mut store = Store::new();
        let _ = store.initialize(
            parsed
                .state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            std::iter::empty(),
        );
        let run = |work| {
            evaluate_tree_with_limits(
                TreeInput::new(&parsed).with_library(&library),
                &store,
                Limits::DEFAULT.with_work(work),
            )
        };
        assert!(!run(expected).size_limited, "{count} strings at {expected}");
        assert!(
            run(expected - 1).size_limited,
            "exact work for {count} strings"
        );
        if count == 11 {
            assert!(!run(1_000_000).size_limited);
        }
    }
}

use openui_runtime::{
    Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, ExpressionValue, Value,
};
use std::sync::Arc;
fn value(text: &str) -> Value {
    let json = parse_json(text).unwrap();
    let result = Value::from_json(&json);
    free_json(json);
    result
}
fn request(name: &str) -> Ast {
    Ast::Obj(vec![("readState".into(), Ast::Str(name.into()))].into())
}
struct Select;
impl EvaluationScope for Select {
    fn state(&self, name: &str, eval: &ExpressionEvaluator<'_>) -> Option<Value> {
        match name {
            "$value" => Some(Value::Num(7.)),
            "$computed" => {
                eval.with_projection(|e| e.evaluate_result(&request("$value")))
                    .0
                    .value
            }
            "$dataComputed" => {
                eval.with_projection(|e| {
                    e.evaluate_value(&value(
                        r#"{"k":"Obj","entries":[["readState",{"k":"Str","v":"$value"}]]}"#,
                    ))
                })
                .0
                .value
            }
            "$nested" => Some(value(r#"{"readState":"$wrong","payload":123}"#)),
            "$big" => Some(Value::Str("x".repeat(20_000))),
            "$deep" => {
                let mut v = Value::Null;
                for _ in 0..1000 {
                    v = Value::Arr(Arc::new(vec![v]));
                }
                Some(v)
            }
            _ => None,
        }
    }
    fn extra(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        (name == "$value").then_some(Value::Num(88.))
    }
    fn extra_is_object(&self) -> bool {
        true
    }
    fn project_result<'v>(
        &self,
        v: ExpressionValue<'v>,
        state: &dyn Fn(&str) -> ExpressionValue<'v>,
    ) -> Option<ExpressionValue<'v>> {
        if !v.is_object() {
            return None;
        }
        let name = v.property("readState");
        if let Some(name) = name.as_str() {
            let v = state(name);
            return Some(if v.is_nullish() {
                ExpressionValue::null()
            } else {
                v
            });
        }
        let selected = v.property("select");
        selected
            .as_bool()
            .filter(|v| *v)
            .map(|_| v.property("payload"))
    }
}
#[test]
fn projection_hooks_keep_replacements_and_read_root_state_without_extra() {
    let session = EvaluationSession::new(&Select, 1000);
    session.with_evaluator(|eval| {
        for name in ["$computed","$dataComputed"] {
            assert_eq!(eval.evaluate(&Ast::StateRef(name.into())),Some(Value::Num(7.)));
        }
        let (read,replaced)=eval.with_projection(|e|e.evaluate_result(&request("$value")));
        assert!(replaced);assert_eq!(read.value,Some(Value::Num(7.)));
        let (read,replaced)=eval.with_projection(|e|e.evaluate_result(&request("$nested")));
        assert!(replaced);assert_eq!(read.value,Some(value(r#"{"readState":"$wrong","payload":123}"#)));
        let descriptor=value(r#"{"k":"Obj","entries":[["select",{"k":"Bool","v":true}],["payload",{"k":"Num","v":42}]]}"#);
        let (read,replaced)=eval.with_projection(|e|e.evaluate_value(&descriptor));
        assert!(replaced);assert_eq!(read.value,Some(Value::Num(42.)));
        let (read,replaced)=eval.with_projection(|e|e.evaluate_call_prop(&Value::Null));
        assert!(!replaced);assert!(read.failure.is_some());
        // Nested callbacks explicitly opt in; ordinary evaluation stays ordinary.
        assert_eq!(eval.evaluate(&request("$value")),Some(value(r#"{"readState":"$value"}"#)));
        let _=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| eval.with_projection(|_|panic!("scope"))));
        assert_eq!(eval.evaluate(&request("$value")),Some(value(r#"{"readState":"$value"}"#)));
    });
}
#[test]
fn unmodified_projection_hooks_keep_exact_room_and_work() {
    struct Identity;
    impl EvaluationScope for Identity {
        fn project_result<'v>(
            &self,
            value: ExpressionValue<'v>,
            _: &dyn Fn(&str) -> ExpressionValue<'v>,
        ) -> Option<ExpressionValue<'v>> {
            Some(value)
        }
    }
    for ast in [
        Ast::Str("x".repeat(20_000)),
        Ast::Num(7.),
        Ast::Null,
        Ast::Obj(vec![("field".into(), Ast::Num(2.))].into()),
    ] {
        let minimum = |scope: &dyn EvaluationScope, project, kind| {
            let mut low = 0;
            let mut high = if kind == 2 { 64 } else { 100_000 };
            while low < high {
                let mid = low + (high - low) / 2;
                let limits = match kind {
                    0 => Limits::DEFAULT.with_result_size(mid),
                    1 => Limits::DEFAULT.with_work(mid),
                    _ => Limits::DEFAULT.with_depth(mid),
                };
                let s = EvaluationSession::with_limits(scope, 1000, limits);
                let result = s.with_evaluator(|e| {
                    if project {
                        e.with_projection(|e| e.evaluate_result(&ast)).0
                    } else {
                        e.evaluate_result(&ast)
                    }
                });
                if result.size_limited || result.depth_limited {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            low
        };
        for kind in 0..3 {
            assert_eq!(minimum(&Select, false, kind), minimum(&Select, true, kind));
            assert_eq!(
                minimum(&Select, false, kind),
                minimum(&Identity, true, kind)
            );
        }
    }
}
#[test]
fn hook_replacements_cannot_skip_output_work_room_or_depth() {
    for limits in [
        Limits::DEFAULT.with_result_size(32),
        Limits::DEFAULT.with_work(10),
    ] {
        let s = EvaluationSession::with_limits(&Select, 1000, limits);
        let result =
            s.with_evaluator(|e| e.with_projection(|e| e.evaluate_result(&request("$big"))).0);
        assert!(result.size_limited);
        assert!(
            result
                .value
                .as_ref()
                .is_none_or(|v| matches!(v, Value::Null))
        );
    }
    let s = EvaluationSession::with_limits(&Select, 1000, Limits::DEFAULT.with_depth(16));
    let result = s.with_evaluator(|e| {
        e.with_projection(|e| e.evaluate_result(&request("$deep")))
            .0
    });
    assert!(result.depth_limited);
    struct Count(std::cell::Cell<usize>);
    impl EvaluationScope for Count {
        fn project_result<'v>(
            &self,
            _: ExpressionValue<'v>,
            _: &dyn Fn(&str) -> ExpressionValue<'v>,
        ) -> Option<ExpressionValue<'v>> {
            self.0.set(self.0.get() + 1);
            None
        }
    }
    let mut deep_ast = Ast::Null;
    for _ in 0..64 {
        deep_ast = Ast::Arr(vec![deep_ast].into());
    }
    for limits in [Limits::DEFAULT.with_work(0), Limits::DEFAULT.with_depth(8)] {
        let scope = Count(std::cell::Cell::new(0));
        let s = EvaluationSession::with_limits(&scope, 0, limits);
        let result = s.with_evaluator(|e| e.with_projection(|e| e.evaluate_result(&deep_ast)).0);
        assert!(result.size_limited || result.depth_limited);
        assert_eq!(scope.0.get(), 0, "a cut must not invoke the hook");
    }
    struct Stop(std::cell::Cell<usize>);
    impl EvaluationScope for Stop {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            self.0.set(self.0.get() + 1);
            let mut object = openui_runtime::JsObject::new();
            object.define("payload", Value::Str("x".repeat(20_000)));
            Some(Value::Obj(Arc::new(object)))
        }
        fn project_result<'v>(
            &self,
            _: ExpressionValue<'v>,
            state: &dyn Fn(&str) -> ExpressionValue<'v>,
        ) -> Option<ExpressionValue<'v>> {
            let v = state("$first");
            assert!(
                v.property("payload").is_nullish(),
                "inspection stops after the lookup cuts"
            );
            let _ = state("$later");
            Some(ExpressionValue::null())
        }
    }
    let scope = Stop(std::cell::Cell::new(0));
    let s = EvaluationSession::with_limits(&scope, 0, Limits::DEFAULT.with_work(1000));
    let result = s.with_evaluator(|e| e.with_projection(|e| e.evaluate_result(&Ast::Num(0.))).0);
    assert!(result.size_limited);
    assert_eq!(scope.0.get(), 1);
    struct Null;
    impl EvaluationScope for Null {
        fn project_result<'v>(
            &self,
            _: ExpressionValue<'v>,
            _: &dyn Fn(&str) -> ExpressionValue<'v>,
        ) -> Option<ExpressionValue<'v>> {
            Some(ExpressionValue::null())
        }
    }
    for work in [4, 5] {
        let s = EvaluationSession::with_limits(&Null, 0, Limits::DEFAULT.with_work(work));
        let result =
            s.with_evaluator(|e| e.with_projection(|e| e.evaluate_result(&Ast::Num(0.))).0);
        assert_eq!(result.size_limited, work == 4);
    }
    let mut deep = Value::Null;
    for _ in 0..1000 {
        deep = Value::Arr(Arc::new(vec![deep]));
    }
    let mut payload = openui_runtime::JsObject::new();
    payload.define("select", Value::Bool(true));
    payload.define("payload", deep);
    let mut descriptor = openui_runtime::JsObject::new();
    descriptor.define("k", Value::Str("Str".into()));
    descriptor.define("v", Value::Obj(Arc::new(payload)));
    let descriptor = Value::Obj(Arc::new(descriptor));
    let s = EvaluationSession::with_limits(&Select, 0, Limits::DEFAULT.with_depth(16));
    let (result, replaced) =
        s.with_evaluator(|e| e.with_projection(|e| e.evaluate_value(&descriptor)));
    assert!(replaced);
    assert!(result.depth_limited);
    for room in [3, 4] {
        let s = EvaluationSession::with_limits(&Null, 0, Limits::DEFAULT.with_result_size(room));
        let result =
            s.with_evaluator(|e| e.with_projection(|e| e.evaluate_result(&Ast::Num(0.))).0);
        assert_eq!(result.size_limited, room == 3);
    }
}

#[test]
fn projection_inspection_stays_with_its_owner_inside_foreign_sessions() {
    struct Foreign {
        lookup: bool,
        chained: bool,
    }
    impl EvaluationScope for Foreign {
        fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
            Some(Value::Str("x".repeat(20_000)))
        }
        fn project_result<'v>(
            &self,
            v: ExpressionValue<'v>,
            state: &dyn Fn(&str) -> ExpressionValue<'v>,
        ) -> Option<ExpressionValue<'v>> {
            let foreign =
                EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(100_000));
            Some(foreign.with_evaluator(|_| {
                if self.lookup {
                    state("$big").property("length")
                } else if self.chained {
                    v.property("payload").property("length")
                } else {
                    v.property("length")
                }
            }))
        }
    }
    for (lookup, chained) in [(false, false), (true, false), (false, true)] {
        let scope = Foreign { lookup, chained };
        let owner = EvaluationSession::with_limits(&scope, 0, Limits::DEFAULT.with_work(1000));
        let ast = if lookup {
            Ast::Num(0.)
        } else if chained {
            Ast::Obj(vec![("payload".into(), Ast::Str("x".repeat(20_000)))].into())
        } else {
            Ast::Str("x".repeat(20_000))
        };
        let result = owner.with_evaluator(|e| e.with_projection(|e| e.evaluate_result(&ast)).0);
        assert!(
            result.size_limited,
            "foreign session must not pay for owner lookup={lookup}, chained={chained}"
        );
        assert!(owner.flags().size_limited);
    }
}
