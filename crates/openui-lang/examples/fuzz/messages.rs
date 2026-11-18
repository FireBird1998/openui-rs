//! Differential message reads, streaming cuts and writes from the case corpus.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Instant;

use openui_lang::{
    Limits, MessageError, Value as LangValue, build_message_json, parse_json, parse_message,
    stringify_json, utf16_units,
};
use serde_json::{Value, json};

use crate::findings::{Outcome, run as hunt};
use crate::reference::Reference;
use crate::rng::Rng;

const CASES: &str = include_str!("../../../../conformance/messages.json");
const FRAGMENTS: &[&str] = &[
    "]]>openui:content",
    "]]>openui:content?a+%z=q",
    "\n]]>openui:context\n",
    "\r\n]]>openui:end\r\n",
    "]]>openui:con",
    "\n]]>openui:scripts\n",
    "\r",
    "\n",
    "\u{2028}",
    "\u{2029}",
    "?&a==b&",
    "%ED%A0%80",
    "%C3%A9",
    "%A0",
    "é😀",
    "\u{10F800}",
    "\u{10FFFF}",
    r#"{"__proto__":2,"a":3}"#,
    "1e9999",
    "-0",
    "",
];

fn request(seed: u64, index: u64, seeds: &[Value]) -> Value {
    let mut rng = Rng::for_program(seed, index);
    let original = usize::try_from(index).ok().and_then(|i| seeds.get(i));
    let chosen = original.unwrap_or_else(|| rng.pick(seeds));
    if let Some(raw) = chosen["raw"].as_str() {
        let text = if original.is_some() {
            raw.to_owned()
        } else {
            change(raw, &mut rng)
        };
        json!({"op":"parseMessage","raw":text})
    } else {
        let mut build = chosen["build"].clone();
        if original.is_none() {
            match rng.below(4) {
                0 => build["content"] = json!(change(build["content"].as_str().unwrap(), &mut rng)),
                1 => {
                    build["context"] =
                        json!(change(build["context"].as_str().unwrap_or(""), &mut rng))
                }
                2 => {
                    let mut attributes =
                        build["attributes"].as_object().cloned().unwrap_or_default();
                    attributes.insert(change("", &mut rng), json!(change("", &mut rng)));
                    build["attributes"] = Value::Object(attributes);
                }
                _ => build["end"] = json!(rng.chance(50)),
            }
        }
        json!({"op":"buildMessage","build":build})
    }
}

fn change(text: &str, rng: &mut Rng) -> String {
    let mut chars: Vec<_> = text.chars().collect();
    for _ in 0..=rng.below(3) {
        let from = rng.below(chars.len() + 1);
        let to = if rng.chance(35) {
            from + rng.below(chars.len() - from + 1)
        } else {
            from
        };
        chars.splice(from..to, rng.pick(FRAGMENTS).chars());
    }
    chars.into_iter().collect()
}

fn read(raw: &str) -> Result<Value, MessageError> {
    let parsed = parse_message(raw, false, Limits::DEFAULT)?;
    let context_number = match &parsed.context {
        LangValue::Num(n) if *n == 0.0 && n.is_sign_negative() => json!("-0"),
        LangValue::Num(n) if !n.is_finite() => json!(if n.is_sign_negative() {
            "-Infinity"
        } else {
            "Infinity"
        }),
        LangValue::Num(n) => json!(stringify_json(&json!(n))),
        _ => Value::Null,
    };
    let units: Vec<_> = utf16_units(raw).collect();
    let mut cuts = Vec::with_capacity(units.len() + 1);
    for cut in 0..=units.len() {
        let mut prefix = String::new();
        for ch in char::decode_utf16(units[..cut].iter().copied()) {
            match ch {
                Ok(ch) => prefix.push(ch),
                Err(err) => {
                    prefix.push_str(&openui_lang::utf16_unit_string(err.unpaired_surrogate()))
                }
            }
        }
        cuts.push(parse_message(&prefix, true, Limits::DEFAULT)?.to_json());
    }
    Ok(json!({"complete":parsed.to_json(),"contextNumber":context_number,"cuts":cuts}))
}

fn rust(request: &Value) -> Result<Value, MessageError> {
    if let Some(raw) = request["raw"].as_str() {
        return read(raw);
    }
    match build_message_json(&stringify_json(&request["build"]), Limits::DEFAULT) {
        Ok(written) => {
            let parsed = read(&written)?;
            Ok(json!({"text":written,"read":parsed}))
        }
        Err(MessageError::InvalidAttribute) => Ok(json!({"threw":"URIError"})),
        Err(other) => Err(other),
    }
}

pub fn run(seed: u64, batches: impl Iterator<Item = Vec<u64>>, reference: &mut Reference) -> bool {
    let corpus = parse_json(CASES).expect("read the message cases");
    let seeds = corpus.as_array().expect("message cases must be an array");
    let mut count = 0;
    let mut matches = 0;
    let mut failures = HashMap::new();
    for batch in batches {
        let Some(&first) = batch.first() else {
            continue;
        };
        hunt().progress(
            "messages",
            &format!(
                "scripts/fuzz.sh --messages --seed {seed} --from {first} --count {}",
                batch.len()
            ),
        );
        let tests: Vec<_> = batch
            .iter()
            .map(|&index| request(seed, index, seeds))
            .collect();
        // Bun's readline also treats JS paragraph and line separators as
        // request boundaries. Escape them on the wire and keep the input units.
        let requests: Vec<_> = tests
            .iter()
            .map(|request| {
                stringify_json(request)
                    .replace('\u{2028}', "\\u2028")
                    .replace('\u{2029}', "\\u2029")
            })
            .collect();
        let answers = reference.ask_lines(&requests);
        for ((&index, test), upstream) in batch.iter().zip(&tests).zip(answers) {
            count += 1;
            let start = Instant::now();
            let rust = catch_unwind(AssertUnwindSafe(|| {
                rust(test).map(|answer| stringify_json(&answer))
            }));
            let core_ms = start.elapsed().as_secs_f64() * 1000.0;
            let (outcome, actual) = match rust {
                Ok(result) if start.elapsed() > hunt().slow => {
                    (Outcome::BlowUp, format!("{result:?}"))
                }
                Ok(Ok(actual)) if actual == upstream.trim_end() => {
                    matches += 1;
                    continue;
                }
                Ok(Ok(actual)) => (Outcome::Mismatch, actual),
                Ok(Err(err)) => (Outcome::Mismatch, err.to_string()),
                Err(_) => (Outcome::CorePanicked, String::new()),
            };
            *failures.entry(outcome).or_insert(0) += 1;
            let replay = format!("scripts/fuzz.sh --messages --seed {seed} --only {index}");
            eprintln!("message {outcome:?}, {replay}");
            eprintln!("request: {}", stringify_json(test));
            eprintln!("lang-core: {}\nRust: {actual}", upstream.trim_end());
            hunt().record(&json!({
                "kind": outcome.kind(), "check": "messages", "seed": seed, "index": index,
                "replay": replay, "core_ms": core_ms, "request": stringify_json(test),
                "reference": upstream.trim_end(), "rust": actual,
            }));
        }
    }
    let total_failures: usize = failures.values().sum();
    println!(
        "message fuzz seed {seed}: {count} inputs, {matches} match, {total_failures} findings"
    );
    hunt().summary(
        "messages",
        &json!({
            "inputs": count, "match": matches,
            "mismatch": failures.get(&Outcome::Mismatch).copied().unwrap_or(0),
            "crash": failures.get(&Outcome::CorePanicked).copied().unwrap_or(0),
            "blow-up": failures.get(&Outcome::BlowUp).copied().unwrap_or(0),
        }),
    );
    failures.is_empty()
}
