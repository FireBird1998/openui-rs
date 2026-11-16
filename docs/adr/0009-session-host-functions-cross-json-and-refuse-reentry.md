# Session host functions cross JSON and refuse reentry

Status: accepted.

Session callbacks reuse LibraryFunctions and TreeInput::with_functions. JSON
crossing cannot reconstruct custom authority. The runtime continues to own
schema validation, argument caching, defaults, diagnostics and execution order.
Registration is per session and bounded; replacement is atomic. Both bindings
use the same core adapter, and source/generated-interface checks cover it.

Native callbacks execute directly on the session worker without a time bound or
per-call thread. A correct synchronous implementation must give the Rust runtime's
result independently of elapsed time. Timeouts that substitute null were rejected:
they change output, and Rust cannot kill the outstanding host thread anyway.

A worker must never queue a request to itself. Every same-session operation on
that worker therefore refuses before transport locking, including host invocation
and replacement/disposal destructors. Refused jobs can destroy implementations
on caller threads too: a scoped thread-local stack of private per-session owner
tokens fences that destruction. No shared flag is saved/restored across threads;
independent destructors cannot overwrite another scope's exit or poison a session.
The TLS key has no destructor; its stack storage is released explicitly when
empty, so scopes remain usable during host thread-local destruction too.
Panics are caught at the invocation/destruction boundary. Native dispose is
infallible and cleanly refuses only such reentry.

Other threads retain the existing queueing contract. Submission releases the
transport mutex before waiting; disposal closes submission, queues Stop after
accepted commands, and joins outside that lock. A condition variable preserves
concurrent disposal joins. Foreign callback destruction never holds a transport
lock. A host callback or destructor that waits for another thread to call this
same session creates its own deadlock: the call queues behind that host code.
Likewise, host code that never returns keeps its caller waiting. These are the
ordinary responsibilities of a synchronous implementation, not binding timeouts.

Function failures follow Rust's diagnostics/null and continuing-action semantics,
as pinned in the public binding tests and described in runtime-binding.md. No
whole-action rollback or callback-quota refusal is promised by this contract.

Wasm stores JavaScript callbacks in a thread-local registry. Core closures keep
private registration keys, never JS values or JSON authority. Each private key
is owned by an Arc allocation; the registry removes it before that allocation is
freed. Refused registrations retain no serial-counter or free-slot history. The
registry borrow ends before host invocation. Disposal releases registrations.
The package guard stays active around callbacks; generated catch imports contain
throws and every public reentrant call refuses before wasm lowering.

JavaScript callbacks are synchronous too: the host must return for progress.
Replaying an evaluation after yielding was rejected because it changes call
counts and host effects. Same-realm private-helper hardening (#60) stays open.
Neither binding can roll back external host effects.
