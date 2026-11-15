//! Measure each shared-subtree use alone and in both orders.
#[path = "../tests/support/subtree.rs"]
mod subtree;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
fn main() {
    for source in [
        "[[q2],[q2[0]]]",
        "[[q2],null]",
        "[null,[q2[0]]]",
        "[[q2],[@Id(q2[0])]]",
        "[null,[@Id(q2[0])]]",
        "[[q3],[q3.a]]",
        "[[q3],null]",
        "[null,[q3.a]]",
        "[[$e],[$e[0]]]",
        "[[$e],null]",
        "[null,[$e[0]]]",
        "[[q2[0]],[q2]]",
        "[[@Id(q2[0])],[q2]]",
        "[[q3.a],[q3]]",
        "[[$e[0]],[$e]]",
    ] {
        for data in [false, true] {
            for prepared in [false, true] {
                for object in [false, true] {
                    let first_cut =
                        (748..=760).find(|&k| subtree::cut(source, k, data, prepared, object));
                    println!(
                        "{source}\t{data}\t{prepared}\t{object}\t{}",
                        first_cut.unwrap_or(0)
                    );
                }
            }
        }
    }
}
