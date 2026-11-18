//! Differential reads of response bundles, including paired revision equality.

#[path = "../../tests/support/response_bundles.rs"]
mod support;

use std::{panic::catch_unwind, time::Instant};

use serde_json::{Value, json};

use crate::{
    findings::{Outcome, run as hunt},
    reference::Reference,
    rng::Rng,
};

const CHECK: &str = "response-bundles";
const FRAGMENTS: &[&str] = &[
    "]]>openui:content?name=%F4%8F%A0%80",
    "]]>openui:content?name=x\r",
    "]]>openui:scripts?",
    "\n]]>openui:scripts {\"count\":1}\n[{\"name\":\"x\",\"code\":\"a\"}]",
    "\n]]>openui:context\n",
    "\r\n]]>openui:end\r\n",
    "]]>openui:con",
    "]]>unknown",
    "\u{2028}",
    "\u{2029}",
    "\r",
    "\n",
    "'",
    "\"",
    "\\",
    "//",
    "#",
    "%zz",
    "%EF%B7%90",
    "é😀",
    "\u{fdd0}\u{e000}\u{10f800}",
    r#"\ud83d\ude00"#,
    "",
];

type Input = Option<Vec<u16>>;
type Batch = Vec<Input>;

fn inputs(corpus: &Value) -> Vec<Batch> {
    let Some(cases) = corpus["cases"].as_array() else {
        return Vec::new();
    };
    let input = |case: &Value| support::units(case.get("units").unwrap_or(&case["raw"]));
    let mut seeds: Vec<_> = cases.iter().map(|case| vec![input(case)]).collect();
    if let Some(pairs) = corpus["revisionPairs"].as_array() {
        seeds.extend(pairs.iter().filter_map(|pair| {
            // The paired golden test checks the direction of locale tie
            // differences. Both responses still seed the individual reads.
            if pair["localeTie"] == true {
                return None;
            }
            let left = cases.iter().find(|case| case["name"] == pair["left"])?;
            let right = cases.iter().find(|case| case["name"] == pair["right"])?;
            Some(vec![input(left), input(right)])
        }));
    }
    seeds
}

fn request(seed: u64, index: u64, seeds: &[Batch]) -> Batch {
    if let Some(original) = usize::try_from(index).ok().and_then(|at| seeds.get(at)) {
        return original.clone();
    }
    let mut rng = Rng::for_program(seed, index);
    let base = rng.pick(seeds);
    let input = rng.pick(base).as_deref().unwrap_or_default();
    let mut edited = input.to_owned();
    for _ in 0..=rng.below(3) {
        let start = rng.below(edited.len() + 1);
        let end = if rng.chance(45) {
            start + rng.below(edited.len() - start + 1)
        } else {
            start
        };
        let replacement: Vec<u16> = if rng.chance(10) {
            vec![0xd800 + rng.below(0x800) as u16]
        } else {
            rng.pick(FRAGMENTS).encode_utf16().collect()
        };
        edited.splice(start..end, replacement);
    }
    vec![Some(input.to_owned()), Some(edited)]
}

fn wire(inputs: &Batch) -> String {
    let mut request =
        json!({"op":"parseResponseBundles","raw":inputs.first().and_then(Option::as_ref)});
    if let Some(other) = inputs.get(1) {
        request["other"] = json!(other);
    }
    request.to_string()
}

fn compare(inputs: &Batch, upstream: &str) -> Result<(), String> {
    let expected: Vec<Value> =
        serde_json::from_str(upstream).map_err(|error| format!("reference reply: {error}"))?;
    if expected.len() != inputs.len() {
        return Err(format!(
            "reference returned {} reads, wanted {}",
            expected.len(),
            inputs.len()
        ));
    }
    let mut revisions = Vec::new();
    for (index, (expected, input)) in expected.iter().zip(inputs).enumerate() {
        let actual =
            support::read(input.as_deref(), &mut revisions).map_err(|error| error.to_string())?;
        if actual["complete"] != expected["complete"] {
            return Err(mismatch(
                index,
                "stored",
                &expected["complete"],
                &actual["complete"],
            ));
        }
        let want = expected["cuts"]
            .as_array()
            .ok_or("reference reply has no cuts")?;
        let got = actual["cuts"].as_array().ok_or("Rust reply has no cuts")?;
        if want.len() != got.len() {
            return Err(format!(
                "response {index}: reference has {} cuts, Rust has {}",
                want.len(),
                got.len()
            ));
        }
        for (cut, (want, got)) in want.iter().zip(got).enumerate() {
            if want != got {
                return Err(mismatch(index, &format!("cut {cut}"), want, got));
            }
        }
    }
    Ok(())
}

fn mismatch(index: usize, at: &str, expected: &Value, actual: &Value) -> String {
    format!("response {index} {at}\nrenderer: {expected}\nRust: {actual}")
}

pub fn run(seed: u64, batches: impl Iterator<Item = Vec<u64>>, reference: &mut Reference) -> bool {
    let seeds =
        serde_json::from_str(support::CASES).map_or_else(|_| Vec::new(), |corpus| inputs(&corpus));
    if seeds.is_empty() {
        eprintln!("response bundle fuzz: invalid or empty corpus");
        return false;
    }
    let (mut count, mut reads, mut cuts, mut matched, mut mismatches, mut crashes, mut slow) =
        (0, 0, 0, 0, 0, 0, 0);
    for batch in batches {
        let Some(&first) = batch.first() else {
            continue;
        };
        hunt().progress(
            CHECK,
            &format!(
                "scripts/fuzz.sh --{CHECK} --seed {seed} --from {first} --count {}",
                batch.len()
            ),
        );
        let inputs: Vec<_> = batch
            .iter()
            .map(|&index| request(seed, index, &seeds))
            .collect();
        let requests: Vec<_> = inputs.iter().map(wire).collect();
        let replies = reference.ask_lines(&requests);
        for (((&index, inputs), request), reply) in
            batch.iter().zip(&inputs).zip(&requests).zip(replies)
        {
            count += 1;
            reads += inputs.len();
            cuts += inputs
                .iter()
                .map(|input| input.as_ref().map_or(1, |text| text.len() + 1))
                .sum::<usize>();
            let start = Instant::now();
            let result = catch_unwind(|| compare(inputs, &reply));
            let elapsed = start.elapsed();
            let (outcome, detail) = match result {
                Ok(result) if elapsed > hunt().slow => {
                    slow += 1;
                    (Outcome::BlowUp, format!("{result:?} after {elapsed:?}"))
                }
                Ok(Ok(())) => {
                    matched += 1;
                    continue;
                }
                Ok(Err(detail)) => {
                    mismatches += 1;
                    (Outcome::Mismatch, detail)
                }
                Err(_) => {
                    crashes += 1;
                    (Outcome::CorePanicked, String::new())
                }
            };
            let replay = format!("scripts/fuzz.sh --{CHECK} --seed {seed} --only {index}");
            eprintln!("response bundle {outcome:?}, {replay}\n{detail}");
            hunt().record(&json!({
                "kind": outcome.kind(), "check": CHECK, "seed": seed, "index": index,
                "replay": replay, "core_ms": elapsed.as_secs_f64() * 1000.0, "request": request,
                "reference": reply.trim_end(), "rust": detail,
            }));
        }
    }
    let findings = mismatches + crashes + slow;
    println!(
        "response bundle fuzz seed {seed}: {count} requests, {reads} stored reads, {cuts} streaming cuts, {matched} matching requests, {findings} findings"
    );
    hunt().summary(CHECK, &json!({
        "inputs": count, "match": matched, "mismatch": mismatches, "crash": crashes, "blow-up": slow,
    }));
    findings == 0
}
