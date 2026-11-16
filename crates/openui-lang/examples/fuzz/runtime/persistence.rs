//! Reproduce the oracle's input recipe locally, then save through the Store's
//! public JSON writer and reload through the core's public JSON reader.
use super::*;

struct Mulberry32(u32);
impl Mulberry32 {
    fn below(&mut self, n: u32) -> u32 {
        self.0 = self.0.wrapping_add(0x6d2b_79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        (((t ^ (t >> 14)) as u64 * n as u64) >> 32) as u32
    }
}

pub(super) fn saved(c: &Value, result: &ParseResult) -> Result<(Value, String)> {
    let seed = u32::from_str_radix(
        &fingerprint(&format!("{}:persisted", text(get(c, "name")))),
        16,
    )?;
    let mut rng = Mulberry32(seed);
    let parsed = openui_lang::parse_json(&result.to_json())?;
    let declarations = get(&parsed, "stateDeclarations");
    let mut store = Store::new();
    for (name, value) in object(declarations) {
        let value = match value {
            Value::String(_) => json!(format!("saved-{}", rng.below(1000))),
            Value::Bool(b) => json!(!b),
            Value::Number(n) => {
                json!(n.as_f64().ok_or("state number")? + 1.0 + rng.below(9) as f64)
            }
            Value::Array(items) => {
                let mut items = items.clone();
                items.push(json!(rng.below(10)));
                json!(items)
            }
            Value::Object(entries) => {
                let mut entries = entries.clone();
                entries.insert("saved".into(), json!(rng.below(1000)));
                json!(entries)
            }
            Value::Null => json!(rng.below(1000)),
        };
        store.set(name, Some(runtime(&value)))?;
    }
    let bytes = store.snapshot_json()?;
    Ok((openui_lang::parse_json(&bytes)?, bytes))
}
