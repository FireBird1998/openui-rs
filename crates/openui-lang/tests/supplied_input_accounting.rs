//! Supplied inputs enlarge an allowance without refreshing work already spent.
use openui_lang::{EvaluationSession, Limits, ParseResult, PreparedProgram, Value};
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(256 * 1024 * 1024);
fn program(bytes: usize) -> PreparedProgram {
    PreparedProgram::new(&program_value(bytes), Limits::DEFAULT)
}
fn program_value(bytes: usize) -> ParseResult {
    let mut p = ParseResult::default();
    p.state_declarations
        .define("$text", Value::Str("x".repeat(bytes)));
    p
}
#[test]
fn a_replacement_program_earns_only_the_increase_in_supplied_input() {
    let small = program(2000);
    let large = program(3000);
    let session = EvaluationSession::new(&(), 0);
    session.with_evaluator(|e| {
        assert!(supply(e, &small));
        assert!(e.charge_work(100_000));
        assert!(supply(e, &large));
        assert!(e.charge_work(2_500_000));
    });
    let session = EvaluationSession::new(&(), 0);
    session.with_evaluator(|e| {
        assert!(supply(e, &small));
        assert!(supply(e, &large));
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
            assert!(supply(e, &p));
        }
        assert!(!e.charge_work(3_000_000));
    });
    let limits = Limits::DEFAULT.with_work(40_000);
    let p = PreparedProgram::new(&program_value(2000), limits);
    let session = EvaluationSession::with_limits(&(), 0, limits);
    session.with_evaluator(|e| {
        assert!(supply(e, &p));
        assert!(e.charge_work(30_000));
        for _ in 0..10 {
            assert!(supply(e, &p));
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
    let limits = Limits::DEFAULT.with_work(7);
    let p = PreparedProgram::new(&program_value(100_000), limits);
    let session = EvaluationSession::with_limits(&(), 0, limits);
    session.with_evaluator(|e| {
        for _ in 0..1000 {
            assert!(supply(e, &p));
        }
        assert!(e.charge_work(7));
        assert!(!e.charge_work(1));
    });
}

fn supply(e: &openui_lang::ExpressionEvaluator<'_>, p: &PreparedProgram) -> bool {
    e.supply_input(openui_lang::SuppliedInput {
        program: openui_lang::SuppliedProgram::Prepared(p),
        start: openui_lang::SuppliedStart::Raw {
            action: None,
            message: "",
            form: None,
        },
        references: None,
    })
    .is_ok()
}
#[test]
fn receipts_from_different_limits_are_refused_without_changing_credit() {
    let p = program(3000);
    let s = EvaluationSession::with_limits(&(), 0, Limits::DEFAULT.with_work(7));
    s.with_evaluator(|e| {
        assert!(!supply(e, &p));
        assert!(e.charge_work(7));
    });
}
