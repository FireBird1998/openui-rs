//! Exact public work and result-size boundaries for evaluator pricing.
use openui_lang::{Library, Limits, ParseResult, parse_with_limits};

fn under(program: &str, work: usize, room: usize) -> ParseResult {
    parse_with_limits(
        program,
        &Library::from_json_schema(&serde_json::json!({})),
        None,
        Limits::default().with_work(work).with_result_size(room),
    )
}

fn threshold(program: &str, work: bool) -> usize {
    let (mut lo, mut hi) = (0, 2_000_000);
    assert!(!under(program, hi, hi).size_limited);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        let result = if work {
            under(program, mid, 2_000_000)
        } else {
            under(program, 2_000_000, mid)
        };
        if result.size_limited {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// A boundary must both preserve the whole result at the exact price and
/// cut one step/byte below it. No evaluator counters are inspected.
fn edges(program: &str, work: usize, room: Option<usize>) {
    let whole = under(program, 2_000_000, 2_000_000);
    assert!(!whole.size_limited);
    assert_eq!(threshold(program, true), work, "work: {program}");
    assert_eq!(under(program, work, 2_000_000), whole);
    let cut = under(program, work - 1, 2_000_000);
    assert!(cut.size_limited);
    assert!(cut.to_json().contains("\"$a\":null"));
    if let Some(room) = room {
        assert_eq!(threshold(program, false), room, "room: {program}");
        assert_eq!(under(program, 2_000_000, room), whole);
        let cut = under(program, 2_000_000, room - 1);
        assert!(cut.size_limited);
        assert!(cut.to_json().contains("\"$a\":null"));
    }
}

#[test]
fn number_prices_include_digits_sign_and_the_large_or_fractional_fallback() {
    // Three evaluated expression nodes, then number_cost: digits plus a
    // negative sign below 1e15; otherwise 24. Zero has one digit, no sign.
    for (number, price) in [
        ("0", 1),
        ("1", 1),
        ("12", 2),
        ("-12", 3),
        ("0.5", 24),
        ("999999999999999", 16),
        ("1000000000000000", 24),
        ("-1000000000000000", 24),
    ] {
        edges(&format!("$a = {number} + 0"), 3 + price, None);
    }
}

#[test]
fn json_prices_include_string_bytes_container_items_and_object_keys() {
    // A conditional and its chosen value are three nodes before the own
    // JSON charge and each child's charge. Nonempty shapes differ from
    // the fallback cost (4), and empty shapes pin the container's base 2.
    for (expression, work) in [
        ("true ? [] : null", 5),
        ("true ? [1, 22, 333] : null", 17),
        ("true ? {} : null", 5),
        ("true ? {abc: 1, z: 22} : null", 22),
        ("\"\" + \"\"", 5),
        ("\"abc\" + \"\"", 11),
    ] {
        edges(&format!("$a = {expression}"), work, None);
    }
}

#[test]
fn amplified_values_pin_work_and_room_independently() {
    // A 100-item value, copied into each of 100 results, makes evaluation's
    // room price exceed the initial parsed value's price. Work also counts
    // reading nodes and string bytes; room only counts what is built.
    // These are Rust limit expectations, not reference result semantics.
    for (value, work, room) in [
        ("0", 20_805, 22_805),
        ("12", 30_805, 32_805),
        ("-12", 40_805, 42_805),
        ("0.5", 250_805, 252_805),
        ("1000000000000000", 250_805, 252_805),
        ("\"abc\"", 61_105, 60_905),
        ("[]", 30_805, 30_605),
        ("[1,22,333]", 121_105, 128_105),
        ("{}", 30_805, 30_605),
        ("{abc: 1, z: 22}", 181_405, 186_805),
    ] {
        let values = vec![value; 100].join(",");
        let items = vec!["0"; 100].join(",");
        let program = format!("$v = [{values}]\n$a = @Each([{items}], \"x\", $v)");
        edges(&program, work, Some(room));
    }
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);
