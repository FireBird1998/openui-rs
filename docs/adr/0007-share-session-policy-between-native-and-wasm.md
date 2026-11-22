# 0007: Share session policy in openui-session

Status: accepted for #42.

`openui-session` depends on the parser and runtime, and neither depends on it.
It owns session policy: retained programs/roots, lazy action paths, opaque
capabilities, generations, pending mutation eviction/release, JSON admission,
error translation, state/form/binding operations and limits. This avoids making
binding lifecycle policy part of openui-runtime's public evaluator API. UniFFI
retains its worker, synchronous transport, Send + Sync wrappers and exported
records. WebAssembly calls the same core on its calling thread.

Core continuation tokens send release notifications to a core-owned control
queue. Native additionally supplies a weak transport wakeup, so only its worker
drops a pending runtime continuation, including when a cleaner drops a token.
Wasm exported tokens and action outputs have a final field holding a weak
session cleanup guard. After their core token drops, that guard drains releases
on the same thread, without calling JavaScript or retaining the session. A busy
Rust borrow defers draining to the next operation. Disposal drops all entries.

Wasm exports actual wasm-bindgen objects. No numeric IDs or JSON constructors
are provided. Generated private `__*` helpers and memory/pointer fields are
implementation internals, outside the application capability interface, just
as generated native unsafe/testing constructors are (ADR 0006).
The generated package installs a JavaScript guard (crates/openui-wasm/js/
package-guards.js) around every public entry point. Its rules:

1. Every public member of every generated class, string- or symbol-keyed and
   statics included, is replaced by a guarded one. wasm-bindgen's
   `[Symbol.dispose] = free` alias becomes the guarded free, so `using` frees
   through the guard. The guard describes the surface member by member; a
   generated member it does not describe, or a described member the package
   lacks, stops the module from loading (scripts/wasm.sh loads it once).
2. A guarded call refuses reentry (Worker, without entering Rust), converts
   every argument to its primitive type inside the guard, checks the receiver
   and capability arguments (minted by the package, of the right class, not
   freed), and only then calls the generated method. Booleans, strings and
   bigints of the wrong type are refused, never coerced, so no `valueOf`,
   `toString` or `length` runs between those checks and the wasm call. The
   one conversion that can run application code, reading an action path's
   elements, runs while the guard is active, where every free refuses.
3. A freed object used in any way throws Disposed; freeing again is a no-op.

This closes the interval before Rust's try_borrow_mut guard is acquired; a
mutable exported Rust receiver alone would let wasm-bindgen's recursive-use
exception poison its borrow state. Apart from registered host-function callbacks (ADR 0009), the core calls no
JavaScript while it holds an exported object: errors leave Rust as `openui:<Kind>\n<message>`
strings, records and event lists as JSON text, and action paths enter as JSON
text. The guard builds the Error (defining `kind`, never assigning it) and
parses records after Rust returns, with intrinsics captured at load, and
rebuilds the generated string-cast and externref helpers on those intrinsics.
A throwing `Object.prototype.kind` setter or a patched `JSON.parse` therefore
cannot unwind through a borrowed Rust frame and leave the object unfreeable.
The built-package tests check the module imports nothing else. Generated
`__*` helpers refuse outside a guarded call, pointer fields refuse a new
value, and the guarded prototypes and classes are frozen. That stops mistakes;
it is not a boundary against hostile code in the same realm.

The guard's guarantee is about the public interface used by the embedding
application's own code: argument values, handle misuse, frees in any order
and reentry cannot make Rust use a freed object or change state from a
refused call. Program text, JSON and model output never reach the guard as
code. JavaScript running in the same realm is trusted. Code that deliberately
reaches for the private `__*` helpers or pointer fields from inside a guarded
call (for example from a getter on an action path), or that replaces
intrinsics the generated glue uses (`String.prototype.charCodeAt`,
`Object.create`, an `Object.prototype.__wbg_ptr` accessor), before or after
the package loads, can forge or free objects and crash the instance. Such
code could equally call the module's raw exports. A page that runs untrusted
JavaScript must isolate it from this package (a separate realm or worker).

PreparedReferenceValues::owned_with_limits owns its source and opaque core
measurement receipts. Wasm prepares once on setReferenceValues and reuses the
same preparation for all operations. Native retains its lexical-loop reuse.
No unsafe lifetime extension, leaked input or caller-asserted measurements are
needed; owned and borrowed preparations have identical credit and cut flags.

Library functions now cross a JSON-only callback bridge (ADR 0009). Generated
wasm packages require the guard appended by scripts/wasm.sh; hand-running
wasm-bindgen alone does not produce the supported application package.

(2026-11-22: since #145, after a session call throws, a private `&mut self`
wasm-bindgen method checks that the call released its borrow; if it did not,
the guard retires the session as Disposed, releases the wrapper through
wasm-bindgen's finalizer path and stops its driver; Rust allocations of the
abandoned frame stay until the instance is discarded.)
