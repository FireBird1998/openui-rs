# Session host functions cross JSON and refuse reentry

Status: accepted.

Session callbacks reuse LibraryFunctions and TreeInput::with_functions. JSON
crossing cannot reconstruct custom authority. The runtime continues to own
schema validation, argument caching, defaults, diagnostics and execution order.
Registration is per session and bounded; replacement is atomic. Both bindings
use the same core adapter, and source/generated-interface checks cover it.

Native sessions lazily create one long-lived callback thread after the first
registration passes shared-core admission preflight, and join it after worker
teardown at disposal or final drop. Sessions without registrations create no callback thread.
The worker sends each invocation and waits synchronously, without a timeout or
per-invocation thread. A correct synchronous implementation must give the Rust
runtime's result independently of elapsed time. Timeouts that substitute null
were rejected: they change output, and Rust cannot kill outstanding host code.

This replaces direct invocation on the session worker (#65). The core can grow
its stack; Android and iOS start every walk on such a segment (ADR 0002). Entering
a foreign runtime there is unsafe: Android's ART aborts while attaching the Rust
worker because its stack pointer is outside the thread's own stack. The callback
thread starts on an ordinary OS thread stack and performs no core walks before
entering host code. It owns the foreign implementations. Worker closures carry
only private allocation-identity keys and a sender; invocation and final release
both cross the same synchronous channel. Successful replacement and ordinary
registration refusal release complete before acknowledgement. A registration's caller retains its
implementation in a shared handoff slot until the callback thread takes it.
Name/limit refusals create no callback thread; their implementations, failed
spawn/send handoffs and unexecuted queued registrations are released on the
registering caller under its reentry fence. Only the callback thread may take
the slot, so a failed worker cannot release foreign code while discarding jobs.
As corrected in #72, first-registration preflight uses a temporary empty core
with the same library and limits. Thread creation precedes live admission, so a
failed start returns Worker without changing the session and can be retried.
An admitted failed handoff still revokes the session rather than exposing a
partially installed replacement. A reclaimed handoff receives an explicit error;
the callback loop keeps serving jobs.
Callback transport loss or unexpected callback-thread exit records a sticky
Worker error, including for later operations that use no functions. An operation that detects callback
transport failure returns Worker too; host throws and caught host/destructor panics
remain ordinary function failures. Safe host code has no way to terminate the
callback thread: these terminal paths are tested with infrastructure injection.
Normal shutdown closes the callback mailbox deliberately and remains Disposed.
Worker teardown releases registrations before closing and joining the callback
thread, including after a worker panic.
No evaluator or stack-growth policy changes are needed.

A worker must never queue a request to itself. Every same-session operation on
its worker or callback thread refuses before transport locking, including host
invocation and replacement/disposal destructors. Refused jobs can destroy
implementations on native caller threads too: a scoped thread-local stack of
private per-session owner tokens fences that destruction. Commands carry the
calling scope's ancestry to the destination worker and callback thread, including
shutdown destruction. Queued registrations discarded after worker failure
release under their registering caller's original scope. Thus A's function may
call B, but B's function or destructor
calling A refuses with Worker("session already in use") rather than queueing
behind A. Scopes restore on return and unwind. No shared flag is saved/restored
across threads; independent destructors cannot overwrite another scope's exit or
poison a session. The TLS key has no destructor; its storage is released when
empty so scopes remain usable during host thread-local destruction too.
Panics are caught at the invocation/destruction boundary on the thread entering
foreign code. Native dispose is infallible and cleanly refuses such reentry.

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
