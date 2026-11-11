//! Exact public work and result-size boundaries for evaluator pricing.
use openui_lang::{Library, Limits, ParseResult, parse_with_limits};

/// `program` under these limits, after a root: without one the `no-root`
/// error, written after the defaults, would be what's cut first.
fn under(program: &str, work: usize, room: usize) -> ParseResult {
    parse_with_limits(
        &format!("root = Stack()\n{program}"),
        &Library::from_json_schema(&serde_json::json!({"$defs": {"Stack": {"properties": {}}}})),
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
    // Room also holds what each declaration writes around its value, its
    // name included (6 bytes each here).
    // These are Rust limit expectations, not reference result semantics.
    for (value, work, room) in [
        ("0", 20_805, 22_922),
        ("12", 30_805, 32_922),
        ("-12", 40_805, 42_922),
        ("0.5", 250_805, 252_922),
        ("1000000000000000", 250_805, 252_922),
        ("\"abc\"", 61_105, 61_022),
        ("[]", 30_805, 30_722),
        ("[1,22,333]", 121_105, 128_222),
        ("{}", 30_805, 30_722),
        ("{abc: 1, z: 22}", 181_405, 186_922),
    ] {
        let values = vec![value; 100].join(",");
        let items = vec!["0"; 100].join(",");
        let program = format!("$v = [{values}]\n$a = @Each([{items}], \"x\", $v)");
        edges(&program, work, Some(room));
    }
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

#[test]
fn runtime_expression_conversion_has_an_exact_work_boundary() {
    let program = "$a = @Each([1], \"i\", $t = [1, {a: 1}])";
    edges(program, 195, None);
}

#[test]
fn zero_work_is_cut_before_converting_a_runtime_expression_at_the_depth_limit() {
    // n16 is whole. Adding TWO levels at once puts its raw runtime expression
    // exactly at the depth limit, without an intervening default becoming null.
    let mut program = "$n0 = @Each([1], \"i\", $t = 1)[0].expr\n".to_owned();
    for i in 1..=16 {
        program.push_str(&format!("$n{i} = [$n{}]\n", i - 1));
    }
    program.push_str("$a = [[$n16]]\n");
    let parse = |work| {
        parse_with_limits(
            &program,
            &Library::from_json_schema(&serde_json::json!({})),
            None,
            Limits::default().with_depth(6).with_work(work),
        )
    };
    // At 932, whole_ast sees exactly zero work. Its early-return null would
    // hit depth before drawing work: ran_out_of_work must mark Work FIRST.
    for work in [931, 932, 933, 939] {
        let cut = parse(work);
        assert_eq!(
            (cut.size_limited, cut.depth_limited),
            (true, false),
            "work {work}"
        );
        assert!(cut.to_json().contains("\"$a\":null"));
    }
    let depth = parse(940);
    assert_eq!((depth.size_limited, depth.depth_limited), (false, true));
    assert_eq!(depth, parse(2_000_000));
}

#[test]
fn plucking_list_properties_charges_one_step_per_item() {
    let program = "$xs = [{v: 1}, {v: 22}, {v: 333}]\n$a = $xs.v";
    edges(program, 26, None);
    assert!(
        under(program, 26, 2_000_000)
            .to_json()
            .contains("\"$a\":[1,22,333]")
    );
}

#[test]
fn text_operations_charge_both_operands_and_nested_arrays() {
    for (program, work, room) in [
        ("$a = @Sort([\"ab\", \"cde\"], \"\")", 26, 357),
        (
            "$a = @Filter([{v: \"abc\"}, {v: \"defg\"}], \"v\", \"contains\", \"ab\")",
            37,
            626,
        ),
        ("$a = \"\" + [[1, 22], [333, null], []]", 53, 397),
    ] {
        edges(program, work, None);
        assert_eq!(threshold(program, false), room);
        assert_eq!(
            under(program, 2_000_000, room),
            under(program, 2_000_000, 2_000_000)
        );
        assert!(under(program, 2_000_000, room - 1).size_limited);
    }
}

#[test]
fn runtime_inside_element_shaped_data_does_not_charge_evaluation() {
    for program in [
        "$a = {type: \"element\", typeName: \"Text\", partial: false, props: {value: 1 + 2}}",
        "$a = {k: \"Str\", v: 1 + 2}",
    ] {
        assert_eq!(
            under(program, 0, 2_000_000),
            under(program, 2_000_000, 2_000_000)
        );
        assert!(!under(program, 0, 2_000_000).size_limited);
    }
}

#[test]
fn a_depth_cut_default_still_consumes_result_room() {
    let mut program = "root = Stack()\n$n0 = 1\n".to_owned();
    for i in 1..=32 {
        program.push_str(&format!("$n{i} = [$n{}]\n", i - 1));
    }
    // The 18th wrapper exceeds the evaluator's 18 JSON levels. The orphan
    // comes after evaluation in the result, exposing the room already spent
    // by that cut default even though its value is replaced with null.
    program.push_str(&format!("{} = 1\n", "unused".repeat(100)));
    let lib =
        Library::from_json_schema(&serde_json::json!({"$defs": {"Stack": {"properties": {}}}}));
    let run = |room| {
        parse_with_limits(
            &program,
            &lib,
            None,
            Limits::default().with_depth(6).with_result_size(room),
        )
    };
    let whole = run(20000);
    assert!(whole.depth_limited && !whole.size_limited);
    assert_eq!(run(1888), whole);
    assert!(run(1887).size_limited);
}
