//! Program generation shared by streaming regressions and the main differential.

pub fn late_aliases(
    depth: usize,
    chain: usize,
    offset: usize,
    shape: usize,
    late: usize,
) -> Vec<String> {
    let wraps = depth.saturating_sub(5) + offset;
    let deep = format!("{}c0{}", "Box(".repeat(wraps), ")".repeat(wraps));
    let children = match shape {
        0 => format!("{deep}, L, {deep}"),
        1 => format!("{deep}, Box(L), {deep}"),
        _ => format!("{deep}, L, Box(c0), {deep}, L"),
    };
    let mut lines = vec![format!("root=Stack([{children}])\n")];
    for i in 0..chain {
        lines.push(format!("c{i}=c{}\n", i + 1));
    }
    lines.push(format!("c{chain}=Req()\n"));
    let wraps = late / 5;
    lines.push(format!(
        "L={}c{}{}\n",
        "Box(".repeat(wraps),
        late % (chain + 1),
        ")".repeat(wraps)
    ));
    lines.push("unrelated=7\n".into());
    lines
}
