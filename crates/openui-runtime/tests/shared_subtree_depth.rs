//! A subtree keeps its own height when its parent is admitted first.
#[path = "support/subtree.rs"]
mod subtree;
use subtree::cut;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(512 * 1024 * 1024);
#[test]
fn measuring_a_parent_does_not_raise_its_shared_childs_use_depth() {
    for row in include_str!("support/subtree-read-depth.tsv").lines() {
        let parts = row.split('\t').collect::<Vec<_>>();
        let source = parts[0];
        let data = parts[1] == "true";
        let prepared = parts[2] == "true";
        let object = parts[3] == "true";
        let first_cut = parts[4].parse::<usize>().unwrap();
        for k in [first_cut - 1, first_cut, first_cut + 1] {
            assert_eq!(
                cut(source, k, data, prepared, object),
                k >= first_cut,
                "source={source} k={k} data={data} prepared={prepared} object={object}"
            );
        }
    }
}

#[test]
fn shared_subtree_cuts_equal_the_union_of_its_separate_uses() {
    // A fixed xorshift stream varies the location, selection, and use of a
    // shared child. Null placeholders preserve each isolated use's position.
    let mut seed = 0xc159_3487_a2b6_09ddu64;
    for case in 0..384 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let (parent, child) = match seed % 4 {
            0 => ("q2", "q2[0]"),
            1 => ("q3", "q3.a"),
            2 => ("$e", "$e[0]"),
            _ => ("q2", "q1"),
        };
        let wrap = |s: &str, code: u64| match code % 4 {
            0 => format!("[{s}]"),
            1 => format!("[@Id({s})]"),
            2 => format!("{{a:[{s}]}}"),
            _ => format!("[[{s}]]"),
        };
        let mut a = wrap(parent, seed >> 3);
        let mut b = wrap(child, seed >> 5);
        if seed & 256 != 0 {
            std::mem::swap(&mut a, &mut b);
        }
        let k = 748 + ((seed >> 10) % 11) as usize;
        for data in [false, true] {
            for prepared in [false, true] {
                for object in [false, true] {
                    let separate = cut(&format!("[{a},null]"), k, data, prepared, object)
                        | cut(&format!("[null,{b}]"), k, data, prepared, object);
                    assert_eq!(
                        cut(&format!("[{a},{b}]"), k, data, prepared, object),
                        separate,
                        "case={case} [{a},{b}] k={k} data={data} prepared={prepared} object={object}"
                    );
                }
            }
        }
    }
}
