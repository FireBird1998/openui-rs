//! Public native transport liveness; schedules use gates, never speed assertions.
use openui_ffi::*;
use std::sync::{
    Arc, Barrier, Mutex, Weak,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);
const SCHEMA: &str = r#"{"$defs":{"Box":{"properties":{"value":{}}}},"functions":{"Echo":{"params":{}},"Other":{"params":{}}}}"#;
fn setup() -> Arc<RuntimeSession> {
    let library = ComponentLibrary::new(SCHEMA.into()).unwrap();
    let session = RuntimeSession::new(library.clone(), None).unwrap();
    session
        .set_program(parse("$n=0\nroot=Box(@Echo())".into(), library, None).program)
        .unwrap();
    session.initialize(None).unwrap();
    session
}
struct Function<F>(F);
impl<F: Fn(String) -> Result<Option<String>, FunctionError> + Send + Sync> LibraryFunction
    for Function<F>
{
    fn call(&self, json: String) -> Result<Option<String>, FunctionError> {
        (self.0)(json)
    }
}
// Isolate potential deadlocks so a broken transport fails instead of hanging the
// suite. This is a generous liveness watchdog, not an elapsed-time assertion.
// Stress work itself is a fixed number of seeded steps.
fn watched(name: &str, body: impl FnOnce()) {
    if std::env::var("OPENUI_CALLBACK_CHILD").as_deref() == Ok(name) {
        body();
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", name, "--nocapture"])
        .env("OPENUI_CALLBACK_CHILD", name)
        .spawn()
        .unwrap();
    for _ in 0..6000 {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "callback schedule failed: {status}");
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.kill().unwrap();
    child.wait().unwrap();
    panic!("callback schedule stalled (liveness watchdog)");
}
struct DropFunction {
    session: Weak<RuntimeSession>,
    drops: Arc<AtomicUsize>,
    slow: bool,
}
impl LibraryFunction for DropFunction {
    fn call(&self, _: String) -> Result<Option<String>, FunctionError> {
        if self.slow {
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(Some("7".into()))
    }
}
impl Drop for DropFunction {
    fn drop(&mut self) {
        if self.slow {
            std::thread::sleep(Duration::from_millis(2));
        }
        if let Some(session) = self.session.upgrade() {
            // This must refuse on the worker AND on callers destroying a job
            // refused after disposal. A caught destructor panic cannot hide it:
            // the successful-refusal counter is incremented last.
            assert_eq!(
                session.version().unwrap_err().to_string(),
                "session already in use"
            );
            assert!(matches!(
                session.snapshot(),
                Err(OpenUiError::Worker { .. })
            ));
            session.dispose();
        }
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn implementation(
    session: &Arc<RuntimeSession>,
    drops: &Arc<AtomicUsize>,
    slow: bool,
) -> Box<dyn LibraryFunction> {
    Box::new(DropFunction {
        session: Arc::downgrade(session),
        drops: drops.clone(),
        slow,
    })
}
#[test]
fn slow_registration_during_callback_queues_and_never_poison_refusal_or_disposal() {
    watched(
        "slow_registration_during_callback_queues_and_never_poison_refusal_or_disposal",
        || {
            let session = setup();
            let (entered, active) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let wait = Mutex::new(wait);
            session
                .register_function(
                    "Echo".into(),
                    Box::new(Function(move |_| {
                        entered.send(()).unwrap();
                        wait.lock().unwrap().recv().unwrap();
                        Ok(Some("7".into()))
                    })),
                )
                .unwrap();
            let evaluating = session.clone();
            let evaluation = std::thread::spawn(move || evaluating.evaluate().unwrap().json);
            active.recv().unwrap();
            let drops = Arc::new(AtomicUsize::new(0));
            let registering = session.clone();
            let function = implementation(&session, &drops, true);
            let (started, registering_now) = mpsc::channel();
            let registration = std::thread::spawn(move || {
                started.send(()).unwrap();
                registering.register_function("Other".into(), function)
            });
            registering_now.recv().unwrap();
            release.send(()).unwrap();
            assert!(evaluation.join().unwrap().contains("\"value\":7"));
            // Old code refused this registration and its slow destructor raced the
            // callback's flag reset. Queueing now admits it regardless of schedule.
            registration.join().unwrap().unwrap();
            assert!(session.version().is_ok());
            session
                .register_function("Other".into(), implementation(&session, &drops, true))
                .unwrap();
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            session.dispose();
            assert_eq!(drops.load(Ordering::SeqCst), 2);
            assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
        },
    );
}
#[test]
fn long_callback_matches_rust_and_replacement_destructor_never_queues_to_itself() {
    watched(
        "long_callback_matches_rust_and_replacement_destructor_never_queues_to_itself",
        || {
            let session = setup();
            let drops = Arc::new(AtomicUsize::new(0));
            session
                .register_function("Other".into(), implementation(&session, &drops, true))
                .unwrap();
            let ids = Arc::new(Mutex::new(Vec::new()));
            let threads = ids.clone();
            let (entered, active) = mpsc::channel();
            session
                .register_function(
                    "Echo".into(),
                    Box::new(Function(move |_| {
                        threads.lock().unwrap().push(std::thread::current().id());
                        entered.send(()).unwrap();
                        // Deliberately exceed the removed 1s bound. Only output is asserted.
                        std::thread::sleep(Duration::from_millis(1200));
                        Ok(Some("7".into()))
                    })),
                )
                .unwrap();
            let evaluating = session.clone();
            let evaluation = std::thread::spawn(move || evaluating.evaluate().unwrap().json);
            active.recv().unwrap();
            // The worker must finish the callback, then refuse destructor reentry
            // even though no callback is active anymore (review's deadlock case).
            session
                .register_function("Other".into(), implementation(&session, &drops, true))
                .unwrap();
            let schema: serde_json::Value = serde_json::from_str(SCHEMA).unwrap();
            let library = openui_lang::Library::from_json_schema(&schema);
            let program = openui_lang::parse("$n=0\nroot=Box(@Echo())", &library, None);
            let mut functions = openui_runtime::LibraryFunctions::new(&schema["functions"]);
            functions
                .register("Echo", |_| Ok(Some(openui_lang::Value::Num(7.0))))
                .unwrap();
            let expected = openui_runtime::evaluate_tree(
                openui_runtime::TreeInput::new(&program)
                    .with_library(&library)
                    .with_functions(&functions),
                &openui_runtime::Store::new(),
            )
            .to_json()
            .unwrap();
            assert_eq!(evaluation.join().unwrap(), expected);
            assert_eq!(session.evaluate().unwrap().json, expected);
            let ids = ids.lock().unwrap();
            assert_eq!(ids.len(), 2);
            assert_eq!(ids[0], ids[1], "callbacks reuse the session worker");
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            session.dispose();
            assert_eq!(drops.load(Ordering::SeqCst), 2);
        },
    );
}
#[test]
fn external_calls_and_disposal_finish_after_a_callback_returns() {
    watched(
        "external_calls_and_disposal_finish_after_a_callback_returns",
        || {
            let session = setup();
            let (entered, active) = mpsc::channel();
            let (release, wait) = mpsc::channel();
            let wait = Mutex::new(wait);
            session
                .register_function(
                    "Echo".into(),
                    Box::new(Function(move |_| {
                        entered.send(()).unwrap();
                        wait.lock().unwrap().recv().unwrap();
                        Ok(Some("7".into()))
                    })),
                )
                .unwrap();
            let evaluating = session.clone();
            let evaluation = std::thread::spawn(move || evaluating.evaluate().unwrap().json);
            active.recv().unwrap();
            let disposing = session.clone();
            let dispose = std::thread::spawn(move || disposing.dispose());
            release.send(()).unwrap();
            assert!(evaluation.join().unwrap().contains("\"value\":7"));
            dispose.join().unwrap();
            assert!(matches!(session.snapshot(), Err(OpenUiError::Disposed)));
        },
    );
}
#[test]
fn seeded_callbacks_destructors_replacement_disposal_and_concurrent_calls_make_progress() {
    watched(
        "seeded_callbacks_destructors_replacement_disposal_and_concurrent_calls_make_progress",
        || {
            // Several schedules ensure refused registrations destroy implementations
            // on several caller threads as well as on the worker.
            for seed in 1u64..=8 {
                let session = setup();
                let drops = Arc::new(AtomicUsize::new(0));
                let created = Arc::new(AtomicUsize::new(0));
                let start = Arc::new(Barrier::new(4));
                let progress = Arc::new(AtomicUsize::new(0));
                let mut threads = Vec::new();
                for id in 0..4 {
                    let (session, drops, created, start, progress) = (
                        session.clone(),
                        drops.clone(),
                        created.clone(),
                        start.clone(),
                        progress.clone(),
                    );
                    threads.push(std::thread::spawn(move || {
                        let mut rng = seed.wrapping_mul(0x9e3779b97f4a7c15) ^ (id + 1);
                        start.wait();
                        for step in 0..40 {
                            rng ^= rng << 13;
                            rng ^= rng >> 7;
                            rng ^= rng << 17;
                            let result = match rng % 6 {
                                0 | 1 => {
                                    created.fetch_add(1, Ordering::SeqCst);
                                    session.register_function(
                                        if rng & 8 == 0 { "Echo" } else { "Other" }.into(),
                                        implementation(&session, &drops, rng & 16 != 0),
                                    )
                                }
                                2 => session.evaluate().map(|_| ()),
                                3 => session.set("$n".into(), Some(step.to_string())).map(|_| ()),
                                4 => session.snapshot().map(|_| ()),
                                _ => session.version().map(|_| ()),
                            };
                            assert!(
                                result.is_ok() || matches!(result, Err(OpenUiError::Disposed)),
                                "{result:?}"
                            );
                            if id == seed % 4 && step == 30 {
                                session.dispose();
                            }
                            progress.fetch_add(1, Ordering::SeqCst);
                        }
                    }));
                }
                for thread in threads {
                    thread.join().unwrap();
                }
                session.dispose();
                assert_eq!(progress.load(Ordering::SeqCst), 160);
                assert_eq!(drops.load(Ordering::SeqCst), created.load(Ordering::SeqCst));
                assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
            }
        },
    );
}

#[test]
fn refused_callback_destruction_during_thread_local_teardown_does_not_unwind() {
    watched(
        "refused_callback_destruction_during_thread_local_teardown_does_not_unwind",
        || {
            struct OnExit(Arc<RuntimeSession>, Arc<AtomicUsize>);
            impl Drop for OnExit {
                fn drop(&mut self) {
                    assert!(matches!(
                        self.0.register_function(
                            "Echo".into(),
                            implementation(&self.0, &self.1, false)
                        ),
                        Err(OpenUiError::Disposed)
                    ));
                }
            }
            thread_local! {
                static LAST: std::cell::RefCell<Option<OnExit>> = const { std::cell::RefCell::new(None) };
            }
            let session = setup();
            session.dispose();
            let drops = Arc::new(AtomicUsize::new(0));
            let (other, destroyed) = (session.clone(), drops.clone());
            std::thread::spawn(move || {
                // Initialize LAST first, to exercise the order where a dropping
                // scope key would already have been destroyed on this caller.
                LAST.with(|last| {
                    *last.borrow_mut() = Some(OnExit(other.clone(), destroyed.clone()))
                });
                assert!(matches!(
                    other.register_function(
                        "Echo".into(),
                        implementation(&other, &destroyed, false)
                    ),
                    Err(OpenUiError::Disposed)
                ));
            })
            .join()
            .unwrap();
            assert_eq!(drops.load(Ordering::SeqCst), 2);
            assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
        },
    );
}

#[test]
fn sessions_without_functions_keep_queueing_during_thread_local_teardown() {
    watched(
        "sessions_without_functions_keep_queueing_during_thread_local_teardown",
        || {
            struct OnExit(Arc<RuntimeSession>, Arc<AtomicUsize>);
            impl Drop for OnExit {
                fn drop(&mut self) {
                    assert!(self.0.version().is_ok());
                    self.0.dispose();
                    assert!(matches!(self.0.snapshot(), Err(OpenUiError::Disposed)));
                    self.1.fetch_add(1, Ordering::SeqCst);
                }
            }
            thread_local! {
                static LAST: std::cell::RefCell<Option<OnExit>> = const { std::cell::RefCell::new(None) };
            }
            let session = setup(); // No function has ever been registered.
            let done = Arc::new(AtomicUsize::new(0));
            let (other, completed) = (session.clone(), done.clone());
            std::thread::spawn(move || {
                LAST.with(|last| *last.borrow_mut() = Some(OnExit(other.clone(), completed)));
                assert!(other.version().is_ok());
            })
            .join()
            .unwrap();
            assert_eq!(done.load(Ordering::SeqCst), 1);
            assert!(matches!(session.version(), Err(OpenUiError::Disposed)));
        },
    );
}
