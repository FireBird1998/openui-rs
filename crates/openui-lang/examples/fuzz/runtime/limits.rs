//! Upstream has no size/depth/work limits. Uncut controls are differential;
//! cut flags and stop-on-cut failure follow ADR 0001/0003's documented rule.
use super::*;
use openui_lang::{Limits, parse_with_limits};
use openui_runtime::{evaluate_tree_with_limits, run_action_with_limits};

pub(super) fn generated(rng: &mut Rng) -> Value {
    let mut probes = Vec::new();
    for stage in ["parse", "tree", "action"] {
        for resource in ["size", "depth", "work"] {
            let width = 8 + rng.below(9);
            let value = format!("{}$n{}", "[".repeat(width), "]".repeat(width));
            let source = match stage {
                "parse" => format!("$n=2\n$out=@Sum([1,2,3,4,5,6,7,8])\nroot=Box({value},$out)"),
                "tree" => format!("$n=2\nroot=Box({value},$n+1)"),
                _ => format!(
                    "$n=2\n$out=null\nroot=Box(Action([@Set($out,{value}),@OpenUrl(\"https://example.test/\")]))"
                ),
            };
            probes.push(
                json!({"stage":stage,"resource":resource,"limit":1+rng.below(2),"program":source}),
            );
        }
    }
    json!(probes)
}

pub(super) fn validate(c: &Value) -> Result<()> {
    if c.get("limitProbes").is_some_and(|p| !p.is_array()) {
        return Err("limit probes must be an array".into());
    }
    let probes = array(get(c, "limitProbes"));
    if probes.len() > 9 {
        return Err("too many limit probes".into());
    }
    for probe in probes {
        if !["parse", "tree", "action"].contains(&text(get(probe, "stage")))
            || !["size", "depth", "work"].contains(&text(get(probe, "resource")))
            || !get(probe, "limit")
                .as_u64()
                .is_some_and(|n| (1..=8).contains(&n))
            || text(get(probe, "program")).len() > MAX_SOURCE
        {
            return Err("invalid limit probe".into());
        }
    }
    Ok(())
}
fn initialize(result: &ParseResult) -> Result<Store> {
    let mut store = Store::new();
    if !store
        .initialize(
            result
                .state_declarations
                .iter()
                .map(|(k, v)| (k.as_str(), Some(v))),
            [],
        )
        .refused
        .is_empty()
    {
        return Err("probe initialization refused".into());
    }
    Ok(store)
}
fn action(
    result: &ParseResult,
    library: &Library,
    bindings: &BindingProps,
    schema: &Value,
    limits: Limits,
) -> Result<(Value, bool, bool)> {
    let mut store = initialize(result)?;
    let refs = JsObject::new();
    let tree_input = input(result, library, bindings, schema, &refs);
    let tree = evaluate_tree(tree_input.clone(), &store);
    let mut plans = Vec::new();
    if let Some(root) = tree.root {
        discover(&root, "tree", vec![], &mut plans);
    }
    let plan = &plans.first().ok_or("probe action missing")?.1;
    let run = run_action_with_limits(tree_input, &mut store, Some(plan), "probe", None, limits);
    let events = run
        .events
        .iter()
        .map(|e| e.to_json())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((
        json!({"events":format!("[{}]",events.join(",")),"snapshot":store.snapshot_json()?,"failed":run.failure.is_some()}),
        run.size_limited,
        run.depth_limited,
    ))
}
pub(super) fn replay(
    c: &Value,
    library: &Library,
    bindings: &BindingProps,
    schema: &Value,
) -> Result<Value> {
    let mut observations = Vec::new();
    for probe in array(get(c, "limitProbes")) {
        let stage = text(get(probe, "stage"));
        let resource = text(get(probe, "resource"));
        let limit = get(probe, "limit").as_u64().ok_or("probe limit")? as usize;
        let limits = match resource {
            "size" => Limits::default().with_result_size(limit),
            "depth" => Limits::default().with_depth(limit),
            "work" => Limits::default().with_work(limit),
            _ => return Err("unknown probe resource".into()),
        };
        let source = text(get(probe, "program"));
        let full = parse(source, library, None);
        let (cut, control, size, depth, control_size, control_depth, failed) = match stage {
            "parse" => {
                let cut = parse_with_limits(source, library, None, limits);
                (
                    json!(cut.to_json()),
                    json!(full.to_json()),
                    cut.size_limited,
                    cut.depth_limited,
                    full.size_limited,
                    full.depth_limited,
                    None,
                )
            }
            "tree" => {
                let store = initialize(&full)?;
                let refs = JsObject::new();
                let tree_input = input(&full, library, bindings, schema, &refs);
                let cut = evaluate_tree_with_limits(tree_input.clone(), &store, limits);
                let control = evaluate_tree(tree_input.clone(), &store);
                (
                    json!(cut.to_json()?),
                    json!(control.to_json()?),
                    cut.size_limited,
                    cut.depth_limited,
                    control.size_limited,
                    control.depth_limited,
                    None,
                )
            }
            "action" => {
                let (cut, size, depth) = action(&full, library, bindings, schema, limits)?;
                let (control, cs, cd) =
                    action(&full, library, bindings, schema, Limits::default())?;
                let failed = get(&cut, "failed").clone();
                (cut, control, size, depth, cs, cd, Some(failed))
            }
            _ => return Err("unknown probe stage".into()),
        };
        observations.push(json!({"stage":stage,"resource":resource,"limit":limit,"cut":{"size":size,"depth":depth},"controlFlags":{"size":control_size,"depth":control_depth},"changed":!same(&cut,&control),"failed":failed,"control":control}));
    }
    Ok(json!(observations))
}
