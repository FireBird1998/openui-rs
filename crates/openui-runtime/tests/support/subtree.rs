//! A subtree keeps its own height when its parent is admitted first.
use openui_lang::{Library, parse, parse_json};
use openui_runtime::{
    JsObject, Limits, PreparedReferenceValues, Store, TreeInput, Value, evaluate_tree_with_limits,
};
use std::sync::Arc;
#[path = "reuse.rs"]
#[allow(dead_code)]
mod reuse;
pub fn cut(source: &str, k: usize, data: bool, prepared: bool, object: bool) -> bool {
    let j=parse_json(r#"{"$defs":{"Box":{"properties":{"a":{}}}},"functions":{"Id":{"params":{"properties":{"value":{}}}}}}"#).unwrap();
    let l = Library::from_json_schema(&j);
    let mut f = openui_runtime::LibraryFunctions::new(&j["functions"]);
    f.register("Id", |v| {
        Ok(if let Value::Obj(o) = &v {
            o.get("value").cloned()
        } else {
            None
        })
    });
    let x = reuse::deep(k, object);
    let mut refs = JsObject::new();
    refs.define("q1", x.clone());
    refs.define("q2", Value::Arr(Arc::new(vec![x.clone()])));
    let mut o = JsObject::new();
    o.define("a", x.clone());
    refs.define("q3", Value::Obj(Arc::new(o)));
    let cached = PreparedReferenceValues::new(&refs);
    let mut s = Store::new();
    s.set("$e", Some(Value::Arr(Arc::new(vec![x.clone(), x]))))
        .unwrap();
    let p = parse(
        &format!(
            "q1=Query(\"t\",{{}})\nq2=Query(\"t\",{{}})\nq3=Query(\"t\",{{}})\nroot=Box({source})"
        ),
        &l,
        None,
    );
    let p = if data { reuse::shaped(p) } else { p };
    let input = TreeInput::new(&p).with_library(&l).with_functions(&f);
    let input = if prepared {
        input.with_prepared_reference_values(&cached)
    } else {
        input.with_reference_values(&refs)
    };
    evaluate_tree_with_limits(input, &s, Limits::DEFAULT).depth_limited
}
