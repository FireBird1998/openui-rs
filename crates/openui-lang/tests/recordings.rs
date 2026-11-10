//! Recorded streams: every recording in `conformance/recordings/` is in the
//! corpus as `scripts/import-recordings.sh` puts it there, so a recording
//! can't be added, or changed, without its cases. The conformance test then
//! checks each recorded case at every step of its recorded chunking.

mod common;

use common::{conformance_dir, read};
use openui_lang::merge_statements;
use serde_json::Value;

/// The fields the viewer's `Recording` writes (`Recording.swift`). Anything
/// else, an API key or a request header say, has no place in one.
const FIELDS: [&str; 14] = [
    "format",
    "version",
    "createdAt",
    "request",
    "model",
    "servedBy",
    "effort",
    "library",
    "stopReason",
    "inputTokens",
    "outputTokens",
    "chunks",
    "patchOf",
    // Which request of a batch it answers, so a stopped batch can resume.
    "batchRequest",
];

fn json(path: &str) -> Value {
    serde_json::from_str(&read(&conformance_dir().join(path))).unwrap()
}

fn recordings() -> Vec<(String, Value)> {
    let dir = conformance_dir().join("recordings");
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().into_string().unwrap())
                .filter(|f| f.ends_with(".recording.json"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .map(|f| {
            let recording = serde_json::from_str(&read(&dir.join(&f))).unwrap();
            (f, recording)
        })
        .collect()
}

/// The case name the importer gives a recording's file.
fn case_name(file: &str) -> String {
    let stem = file.strip_suffix(".recording.json").unwrap();
    let slug: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("recorded/{slug}")
}

fn find<'a>(cases: &'a Value, name: &str) -> Option<&'a Value> {
    cases["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
}

#[test]
fn every_recording_is_in_the_corpus_as_imported() {
    let all = recordings();
    assert!(!all.is_empty(), "conformance/recordings/ has no recordings");
    let parse_cases = json("recorded.json");
    let merge_cases = json("merge-recorded.json");
    let mut expected_parse = 0;
    for (file, recording) in &all {
        for key in recording.as_object().unwrap().keys() {
            assert!(
                FIELDS.contains(&key.as_str()),
                "{file}: unexpected field {key:?}"
            );
        }
        assert_eq!(recording["format"], "openui-recording", "{file}");
        let chunks: Vec<&str> = recording["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["text"].as_str().unwrap())
            .collect();
        let text = chunks.concat();
        let name = case_name(file);
        match recording["patchOf"].as_str() {
            None => {
                let case = find(&parse_cases, &name).unwrap_or_else(|| {
                    panic!("{file}: no case {name}; run scripts/import-recordings.sh")
                });
                assert_eq!(
                    read(&conformance_dir().join(case["file"].as_str().unwrap())),
                    text,
                    "{name}"
                );
                let sizes: Vec<u64> = chunks.iter().map(|c| c.chars().count() as u64).collect();
                let recorded: Vec<u64> = case["recorded"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|n| n.as_u64().unwrap())
                    .collect();
                assert_eq!(
                    recorded, sizes,
                    "{name}: the recorded chunking is the deltas' sizes"
                );
                assert_eq!(case["schema"], recording["library"], "{name}");
                assert_eq!(case["request"], recording["request"], "{name}");
            }
            Some(patch_of) => {
                let merge = find(&merge_cases, &name)
                    .unwrap_or_else(|| panic!("{file}: no merge case {name}"));
                assert_eq!(merge["existing"], patch_of, "{name}");
                assert_eq!(merge["patch"].as_str(), Some(text.as_str()), "{name}");
                let merged = find(&parse_cases, &format!("{name}-merged"))
                    .unwrap_or_else(|| panic!("{file}: no parse case {name}-merged"));
                assert_eq!(
                    read(&conformance_dir().join(merged["file"].as_str().unwrap())),
                    merge_statements(patch_of, &text, None),
                    "{name}-merged is the merged program"
                );
            }
        }
        expected_parse += 1;
    }
    assert_eq!(
        parse_cases["cases"].as_array().unwrap().len(),
        expected_parse,
        "recorded.json has a case for each recording and no others"
    );
}

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
