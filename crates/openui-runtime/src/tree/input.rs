//! Bounded sizing of borrowed tree input, before the one session starts.
use super::*;
enum Item<'a> {
    V(&'a Value, usize),
    Values(&'a [Value], usize),
    A(&'a Ast, usize),
    E(&'a Element, usize),
    S(&'a str),
}

// Size borrowed input without writing exponentially large JSON for a shared
// caller-built DAG. Once both scaled ceilings are saturated its precise size
// cannot affect an allowance. This walk itself has a work and depth bound.
pub(super) fn input_size(input: &TreeInput<'_>, limits: Limits) -> Result<usize, bool> {
    let measured = openui_lang::PreparedProgram::new(input.result, limits);
    let flags = measured.flags();
    if flags.depth_limited || flags.size_limited {
        Err(flags.depth_limited)
    } else {
        Ok(measured.input_bytes())
    }
}

pub(super) fn value_size(
    value: &Value,
    limits: Limits,
    seen: &mut std::collections::HashSet<(u8, usize)>,
    work: &mut usize,
) -> Result<usize, bool> {
    measure(vec![Item::V(value, 0)], 0, limits, seen, work, true)
}
pub(super) fn text_size(text: &str, limits: Limits) -> Result<usize, bool> {
    let work = limits.work_for(usize::MAX);
    if text.len() > work {
        return Err(false);
    }
    let bytes = openui_lang::json_string_len(text).saturating_add(2);
    if bytes > limits.result_size().max(work) {
        Err(false)
    } else {
        Ok(bytes)
    }
}

fn measure(
    mut todo: Vec<Item<'_>>,
    mut bytes: usize,
    limits: Limits,
    prior: &mut std::collections::HashSet<(u8, usize)>,
    allowance: &mut usize,
    refuse_oversize: bool,
) -> Result<usize, bool> {
    let ceiling = limits.result_size().max(limits.work_for(usize::MAX));
    let mut work = *allowance;
    let mut seen = std::mem::take(prior);
    let mut introduced = Vec::new();
    let mut insert = |key| {
        if seen.insert(key) {
            introduced.push(key);
            true
        } else {
            false
        }
    };
    let result = (|| {
        while let Some(item) = todo.pop() {
            if bytes >= ceiling {
                return if refuse_oversize {
                    Err(false)
                } else {
                    Ok(ceiling)
                };
            }
            if work == 0 {
                return Err(false);
            }
            work = work.saturating_sub(1);
            let mut add = 0usize;
            match item {
                Item::Values(values, d) => {
                    if let Some((first, rest)) = values.split_first() {
                        if !rest.is_empty() {
                            todo.push(Item::Values(rest, d));
                        }
                        todo.push(Item::V(first, d));
                    }
                }
                Item::S(s) => {
                    add = 2;
                    for c in s.chars() {
                        if work == 0 {
                            return Err(false);
                        }
                        work = work.saturating_sub(1);
                        add = add.saturating_add(match c {
                            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                            c if c < '\u{20}' || c as u32 >= 0xF0000 => 6,
                            c => c.len_utf8(),
                        });
                        if add >= ceiling {
                            break;
                        }
                    }
                }
                Item::E(e, d) => {
                    if d >= limits.depth() {
                        return Err(true);
                    }
                    if !insert((2u8, std::ptr::from_ref(e) as usize)) {
                        continue;
                    }
                    add = r#"{"type":"element","typeName":,"props":{},"partial":false,"hasDynamicProps":false}"#.len()
                    .saturating_sub(usize::from(e.partial)).saturating_sub(usize::from(e.has_dynamic_props))
                    .saturating_add(e.props.len().saturating_mul(2).saturating_sub(1));
                    todo.push(Item::S(&e.type_name));
                    if let Some(id) = &e.statement_id {
                        todo.push(Item::S(id));
                        add = add.saturating_add(r#","statementId":"#.len());
                    }
                    if e.props.len() > work {
                        return Err(false);
                    }
                    for (k, v) in e.props.iter() {
                        todo.push(Item::S(k));
                        todo.push(Item::V(v, d.saturating_add(1)));
                    }
                }
                Item::V(v, d) => {
                    if d >= limits.depth() {
                        return Err(true);
                    }
                    match v {
                        Value::Null => add = 4,
                        Value::Bool(b) => add = if *b { 4 } else { 5 },
                        Value::Num(n) => add = openui_lang::json_number_len(*n),
                        Value::Str(s) => todo.push(Item::S(s)),
                        Value::Element(e) => todo.push(Item::E(e, d)),
                        Value::Ast(a) => todo.push(Item::A(a, d)),
                        Value::Arr(a) => {
                            if !insert((0u8, Arc::as_ptr(a) as usize)) {
                                continue;
                            }
                            if a.len() > work {
                                return Err(false);
                            }
                            add = 2usize.saturating_add(a.len().saturating_sub(1));
                            todo.push(Item::Values(a, d.saturating_add(1)));
                        }
                        Value::Obj(o) => {
                            if !insert((1u8, Arc::as_ptr(o) as usize)) {
                                continue;
                            }
                            if o.len() > work {
                                return Err(false);
                            }
                            add =
                                2usize.saturating_add(o.len().saturating_mul(2).saturating_sub(1));
                            for (k, v) in o.iter() {
                                todo.push(Item::S(k));
                                todo.push(Item::V(v, d.saturating_add(1)));
                            }
                        }
                    }
                }
                Item::A(a, d) => {
                    if d >= limits.depth() {
                        return Err(true);
                    }
                    if !insert((3u8, std::ptr::from_ref(a) as usize)) {
                        continue;
                    }
                    let n = d.saturating_add(1);
                    add =
                        match a {
                            Ast::Str(_) => r#"{"k":"Str","v":}"#.len(),
                            Ast::Num(n) => openui_lang::json_number_len(*n),
                            Ast::Bool(b) => {
                                if *b {
                                    4
                                } else {
                                    5
                                }
                            }
                            Ast::Null => 4,
                            Ast::Ref(_) => r#"{"k":"Ref","n":}"#.len(),
                            Ast::Ph(_) => r#"{"k":"Ph","n":}"#.len(),
                            Ast::StateRef(_) => r#"{"k":"StateRef","n":}"#.len(),
                            Ast::RuntimeRef { ref_type, .. } => {
                                r#"{"k":"RuntimeRef","n":,"refType":""}"#
                                    .len()
                                    .saturating_add(ref_type.as_str().len())
                            }
                            Ast::Arr(items) => r#"{"k":"Arr","els":[]}"#
                                .len()
                                .saturating_add(items.len().saturating_sub(1)),
                            Ast::Obj(items) => r#"{"k":"Obj","entries":[]}"#
                                .len()
                                .saturating_add(items.len().saturating_mul(4).saturating_sub(1)),
                            Ast::Comp {
                                args,
                                function,
                                mapped_props,
                                ..
                            } => {
                                r#"{"k":"Comp","name":,"args":[]}"#
                                    .len()
                                    .saturating_add(args.len().saturating_sub(1))
                                    .saturating_add(if *function {
                                        r#","fn":true"#.len()
                                    } else {
                                        0
                                    })
                                    .saturating_add(
                                        mapped_props.as_ref().map_or(0, |p| {
                                            r#","mappedProps":{}"#.len().saturating_add(
                                                p.len().saturating_mul(2).saturating_sub(1),
                                            )
                                        }),
                                    )
                            }
                            Ast::BinOp { op, .. } => r#"{"k":"BinOp","op":"","left":,"right":}"#
                                .len()
                                .saturating_add(op.len()),
                            Ast::UnaryOp { op, .. } => r#"{"k":"UnaryOp","op":"","operand":}"#
                                .len()
                                .saturating_add(op.len()),
                            Ast::Ternary { .. } => {
                                r#"{"k":"Ternary","cond":,"then":,"else":}"#.len()
                            }
                            Ast::Member { .. } => r#"{"k":"Member","obj":,"field":}"#.len(),
                            Ast::Index { .. } => r#"{"k":"Index","obj":,"index":}"#.len(),
                            Ast::Assign { .. } => r#"{"k":"Assign","target":,"value":}"#.len(),
                        };
                    match a {
                        Ast::Str(s) | Ast::Ref(s) | Ast::Ph(s) | Ast::StateRef(s) => {
                            todo.push(Item::S(s))
                        }
                        Ast::RuntimeRef { n, .. } => todo.push(Item::S(n)),
                        Ast::Arr(a) => {
                            if a.len() > work {
                                return Err(false);
                            }
                            for v in a.iter() {
                                todo.push(Item::A(v, n));
                            }
                        }
                        Ast::Obj(o) => {
                            if o.len() > work {
                                return Err(false);
                            }
                            for (k, v) in o.iter() {
                                todo.push(Item::S(k));
                                todo.push(Item::A(v, n));
                            }
                        }
                        Ast::Comp {
                            name,
                            args,
                            mapped_props,
                            ..
                        } => {
                            todo.push(Item::S(name));
                            if args.len() > work {
                                return Err(false);
                            }
                            for a in args.iter() {
                                todo.push(Item::A(a, n));
                            }
                            if let Some(p) = mapped_props {
                                if p.len() > work {
                                    return Err(false);
                                }
                                for (k, v) in p.iter() {
                                    todo.push(Item::S(k));
                                    todo.push(Item::A(v, n));
                                }
                            }
                        }
                        Ast::BinOp { left, right, .. } => {
                            todo.push(Item::A(left, n));
                            todo.push(Item::A(right, n));
                        }
                        Ast::UnaryOp { operand, .. } => todo.push(Item::A(operand, n)),
                        Ast::Member { obj, field } => {
                            todo.push(Item::A(obj, n));
                            todo.push(Item::S(field));
                        }
                        Ast::Index { obj, index } => {
                            todo.push(Item::A(obj, n));
                            todo.push(Item::A(index, n));
                        }
                        Ast::Ternary {
                            cond,
                            then,
                            otherwise,
                        } => {
                            todo.push(Item::A(cond, n));
                            todo.push(Item::A(then, n));
                            todo.push(Item::A(otherwise, n));
                        }
                        Ast::Assign { target, value } => {
                            todo.push(Item::S(target));
                            todo.push(Item::A(value, n));
                        }
                        _ => {}
                    }
                }
            }
            bytes = bytes.saturating_add(add);
        }
        if refuse_oversize && bytes >= ceiling {
            Err(false)
        } else {
            Ok(bytes)
        }
    })();
    if result.is_err() {
        for key in introduced {
            seen.remove(&key);
        }
    }
    *prior = seen;
    *allowance = work;
    result
}
