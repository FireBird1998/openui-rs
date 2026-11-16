//! Shared alias shapes, measured through public parsing entry points.
pub fn aliases(n: usize, repeated: bool) -> String {
    let refs = if repeated {
        vec!["a0".to_owned(); 100 * n]
    } else {
        (0..n).map(|i| format!("a{i}")).collect()
    };
    let mut text = format!("root=Stack([{}])\n", refs.join(","));
    for i in 0..n {
        text.push_str(&format!("a{i}=a{}\n", i + 1));
    }
    text.push_str(&format!("a{n}=missing\n"));
    text
}
