//! Bounded sizing of borrowed tree input, before the one session starts.
use super::*;
// Size borrowed input without writing exponentially large JSON for a shared
// caller-built DAG. Once both scaled ceilings are saturated its precise size
// cannot affect an allowance. This walk itself has a work and depth bound.
pub(super) fn input_size(result: &ParseResult, limits: Limits) -> Result<usize, bool> {
    enum Item<'a> {
        V(&'a Value, usize),
        A(&'a Ast, usize),
        E(&'a Element, usize),
        S(&'a str),
    }
    let available = limits.work_for(usize::MAX);
    if result.state_declarations.len() > available {
        return Err(false);
    }
    let mut todo = Vec::new();
    if let Some(root) = &result.root {
        todo.push(Item::E(root, 0));
    }
    for (k, v) in result.state_declarations.iter() {
        todo.push(Item::S(k));
        todo.push(Item::V(v, 0));
    }
    let ceiling = limits.result_size().max(limits.work_for(usize::MAX));
    let mut bytes = r#"{"root":,"stateDeclarations":{}}"#
        .len()
        .saturating_add(if result.root.is_none() { 4 } else { 0 })
        .saturating_add(result.state_declarations.len().saturating_mul(2).saturating_sub(1));
    let mut work = limits.work_for(usize::MAX);
    while let Some(item) = todo.pop() {
        if bytes >= ceiling {
            return Ok(ceiling);
        }
        if work == 0 {
            return Err(false);
        }
        work = work.saturating_sub(1);
        let mut add = 0usize;
        match item {
            Item::S(s) => {
                add = 2;
                for c in s.chars() {
                    if work == 0 {
                        return Err(false);
                    }
                    work = work.saturating_sub(1);
                    add = add.saturating_add(match c {
                        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                        c if c < '\u{20}' => 6,
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
                    Value::Num(_) => add = value_json(v).len(),
                    Value::Str(s) => todo.push(Item::S(s)),
                    Value::Element(e) => todo.push(Item::E(e, d)),
                    Value::Ast(a) => todo.push(Item::A(a, d)),
                    Value::Arr(a) => {
                        if a.len() > work {
                            return Err(false);
                        }
                        add = 2usize.saturating_add(a.len().saturating_sub(1));
                        for v in a.iter() {
                            todo.push(Item::V(v, d.saturating_add(1)));
                        }
                    }
                    Value::Obj(o) => {
                        if o.len() > work {
                            return Err(false);
                        }
                        add = 2usize.saturating_add(o.len().saturating_mul(2).saturating_sub(1));
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
                let n = d.saturating_add(1);
                add=match a {
                    Ast::Str(_)=>r#"{"k":"Str","v":}"#.len(),
                    Ast::Num(_)|Ast::Bool(_)|Ast::Null=>a.to_json().len(),
                    Ast::Ref(_)=>r#"{"k":"Ref","n":}"#.len(),
                    Ast::Ph(_)=>r#"{"k":"Ph","n":}"#.len(),
                    Ast::StateRef(_)=>r#"{"k":"StateRef","n":}"#.len(),
                    Ast::RuntimeRef{ref_type,..}=>r#"{"k":"RuntimeRef","n":,"refType":""}"#.len().saturating_add(ref_type.as_str().len()),
                    Ast::Arr(items)=>r#"{"k":"Arr","els":[]}"#.len().saturating_add(items.len().saturating_sub(1)),
                    Ast::Obj(items)=>r#"{"k":"Obj","entries":[]}"#.len().saturating_add(items.len().saturating_mul(4).saturating_sub(1)),
                    Ast::Comp{args,function,mapped_props,..}=>r#"{"k":"Comp","name":,"args":[]}"#.len().saturating_add(args.len().saturating_sub(1)).saturating_add(if *function{r#","fn":true"#.len()}else{0}).saturating_add(mapped_props.as_ref().map_or(0,|p|r#","mappedProps":{}"#.len().saturating_add(p.len().saturating_mul(2).saturating_sub(1)))),
                    Ast::BinOp{op,..}=>r#"{"k":"BinOp","op":"","left":,"right":}"#.len().saturating_add(op.len()),
                    Ast::UnaryOp{op,..}=>r#"{"k":"UnaryOp","op":"","operand":}"#.len().saturating_add(op.len()),
                    Ast::Ternary{..}=>r#"{"k":"Ternary","cond":,"then":,"else":}"#.len(),
                    Ast::Member{..}=>r#"{"k":"Member","obj":,"field":}"#.len(),
                    Ast::Index{..}=>r#"{"k":"Index","obj":,"index":}"#.len(),
                    Ast::Assign{..}=>r#"{"k":"Assign","target":,"value":}"#.len(),
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
    Ok(bytes)
}
