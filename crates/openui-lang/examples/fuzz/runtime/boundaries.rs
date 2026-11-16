//! Public evaluator probes start an allowance directly, after parsing at default
//! limits. No limited AST/input admission can stand in for evaluation here.
use super::*;
use openui_lang::{
    EvaluationScope, EvaluationSession, ExpressionEvaluator, Limits, parse_expression, tokenize,
};

pub(super) fn generated() -> Value {
    let mut probes = Vec::new();
    for resource in ["depth", "size"] {
        let limits: &[usize] = if resource == "depth" {
            &[1, 2, 3, 4]
        } else {
            &[3, 4, 8, 16]
        };
        for &limit in limits {
            for (edge, cost) in [("under", limit - 1), ("at", limit), ("over", limit + 1)] {
                // Unary operators require evaluator descent; literal arrays may be shared.
                let source = if resource == "depth" {
                    // Existing evaluator depth allows three internal walk levels
                    // per configured level. Root evaluation uses one level.
                    format!("{}$probe", "!".repeat(2 * limit + cost - 1))
                } else {
                    format!("\"{}\"", "x".repeat(cost - 2))
                };
                probes.push(json!({"resource":resource,"limit":limit,"edge":edge,"source":source}));
            }
        }
    }
    json!(probes)
}

pub(super) fn validate(c: &Value) -> Result<()> {
    let Some(probes) = c.get("evaluationProbes") else {
        return Ok(());
    };
    let probes = probes
        .as_array()
        .ok_or("evaluation probes must be an array")?;
    if probes.len() > 24 {
        return Err("too many evaluation probes".into());
    }
    for p in probes {
        if !["depth", "size"].contains(&text(get(p, "resource")))
            || !["under", "at", "over"].contains(&text(get(p, "edge")))
            || !get(p, "limit")
                .as_u64()
                .is_some_and(|n| (1..=16).contains(&n))
            || !get(p, "source").as_str().is_some_and(|s| s.len() <= 128)
        {
            return Err("invalid evaluation probe".into());
        }
    }
    Ok(())
}

struct Scope;
impl EvaluationScope for Scope {
    fn state(&self, _: &str, _: &ExpressionEvaluator<'_>) -> Option<RuntimeValue> {
        Some(RuntimeValue::Bool(true))
    }
}

pub(super) fn replay(c: &Value) -> Result<Value> {
    let mut observations = Vec::new();
    for p in array(get(c, "evaluationProbes")) {
        let resource = text(get(p, "resource"));
        let limit = get(p, "limit").as_u64().ok_or("evaluation limit")? as usize;
        let limits = if resource == "depth" {
            Limits::DEFAULT.with_depth(limit)
        } else {
            Limits::DEFAULT.with_result_size(limit)
        };
        let ast = parse_expression(&tokenize(text(get(p, "source"))));
        let session = EvaluationSession::with_limits(&Scope, 0, limits);
        let before = session.flags();
        let result = session.evaluate(&ast);
        let control = EvaluationSession::new(&Scope, 0).evaluate(&ast);
        let bytes = |v: &Option<RuntimeValue>| -> Result<Value> {
            v.as_ref()
                .map(|v| value_to_json(v).map(|s| json!(s)).map_err(Into::into))
                .unwrap_or(Ok(Value::Null))
        };
        let actual = bytes(&result.value)?;
        let control_bytes = bytes(&control.value)?;
        observations.push(
            json!({"probe":p,"admitted":!before.size_limited&&!before.depth_limited,
            "cut":{"size":result.size_limited,"depth":result.depth_limited},
            "failed":result.failure.is_some(),"control":control_bytes,
            "controlFlags":{"size":control.size_limited,"depth":control.depth_limited},
            "matchesControl":actual==control_bytes}),
        );
    }
    Ok(json!(observations))
}
