//! One bounded preparation walk records both supplied bytes and shape height.
use super::*;
use std::hash::{Hash, Hasher};
// On 64-bit targets this has the same 16-byte size as (u8, usize): height
// occupies padding, rather than adding a second map per supplied graph.
#[derive(Clone, Copy)]
struct Identity {
    address: usize,
    height: u32,
    kind: u8,
}
impl Identity {
    fn new(kind: u8, address: usize) -> Self {
        Self {
            address,
            height: u32::MAX,
            kind,
        }
    }
}
impl PartialEq for Identity {
    fn eq(&self, b: &Self) -> bool {
        self.kind == b.kind && self.address == b.address
    }
}
impl Eq for Identity {}
impl Hash for Identity {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.kind.hash(h);
        self.address.hash(h);
    }
}
enum Item<'a> {
    V(&'a Value, usize),
    Values(&'a [Value], usize),
    A(&'a Ast, usize),
    E(&'a Element, usize),
    S(&'a str),
    End(Identity, usize, usize),
}

fn first<'a>(
    key: Identity,
    depth: usize,
    todo: &mut Vec<Item<'a>>,
    seen: &mut HashSet<Identity>,
    introduced: &mut Vec<Identity>,
    peak: &mut usize,
    memo: bool,
) -> bool {
    if let Some(prior) = seen.get(&key) {
        *peak = (*peak).max(depth.saturating_add(prior.height as usize));
        return false;
    }
    seen.insert(key);
    introduced.push(key);
    if memo {
        todo.push(Item::End(key, depth, *peak));
        *peak = depth;
    } else {
        *peak = (*peak).max(depth);
    }
    true
}
pub(super) fn one(value: &Value, limits: Limits) -> Result<(usize, usize), EvaluationFlags> {
    one_within(value, limits, &mut limits.work_for(usize::MAX))
}
/// [`one`], spending from `work` and leaving what is left in it.
pub(super) fn one_within(
    value: &Value,
    limits: Limits,
    work: &mut usize,
) -> Result<(usize, usize), EvaluationFlags> {
    measure(
        vec![Item::V(value, 0)],
        0,
        limits,
        &mut HashSet::new(),
        work,
        true,
    )
    .map(|(bytes, depth)| (bytes, depth.unwrap_or(0)))
    .map_err(|depth| EvaluationFlags {
        depth_limited: depth,
        size_limited: !depth,
    })
}
/// Measure a set, spending from `work` and leaving what is left in it.
pub(super) fn prepare<'a>(
    values: &'a JsObject<Value>,
    limits: Limits,
    work: &mut usize,
) -> (
    BTreeMap<&'a str, Result<PreparedReferenceValue, EvaluationFlags>>,
    usize,
    EvaluationFlags,
) {
    let mut seen = HashSet::new();
    let mut tail = EvaluationFlags::default();
    let mut bytes = 2usize;
    let mut out = BTreeMap::new();
    let Some(entries) = values.measured_order(work) else {
        tail.size_limited = true;
        return (out, bytes, tail);
    };
    for (name, value) in entries {
        // Every visited value spends measurement work. Once it is exhausted,
        // record one refusal for the tail instead of allocating per-key errors.
        if *work <= name.len() {
            *work = 0;
            tail.size_limited = true;
            break;
        }
        *work -= name.len();
        let entry = match measure(vec![Item::V(value, 0)], 0, limits, &mut seen, work, true) {
            Ok((size, depth)) => {
                let prepared = PreparedReferenceValue::measured(value, size, depth.unwrap_or(0));
                bytes = bytes
                    .saturating_add(crate::json_string_len(name))
                    .saturating_add(2)
                    .saturating_add(size)
                    .saturating_add(2);
                Ok(prepared)
            }
            Err(depth) => {
                let flags = EvaluationFlags {
                    depth_limited: depth,
                    size_limited: !depth,
                };
                Err(flags)
            }
        };
        out.insert(name.as_str(), entry);
    }
    (out, bytes, tail)
}
fn measure(
    mut todo: Vec<Item<'_>>,
    mut bytes: usize,
    limits: Limits,
    prior: &mut HashSet<Identity>,
    allowance: &mut usize,
    refuse_oversize: bool,
) -> Result<(usize, Option<usize>), bool> {
    let ceiling = limits.result_size().max(limits.work_for(usize::MAX));
    let mut work = *allowance;
    let mut seen = std::mem::take(prior);
    let mut introduced = Vec::new();
    let mut peak = 0usize;
    let result = (|| {
        while let Some(item) = todo.pop() {
            if let Item::End(key, start, previous) = item {
                if let Some(mut entry) = seen.take(&key) {
                    entry.height = u32::try_from(peak.saturating_sub(start)).unwrap_or(u32::MAX);
                    seen.insert(entry);
                }
                peak = peak.max(previous);
                continue;
            }
            if bytes >= ceiling {
                return if refuse_oversize {
                    Err(false)
                } else {
                    Ok((ceiling, None))
                };
            }
            if work == 0 {
                return Err(false);
            }
            work = work.saturating_sub(1);
            let mut add = 0usize;
            match item {
                Item::End(_, _, _) => {}
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
                    if !first(
                        Identity::new(2, std::ptr::from_ref(e) as usize),
                        d,
                        &mut todo,
                        &mut seen,
                        &mut introduced,
                        &mut peak,
                        true,
                    ) {
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
                    for (k, v) in e.props.iter_unordered() {
                        todo.push(Item::S(k));
                        todo.push(Item::V(v, d.saturating_add(1)));
                    }
                }
                Item::V(v, d) => {
                    peak = peak.max(d);
                    if d >= limits.depth() {
                        return Err(true);
                    }
                    match v {
                        Value::Null => add = 4,
                        Value::Bool(b) => add = if *b { 4 } else { 5 },
                        Value::Num(n) => add = crate::json_number_len(*n),
                        Value::Str(s) => todo.push(Item::S(s)),
                        Value::Element(e) => todo.push(Item::E(e, d)),
                        Value::Ast(a) => {
                            todo.push(Item::A(a, d.saturating_add(1)));
                        }
                        Value::Arr(a) => {
                            if !first(
                                Identity::new(0, Arc::as_ptr(a) as usize),
                                d,
                                &mut todo,
                                &mut seen,
                                &mut introduced,
                                &mut peak,
                                Arc::strong_count(a) > 1,
                            ) {
                                continue;
                            }
                            if a.len() > work {
                                return Err(false);
                            }
                            add = 2usize.saturating_add(a.len().saturating_sub(1));
                            todo.push(Item::Values(a, d.saturating_add(1)));
                        }
                        Value::Obj(o) => {
                            if !first(
                                Identity::new(1, Arc::as_ptr(o) as usize),
                                d,
                                &mut todo,
                                &mut seen,
                                &mut introduced,
                                &mut peak,
                                Arc::strong_count(o) > 1,
                            ) {
                                continue;
                            }
                            if o.len() > work {
                                return Err(false);
                            }
                            add =
                                2usize.saturating_add(o.len().saturating_mul(2).saturating_sub(1));
                            for (k, v) in o.iter_unordered() {
                                todo.push(Item::S(k));
                                todo.push(Item::V(v, d.saturating_add(1)));
                            }
                        }
                    }
                }
                Item::A(a, d) => {
                    peak = peak.max(d);
                    if d >= limits.depth() {
                        return Err(true);
                    }
                    if !first(
                        Identity::new(3, std::ptr::from_ref(a) as usize),
                        d,
                        &mut todo,
                        &mut seen,
                        &mut introduced,
                        &mut peak,
                        false,
                    ) {
                        continue;
                    }
                    let n = d.saturating_add(1);
                    add =
                        match a {
                            Ast::Str(_) => r#"{"k":"Str","v":}"#.len(),
                            Ast::Num(n) => r#"{"k":"Num","v":}"#
                                .len()
                                .saturating_add(crate::json_number_len(*n)),
                            Ast::Bool(b) => {
                                r#"{"k":"Bool","v":}"#
                                    .len()
                                    .saturating_add(if *b { 4 } else { 5 })
                            }
                            Ast::Null => r#"{"k":"Null"}"#.len(),
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
                                for (k, v) in p.iter_unordered() {
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
            Ok((
                bytes,
                peak.checked_add(1).filter(|d| *d < u32::MAX as usize),
            ))
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
