//! Reusing admitted values preserves first-use work, depth and live memory.
use openui_lang::{ParseResult, parse_json};
use openui_runtime::{JsObject, LibraryFunctions, Value};
use std::sync::Arc;
pub fn schema() -> openui_lang::JsonValue {
    parse_json(r#"{"$defs":{"Box":{"properties":{"a":{},"b":{},"c":{}}}},"functions":{"Id":{"params":{"properties":{"value":{}}}},"Len":{"params":{"properties":{"value":{}}}},"Clone":{"params":{"properties":{"value":{}}}},"Grow":{"params":{"properties":{"value":{}}}},"Throw":{"params":{"properties":{}}}}}"#).unwrap()
}
pub fn functions(j: &openui_lang::JsonValue) -> LibraryFunctions<'_> {
    let mut f = LibraryFunctions::new(&j["functions"]);
    f.register("Id", |v| {
        Ok(match &v {
            Value::Obj(o) => o.get("value").cloned(),
            _ => None,
        })
    })
    .unwrap();
    f.register("Len", |v| {
        Ok(Some(Value::Num(match &v {
            Value::Obj(o) => match o.get("value") {
                Some(Value::Arr(a)) => a.len() as f64,
                Some(Value::Str(s)) => s.len() as f64,
                _ => 0.,
            },
            _ => 0.,
        })))
    })
    .unwrap();
    f.register("Clone", |v| {
        Ok(match &v {
            Value::Obj(o) => o.get("value").map(|v| match v {
                Value::Arr(a) => Value::Arr(Arc::new(a.as_ref().clone())),
                _ => v.clone(),
            }),
            _ => None,
        })
    })
    .unwrap();
    f.register("Grow", |v| {
        Ok(Some(Value::Arr(Arc::new(vec![v.clone(), v]))))
    })
    .unwrap();
    f.register("Throw", |_| Err("boom".into())).unwrap();
    f
}
pub fn shaped(mut p: ParseResult) -> ParseResult {
    let j = parse_json(&p.to_json()).unwrap();
    if let (Some(root), Value::Obj(o)) = (&mut p.root, &Value::from_json(&j["root"]["props"])) {
        root.props = o.as_ref().clone();
    }
    p
}
pub fn deep(n: usize, object: bool) -> Value {
    let mut v = Value::Null;
    for _ in 0..n {
        v = if object {
            let mut o = JsObject::new();
            o.define("a", v);
            Value::Obj(Arc::new(o))
        } else {
            Value::Arr(Arc::new(vec![v]))
        };
    }
    v
}
pub fn nested(mut body: String, n: usize, items: usize, elements: bool) -> String {
    let list = (0..items)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(",");
    for i in 0..n {
        body = format!(
            "@Each([{list}],i{i},{})",
            if elements {
                format!("Box({body})")
            } else {
                body
            }
        );
    }
    body
}

use openui_runtime::Store;
pub fn supplied(width: usize) -> (Store, JsObject<Value>) {
    let mut s = Store::new();
    s.set("$s", Some(Value::Str("x".repeat(width)))).unwrap();
    s.set("$n", Some(Value::Num(3.))).unwrap();
    let mut d = JsObject::new();
    d.define("s", Value::Str("z".repeat(width / 5)));
    s.set("$data", Some(Value::Obj(Arc::new(d)))).unwrap();
    let mut m = JsObject::new();
    m.define("__reactive", Value::Str("assign".into()));
    m.define("target", Value::Str("$s".into()));
    s.set("$marker", Some(Value::Obj(Arc::new(m)))).unwrap();
    s.set("$rows", Some(Value::Arr(Arc::new(vec![Value::Num(1.); 3]))))
        .unwrap();
    for (key, expr, data) in [
        ("$elements", "$n+1", false),
        ("$shaped_elements", "$n+1", true),
        ("$complex", "@Id($data.s)", false),
    ] {
        let mut props = JsObject::new();
        props.define(
            "a",
            Value::Ast(Arc::new(openui_lang::parse_expression(
                &openui_lang::tokenize(expr),
            ))),
        );
        let value = Value::Element(Arc::new(openui_lang::Element {
            type_name: "Box".into(),
            props,
            partial: false,
            has_dynamic_props: true,
            statement_id: None,
        }));
        let value = if data {
            let mut props = JsObject::new();
            props.define("a", value);
            let p = shaped(ParseResult {
                root: Some(openui_lang::Element {
                    type_name: "Box".into(),
                    props,
                    partial: false,
                    has_dynamic_props: true,
                    statement_id: None,
                }),
                ..ParseResult::default()
            });
            p.root.unwrap().props.get("a").unwrap().clone()
        } else {
            value
        };
        s.set(key, Some(Value::Arr(Arc::new(vec![value; 3]))))
            .unwrap();
    }
    let mut refs = JsObject::new();
    refs.define(
        "q1",
        Value::Arr(Arc::new(
            (0..5)
                .map(|_| {
                    let mut o = JsObject::new();
                    o.define("a", Value::Str("y".repeat(width / 5)));
                    Value::Obj(Arc::new(o))
                })
                .collect(),
        )),
    );
    (s, refs)
}
pub fn source(seed: usize) -> String {
    let extended = seed >= 8192;
    let seed = if extended { seed - 8192 } else { seed };
    let leaves: &[&str] = if extended {
        &[
            "@Each($elements,e,e)",
            "@Each($shaped_elements,e,e)",
            "@Each($complex,e,e)",
            "@Len(@Id(@Each($elements,e,e)))",
            "@Len(@Clone(@Each($shaped_elements,e,e)))",
            "@Each($rows,z,Box(@Each($elements,e,e)))",
            "@Len(@Clone(q1))",
            "@Grow([$s,$s])",
            "@Each($rows,r,$marker)",
            "($n>2?@Each($elements,e,e):@Throw())",
            "@Len(@Grow([$s,$s]))",
            "@Each(q1,r,@Id(r.a))",
        ]
    } else {
        &[
            "$s",
            "[$s,$s,$s,$s]",
            "$data.s",
            "q1[0].a",
            "@Each(q1,v0,q1[0].a)",
            "($n>2?[$marker,$data.s]:@Throw())",
            "@Len(@Id([$s,$s,$s,$s]))",
            "@Len(@Clone([$s,$s,$s,$s]))",
            "@Grow($data)",
            "@Id($marker)",
            "@Id(q1)",
            "[$marker,$data.s]",
        ]
    };
    let mut b = leaves[seed % leaves.len()].to_owned();
    let mut code = seed / leaves.len();
    for level in 0..4 {
        b = match code % 6 {
            0 => b,
            1 => format!("@Id({b})"),
            2 => format!("Box({b})"),
            3 => format!("[{b},{b}]"),
            4 => format!("@Each($rows,v{}, {b})", level + 1),
            _ => format!("($n>2?{b}:@Throw())"),
        };
        code /= 6;
    }
    format!("q1=Query(\"t\",{{}})\nroot=Box({b})")
}
