//! Replay one standalone query scenario and check its observable transitions.
#![no_main]

mod checks;

use libfuzzer_sys::fuzz_target;
use openui_lang::{JsonValue, Library, Limits, ParseResult, parse_json, parse_with_limits};
use openui_runtime::{
    QueryCommand, QueryCommandError, QueryManager, QueryUpdate, Store, ToolCall, ToolOutcome,
    TreeInput, Value,
};

fuzz_target!(|data: &[u8]| {
    if data.len() > 8192 {
        return;
    }
    let Some(scenario) = std::str::from_utf8(data)
        .ok()
        .and_then(|s| parse_json(s).ok())
    else {
        return;
    };
    // Exercise the ordinary limits and the admission/refusal paths.
    for limits in [Limits::DEFAULT, checks::SMALL] {
        replay(&scenario, limits);
    }
});

fn parse_program(
    source: &str,
    library: &Library,
    limits: Limits,
    store: &mut Store,
) -> ParseResult {
    let program = parse_with_limits(source, library, None, limits);
    // A refused default is part of the input; the program still runs.
    let _ = store.initialize(
        program
            .state_declarations
            .iter()
            .map(|(name, value)| (name.as_str(), Some(value))),
        [],
    );
    crate::checks::check(source.len(), limits, &program);
    program
}

fn outcome(value: &JsonValue) -> Option<ToolOutcome<'_>> {
    static NEGATIVE_ZERO: Value = Value::Num(-0.0);
    static NAN: Value = Value::Num(f64::NAN);
    Some(match value["kind"].as_str()? {
        "result" if value["hostNumber"] == "-0" => ToolOutcome::ResultValue(&NEGATIVE_ZERO),
        "result" if value["hostNumber"] == "NaN" => ToolOutcome::ResultValue(&NAN),
        "result" => ToolOutcome::Result(value.get("value")),
        "tool-not-found" => ToolOutcome::ToolNotFound(strings(&value["availableTools"])?),
        "mcp-error" => ToolOutcome::McpError(value["message"].as_str()?),
        "tool-error" => ToolOutcome::CallFailed(value["message"].as_str()?),
        _ => return None,
    })
}

fn strings(value: &JsonValue) -> Option<Vec<&str>> {
    value.as_array()?.iter().map(JsonValue::as_str).collect()
}

fn receipt(step: &JsonValue, calls: &[(usize, ToolCall)]) -> Option<usize> {
    if let Some(address) = step.get("dispatch") {
        let index = usize::try_from(address["step"].as_u64()?).ok()?;
        let name = address["toolName"].as_str()?;
        let mut matches = calls.iter().enumerate().filter(|(_, (at, call))| {
            *at == index
                && call.tool_name == name
                && call
                    .arguments_json()
                    .and_then(|args| parse_json(&args).ok())
                    == address.get("args").cloned()
        });
        if let Some(occurrence) = address.get("occurrence") {
            return matches
                .nth(usize::try_from(occurrence.as_u64()?).ok()?)
                .map(|(i, _)| i);
        }
        let (first, _) = matches.next()?;
        return matches.next().is_none().then_some(first);
    }
    usize::try_from(step["call"].as_u64()?)
        .ok()
        .filter(|&i| i < calls.len())
}

fn apply_step(
    manager: &mut QueryManager,
    step: &JsonValue,
    program: &ParseResult,
    store: &Store,
    calls: &[(usize, ToolCall)],
) -> Option<Result<QueryUpdate, QueryCommandError>> {
    let ids;
    let command = match step["kind"].as_str()? {
        "evaluate" => QueryCommand::Evaluate { program, store },
        "advance" => QueryCommand::AdvanceClock {
            now_ms: step["now"].as_u64()?,
        },
        "dispose" => QueryCommand::Dispose,
        "activate" => QueryCommand::Activate,
        "invalidate" => {
            ids = match step.get("ids") {
                Some(ids) => strings(ids)?,
                None => Vec::new(),
            };
            QueryCommand::Invalidate {
                statement_ids: &ids,
            }
        }
        "complete" => {
            let id = &calls[receipt(step, calls)?].1.id;
            let outcome = outcome(&step["outcome"])?;
            manager.audit_delivery(id, &outcome).unwrap();
            QueryCommand::Complete { id, outcome }
        }
        "register" => return Some(manager.register_mutations(&TreeInput::new(program), store)),
        "fire" => {
            return Some(manager.fire_mutation(
                step["name"].as_str()?,
                step.get("args").map(Value::from_json).as_ref(),
            ));
        }
        "extra" => {
            let values = match step.get("values") {
                Some(values) => Some(values.as_object()?),
                None => None,
            };
            let _ = manager.set_extra_key_values(
                step["toolName"].as_str()?,
                values
                    .into_iter()
                    .flatten()
                    .map(|(name, value)| (name.as_str(), value)),
            );
            return None;
        }
        _ => return None,
    };
    Some(manager.apply(command))
}

fn replay(scenario: &JsonValue, limits: Limits) {
    let (Some(text), Some(steps)) = (scenario["program"].as_str(), scenario["steps"].as_array())
    else {
        return;
    };
    let library = Library::from_json_schema(&scenario["library"]);
    let mut store = Store::with_limits(limits);
    let mut program = parse_program(text, &library, limits, &mut store);
    let mut manager = match scenario["minimumMs"].as_u64() {
        Some(ms) => match QueryManager::with_refresh_minimum(limits, ms) {
            Ok(manager) => manager,
            Err(_) => return,
        },
        None => QueryManager::with_limits(limits),
    };
    let mut calls: Vec<(usize, ToolCall)> = Vec::new();
    let mut registered = false;
    for (index, step) in steps.iter().take(64).enumerate() {
        match step["kind"].as_str() {
            Some("replace") => {
                if let Some(text) = step["program"].as_str() {
                    program = parse_program(text, &library, limits, &mut store);
                }
            }
            Some("set") => {
                if let Some(name) = step["name"].as_str() {
                    let _ = store.set(name, step.get("value").map(Value::from_json));
                }
            }
            _ => {}
        }

        let snapshot = manager.snapshot_json().to_owned();
        let deadline = manager.next_refresh_due_ms();
        let cache = (manager.cache_entry_count(), manager.cache_bytes());
        let pending: Vec<_> = calls.iter().map(|(_, call)| call.id.is_pending()).collect();
        match apply_step(&mut manager, step, &program, &store, &calls) {
            Some(Ok(update)) => {
                assert_eq!(update.changed, snapshot != manager.snapshot_json());
                assert_eq!(update.next_refresh_due_ms, manager.next_refresh_due_ms());
                match step["kind"].as_str() {
                    Some("evaluate") if !manager.is_disposed() => registered = true,
                    Some("dispose") => registered = false,
                    Some("complete") => {
                        let id = &calls[receipt(step, &calls).unwrap()].1.id;
                        assert!(!id.is_pending());
                        assert!(matches!(
                            manager.apply(QueryCommand::Complete {
                                id,
                                outcome: ToolOutcome::Result(None)
                            }),
                            Err(QueryCommandError::InvalidCall)
                        ));
                    }
                    _ => {}
                }
                for call in update.calls {
                    assert!(call.id.is_pending());
                    calls.push((index, call));
                }
            }
            Some(Err(_)) | None => {
                assert_eq!(snapshot, manager.snapshot_json());
                assert_eq!(deadline, manager.next_refresh_due_ms());
                assert_eq!(cache, (manager.cache_entry_count(), manager.cache_bytes()));
                assert!(
                    calls
                        .iter()
                        .map(|(_, call)| call.id.is_pending())
                        .eq(pending)
                );
            }
        }
        assert_eq!(manager.cache_bytes(), manager.measured_cache_bytes());
        assert!(manager.cache_bytes() <= manager.cache_limit());
        manager
            .audit_cache(calls.iter().map(|(_, call)| &call.id), registered)
            .unwrap();
        // Bound the retained dispatch addresses on long or dense scripts.
        if calls.len() >= 512 {
            break;
        }
    }
}
