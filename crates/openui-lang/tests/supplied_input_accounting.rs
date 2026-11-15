//! Supplied inputs enlarge an allowance without refreshing work already spent.
use openui_lang::{EvaluationSession, Limits, ParseResult, PreparedInput, Value};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn program(bytes: usize) -> PreparedInput {
    let mut p = ParseResult::default();
    p.state_declarations
        .define("$text", Value::Str("x".repeat(bytes)));
    PreparedInput::from_parse_result(&p, Limits::DEFAULT)
}
#[test]
fn a_replacement_program_earns_only_the_increase_in_supplied_input() {
    let small = program(2000);
    let large = program(3000);
    let session = EvaluationSession::new(&(), 0);
    session.with_evaluator(|e| {
        assert!(e.register_input_floor(&small));
        assert!(e.charge_work(100_000));
        assert!(e.register_input_floor(&large));
        assert!(e.charge_work(2_500_000));
    });
    let session = EvaluationSession::new(&(), 0);
    session.with_evaluator(|e| {
        assert!(e.register_input_floor(&small));
        assert!(e.register_input_floor(&large));
        assert!(
            !e.charge_work(4_000_000),
            "replacement inputs must not add their sizes together"
        );
    });
}
#[test]
fn repeated_prepared_input_cannot_replenish_spent_work() {
    let p = program(2000);
    let session = EvaluationSession::new(&(), 0);
    session.with_evaluator(|e| {
        for _ in 0..100 {
            assert!(e.register_input_floor(&p.clone()));
        }
        assert!(!e.charge_work(3_000_000));
    });
    let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(40_000));
    session.with_evaluator(|e| {
        assert!(e.register_input_floor(&p));
        assert!(e.charge_work(30_000));
        for _ in 0..10 {
            assert!(e.register_input_floor(&p));
        }
        assert!(!e.charge_work(15_000), "fixed work limits must not refresh");
    });
}
#[test]
fn individual_utf16_units_round_trip_including_lone_surrogates() {
    for unit in 0..=u16::MAX {
        assert_eq!(
            openui_lang::utf16_units(&openui_lang::utf16_unit_string(unit)).collect::<Vec<_>>(),
            vec![unit]
        );
    }
}

#[test]
fn supplying_large_input_never_spends_evaluation_work() {
    let p = program(100_000);
    let session = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(7));
    session.with_evaluator(|e| {
        for _ in 0..1000 {
            assert!(e.register_input_floor(&p));
        }
        assert!(e.charge_work(7));
        assert!(!e.charge_work(1));
    });
}

#[test]
fn a_combined_supply_raises_one_floor_without_retaining_its_values() {
    use std::sync::Arc;
    let values = Arc::new(vec![Value::Str("x".repeat(2000))]);
    let input = PreparedInput::from_value(&Value::Arr(values.clone()), Limits::DEFAULT);
    assert_eq!(Arc::strong_count(&values), 1);
    let combined = input.combined(PreparedInput::from_text("message", Limits::DEFAULT));
    let session = EvaluationSession::new(&(), 0);
    session.with_evaluator(|e| {
        assert!(e.register_input_floor(&combined));
        assert_eq!(Arc::strong_count(&values), 1);
        assert!(e.charge_work(1_500_000));
        assert!(e.register_input_floor(&input));
        assert!(!e.charge_work(1_000_000));
    });
}
