//! Public string-unit iteration honors the core's lone-surrogate encoding.
use openui_lang::utf16_units;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(64 * 1024 * 1024);
#[test]
fn paired_lone_and_supplementary_units_keep_javascript_order() {
    assert_eq!(
        utf16_units("a𐀀\u{10f800}\u{10ffff}").collect::<Vec<_>>(),
        vec![97, 0xd800, 0xdc00, 0xd800, 0xdfff]
    );
    assert!(utf16_units("\u{10f800}").cmp(utf16_units("𐀀")).is_lt());
    assert_eq!(
        utf16_units("\u{f0000}").collect::<Vec<_>>(),
        "\u{f0000}".encode_utf16().collect::<Vec<_>>()
    );
}
