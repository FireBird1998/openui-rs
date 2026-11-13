//! Nested public entries receive their full stack allowance.
use openui_lang::{Library, StreamParser, parse, parse_expression, parse_json, tokenize};
use openui_runtime::{Ast, EvaluationScope, EvaluationSession, ExpressionEvaluator, Limits, Value};
use std::cell::Cell;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(128 * 1024 * 1024);
struct Nested<'a, const HELD: usize> {
    session: Cell<Option<&'a EvaluationSession<'a>>>,
    library: Library,
    program: String,
    expected: String,
    json: String,
    links: usize,
    checked: Cell<usize>,
}
impl<const HELD: usize> EvaluationScope for Nested<'_, HELD> {
    #[inline(never)]
    fn reference(&self, name: &str, _: &ExpressionEvaluator<'_>) -> Option<Value> {
        let held = [0u8; HELD];
        std::hint::black_box(&held);
        let n: usize = name.parse().ok()?;
        if n >= self.links * 2 / 3 {
            let parsed = parse(&self.program, &self.library, None);
            assert!(!parsed.size_limited && !parsed.depth_limited);
            assert_eq!(parsed.to_json(), self.expected);
            let json = parse_json(&self.json).unwrap();
            assert!(json.is_array());
            openui_lang::free_json(json);
            let expression = parse_expression(&tokenize(&self.json));
            assert!(!matches!(expression, Ast::Null));
            let mut stream = StreamParser::new(self.library.clone().into(), None);
            assert_eq!(stream.push(&self.program).to_json(), self.expected);
            self.checked.set(self.checked.get() + 1);
        }
        if n == self.links {
            return Some(Value::Num(1.0));
        }
        let value = self
            .session
            .get()?
            .evaluate(&Ast::Ref((n + 1).to_string()))
            .value;
        std::hint::black_box(&held);
        value
    }
}
fn check_nested<const HELD: usize>(links: usize) {
    let library = Library::from_json_schema(
        &parse_json(r#"{"$defs":{"Text":{"properties":{"value":{}}}}}"#).unwrap(),
    );
    let json = format!("{}1{}", "[".repeat(120), "]".repeat(120));
    let program = format!("root = Text({json})\n$x = @Sum([1,2])");
    let expected = parse(&program, &library, None);
    assert!(!expected.size_limited && !expected.depth_limited);
    let scope = Nested::<HELD> {
        session: Cell::new(None),
        library,
        program,
        expected: expected.to_json(),
        json,
        links,
        checked: Cell::new(0),
    };
    let session =
        EvaluationSession::with_limits(&scope, 200_000, Limits::DEFAULT.with_depth(20_000));
    scope.session.set(Some(&session));
    let result = session.evaluate(&Ast::Ref("0".into()));
    assert!(!result.size_limited && !result.depth_limited);
    assert_eq!(result.value, Some(Value::Num(1.0)));
    assert!(scope.checked.get() > links / 3);
}

#[test]
fn deep_session_reentry_does_not_spend_the_nested_parsers_stack_allowance() {
    check_nested::<0>(16_000);
    // Keep enough caller stack live to exercise the same boundary even when
    // release optimization makes the evaluator's own frames much smaller.
    check_nested::<{ 64 * 1024 }>(600);
}
