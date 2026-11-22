//! Registration observations captured independently on main f641ded.
use openui_ffi::*;
use std::sync::{Arc, Mutex};

#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::LIMITS);

struct Constant;
impl LibraryFunction for Constant {
    fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
        Ok(Some("1".into()))
    }
}
struct Replacement;
impl LibraryFunction for Replacement {
    fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
        Ok(Some("2".into()))
    }
}
fn outcome(result: Result<(), OpenUiError>) -> String {
    match result {
        Ok(()) => "Ok".into(),
        Err(error) => format!("{error:?}"),
    }
}
fn draw(session: &RuntimeSession) -> serde_json::Value {
    match session.evaluate() {
        Ok(out) => serde_json::json!([out.json, out.size_limited, out.depth_limited]),
        Err(error) => serde_json::json!(format!("{error:?}")),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Disposed,
    Reentry,
}

#[test]
fn registration_preserves_main_errors_precedence_implementations_and_room() {
    use Mode::*;
    // Name bytes, total retained registration bytes and count are independent
    // checks, in that order around declared-name validation. Include exact
    // boundaries and replacements at full byte/count capacity.
    let rows = [
        ("new", None, 0, "F0", Normal),
        ("replacement", None, 1, "F0", Normal),
        ("invalid", None, 1, "missing", Normal),
        ("name_bytes", Some(1), 0, "F0", Normal),
        ("invalid_name_bytes", Some(1), 0, "missing", Normal),
        ("invalid_with_no_room", Some(64), 0, "x", Normal),
        ("registration_bytes", Some(65), 0, "F0", Normal),
        ("exact_first_bytes", Some(66), 0, "F0", Normal),
        ("replacement_at_first_bytes", Some(66), 1, "F0", Normal),
        ("total_bytes", Some(512), 7, "F7", Normal),
        ("replacement_at_total_bytes", Some(528), 8, "F0", Normal),
        ("invalid_at_total_bytes", Some(528), 8, "missing", Normal),
        ("count", None, 256, "F256", Normal),
        ("replacement_at_count", None, 256, "F0", Normal),
        ("invalid_at_count", None, 256, "missing", Normal),
        ("disposed_new", None, 0, "F0", Disposed),
        ("disposed_replacement", None, 1, "F0", Disposed),
        (
            "disposed_before_name_bytes",
            Some(1),
            0,
            "missing",
            Disposed,
        ),
        ("reentry_new", None, 1, "F1", Reentry),
        ("reentry_replacement", None, 1, "F0", Reentry),
        ("reentry_invalid", None, 1, "missing", Reentry),
        ("reentry_before_name_bytes", Some(512), 1, "", Reentry),
    ];
    let definitions: serde_json::Map<String, serde_json::Value> = (0..257)
        .map(|n| (format!("F{n}"), serde_json::json!({"params":{}})))
        .collect();
    let schema =
        serde_json::json!({"$defs":{"Box":{"properties":{"value":{}}}}, "functions":definitions})
            .to_string();
    let mut observations = Vec::new();
    for (label, cap, filled, target, mode) in rows {
        let library = ComponentLibrary::new(schema.clone()).unwrap();
        let session = RuntimeSession::with_limits(
            library.clone(),
            None,
            RuntimeLimits {
                max_json_bytes: cap.unwrap_or(8 * 1024 * 1024),
                ..Default::default()
            },
        )
        .unwrap();
        let program = outcome(
            session.set_program(parse("root=Box([@F0(),@F1()])".into(), library, None).program),
        );
        for n in 0..filled {
            session
                .register_function(format!("F{n}"), Box::new(Constant))
                .unwrap();
        }
        let target = if target.is_empty() {
            "x".repeat(513)
        } else {
            target.into()
        };
        let attempted = if mode == Reentry {
            let result = Arc::new(Mutex::new(None));
            struct Reenter {
                session: std::sync::Weak<RuntimeSession>,
                target: String,
                result: Arc<Mutex<Option<String>>>,
            }
            impl LibraryFunction for Reenter {
                fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
                    let session = self.session.upgrade().unwrap();
                    *self.result.lock().unwrap() = Some(outcome(
                        session.register_function(self.target.clone(), Box::new(Replacement)),
                    ));
                    Ok(Some("1".into()))
                }
            }
            session
                .register_function(
                    "F0".into(),
                    Box::new(Reenter {
                        session: Arc::downgrade(&session),
                        target: target.clone(),
                        result: result.clone(),
                    }),
                )
                .unwrap();
            session.evaluate().unwrap();
            result.lock().unwrap().take().unwrap()
        } else {
            if mode == Disposed {
                session.dispose();
            }
            outcome(session.register_function(target.clone(), Box::new(Replacement)))
        };
        let after = draw(&session);
        // Probe remaining room through real public admissions; don't infer it
        // from private counters. Skip existing names so replacements cannot
        // conceal room spent by a refused call.
        let mut admitted = 0;
        let mut refusal = None;
        for n in filled..257 {
            let name = format!("F{n}");
            if attempted == "Ok" && name == target {
                continue;
            }
            match session.register_function(name.clone(), Box::new(Constant)) {
                Ok(()) => admitted += 1,
                Err(error) => {
                    refusal = Some((name, format!("{error:?}")));
                    break;
                }
            }
        }
        let after_room = draw(&session);
        let replacement_after_room =
            outcome(session.register_function("F0".into(), Box::new(Replacement)));
        observations.push(
            serde_json::json!({
                "case":label, "program":program, "attempt":attempted,
                "after":after, "room_admissions":admitted, "room_refusal":refusal,
                "after_room":after_room, "replacement_after_room":replacement_after_room,
                "after_replacement":draw(&session), "snapshot":outcome(session.snapshot().map(|_| ())),
                "version":session.version().ok(),
            })
            .to_string(),
        );
    }
    let expected: Vec<_> = include_str!("support/registration-main.jsonl")
        .lines()
        .collect();
    assert_eq!(observations.len(), expected.len());
    for (actual, expected) in observations.iter().zip(expected) {
        assert_eq!(actual, expected);
    }
}

#[test]
fn live_admission_queries_preserve_room_and_match_real_registration() {
    let definitions: serde_json::Map<String, serde_json::Value> = (0..257)
        .map(|n| (format!("F{n}"), serde_json::json!({"params":{}})))
        .collect();
    let schema = serde_json::json!({"$defs":{}, "functions":definitions});
    for cap in [0, 1, 65, 66, 131, 132, 8 * 1024 * 1024] {
        let mut core = openui_session::SessionCore::new(
            Arc::new(openui_session::ComponentLibrary {
                inner: Arc::new(openui_lang::Library::from_json_schema(&schema)),
                schema: Arc::new(schema.clone()),
                defined: None,
            }),
            None,
            openui_session::RuntimeLimits {
                max_json_bytes: cap,
                ..Default::default()
            },
        )
        .unwrap();
        for name in (0..257)
            .map(|n| format!("F{n}"))
            .chain(["F0".into(), "missing".into()])
        {
            let name = name.as_str();
            let query = core.check_function_registration(name);
            // Repeated queries cannot install a callback or consume room.
            assert_eq!(
                format!("{query:?}"),
                format!("{:?}", core.check_function_registration(name))
            );
            let actual = core.register_function(name.into(), Arc::new(|_| Ok(None)));
            assert_eq!(
                format!("{query:?}"),
                format!("{actual:?}"),
                "cap {cap}, name {name}"
            );
        }
    }
}
