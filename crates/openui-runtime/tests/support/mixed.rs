//! Shared fixtures for evaluated elements beside already projected data.
use openui_lang::{Ast, Element, Library, ParseResult, parse, parse_json};
use openui_runtime::{
    JsObject, LibraryFunctions, Limits, Store, TreeInput, Value, evaluate_tree_with_limits,
};
use std::sync::Arc;
pub fn schema() -> openui_lang::JsonValue {
    parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{}}}},"functions":{"Id":{"params":{"properties":{"value":{}}}},"Len":{"params":{"properties":{"value":{}}}},"Grow":{"params":{"properties":{"value":{}}}},"Two":{"params":{"properties":{"a":{},"b":{}}}},"Throw":{"params":{"properties":{}}}}}"#).unwrap()
}
pub fn functions(j: &openui_lang::JsonValue) -> LibraryFunctions<'_> {
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Id", |v| {
        Ok(if let Value::Obj(o) = &v {
            o.get("value").cloned()
        } else {
            None
        })
    })
    .unwrap();
    f.register("Len", |v| {
        Ok(Some(Value::Num(if let Value::Obj(o) = &v {
            match o.get("value") {
                Some(Value::Arr(a)) => a.len() as f64,
                Some(Value::Str(s)) => s.len() as f64,
                _ => 0.,
            }
        } else {
            0.
        })))
    })
    .unwrap();
    f.register("Grow", |v| {
        Ok(if let Value::Obj(o) = &v {
            if let Some(Value::Arr(a)) = o.get("value") {
                let mut out = a.as_ref().clone();
                out.extend(a.iter().cloned());
                Some(Value::Arr(Arc::new(out)))
            } else {
                None
            }
        } else {
            None
        })
    })
    .unwrap();
    f.register("Two", |v| {
        Ok(if let Value::Obj(o) = &v {
            Some(Value::Arr(Arc::new(vec![
                o.get("a").cloned().unwrap_or(Value::Null),
                o.get("b").cloned().unwrap_or(Value::Null),
            ])))
        } else {
            None
        })
    })
    .unwrap();
    f.register("Throw", |_| Err("boom".into())).unwrap();
    f
}
pub fn supplied(n: usize, data: bool) -> Store {
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("x".repeat(n)))).unwrap();
    s.set("$n", Some(Value::Num(3.))).unwrap();
    let mut props = JsObject::new();
    props.define(
        "a",
        Value::Ast(Arc::new(openui_lang::parse_expression(
            &openui_lang::tokenize("$n+1"),
        ))),
    );
    let el = Value::Element(Arc::new(Element {
        type_name: "Box".into(),
        props,
        partial: false,
        has_dynamic_props: true,
        statement_id: None,
    }));
    s.set("$el", Some(if data { shaped_value(el) } else { el }))
        .unwrap();
    let mut marker = JsObject::new();
    marker.define("__reactive", Value::Str("assign".into()));
    marker.define("target", Value::Str("$el".into()));
    s.set("$marker", Some(Value::Obj(Arc::new(marker))))
        .unwrap();
    let mut d = JsObject::new();
    d.define("s", Value::Str("y".repeat(n)));
    s.set("$data", Some(Value::Obj(Arc::new(d)))).unwrap();
    s.set(
        "$rows",
        Some(Value::Arr(Arc::new(vec![Value::Num(0.), Value::Num(1.)]))),
    )
    .unwrap();
    s
}
pub fn shaped(mut p: ParseResult) -> ParseResult {
    let j = parse_json(&p.to_json()).unwrap();
    if let (Some(root), Value::Obj(o)) = (&mut p.root, &Value::from_json(&j["root"]["props"])) {
        root.props = o.as_ref().clone();
    }
    p
}
fn shaped_value(value: Value) -> Value {
    let mut props = JsObject::new();
    props.define("a", value);
    let p = shaped(ParseResult {
        root: Some(Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }),
        ..ParseResult::default()
    });
    p.root.unwrap().props.get("a").unwrap().clone()
}
pub fn minimum(source: &str, n: usize, data: bool) -> usize {
    let j = schema();
    let l = Library::from_json_schema(&j);
    let f = functions(&j);
    let s = supplied(n, data);
    let p = parse(source, &l, None);
    assert!(p.root.is_some() || p.root_expression.is_some());
    let p = if data { shaped(p) } else { p };
    let input = TreeInput::new(&p).with_library(&l).with_functions(&f);
    let (mut lo, mut hi) = (0, 10_000_000);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let r = evaluate_tree_with_limits(
            input.clone(),
            &s,
            Limits::DEFAULT.with_work(mid).with_result_size(64_000_000),
        );
        if r.size_limited || r.depth_limited {
            lo = mid + 1
        } else {
            hi = mid
        }
    }
    lo
}
pub fn ast(s: &str) -> Ast {
    openui_lang::parse_expression(&openui_lang::tokenize(s))
}
pub fn source(seed: usize) -> String {
    let leaves = [
        "[$el,$s]",
        "[$marker,$data.s]",
        "[$el,[$s,$s]]",
        "[$el,{a:$s,b:[$s]}]",
        "[$el,@Set($n,1),$s]",
        "@Grow([$s,$el])",
        "@Two($s,$el)",
        "[$el,{type:\"element\",typeName:\"Box\",props:{a:$s},partial:false,hasDynamicProps:true},$s]",
        "[$el,{type:\"element\",typeName:\"Box\",props:{a:{k:\"Str\",v:$s}},partial:false,hasDynamicProps:true},$s]",
        "[$el,{},[$s],@Set($n,1)]",
        "[$el,@Id({a:$s})]",
        "[$marker,$s,@Two($s,$el)]",
    ];
    let mut body = leaves[seed % leaves.len()].to_string();
    let mut code = seed / leaves.len();
    let style = code % 4;
    code /= 4;
    let levels = code % 4;
    code /= 4;
    for level in 0..levels {
        body = match code % 7 {
            0 => format!("Box({body})"),
            1 => format!("@Len({body})"),
            2 => format!("@Each($rows,i{level},{body})"),
            3 => format!("@Grow([{body},$s])"),
            4 => format!("@Two({body},$el)"),
            5 => format!("{{a:{body},b:$s}}"),
            _ => format!("[{body},$s]"),
        };
        code /= 7;
    }
    match style {
        0 => format!("root=Box(($n>2?{body}:0))"),
        1 => format!("root=Box({body})"),
        2 => format!("root=Box({body},{body})"),
        _ => format!("v={body}\nroot=Box(v,v)"),
    }
}
