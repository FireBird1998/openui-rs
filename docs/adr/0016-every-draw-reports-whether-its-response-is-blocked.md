---
status: accepted
---

# Every draw reports whether its response is blocked

Until #118 the session ignored clicks and Retry while a response bundle had
not settled, but told no driver, so a field looked usable and did nothing.
The maintainer decided on 2026-11-20 (#115, decision 10) that the session
tells drivers and they disable inputs, as upstream's renderer does.

## The flag is the renderer's `blocked`, on every evaluation

Upstream's hook computes `blocked = isStreaming || !bundle.complete ||
!!bundle.error` and hands it to every component as the context's
`isStreaming` (`useOpenUIState.ts` lines 92 and 483 at the pinned commit).
It hands over `blocked`, not `bundleBlocked`: a streaming bare program is
blocked too, although the hook still takes its clicks. So `EvaluationOutput`
gains `blocked` in the session and both bindings, computed by the same
`prepare` that sets the click gate, from the response state the draw was
given. A draw without response state reports false.

Rejected:

- A session getter for the flag. Drivers read the response's state from
  each draw already (`isQueryLoading`, `parserErrorJson`), the flag belongs
  to the drawing it was computed for, and a getter would be one more method
  on both bindings and the package guard.
- Reporting `bundleBlocked`. It would leave a streaming bare program looking
  usable, which upstream does not.

The flag only reports. The session's gate keeps reading its own state, so a
driver that ignores the flag (or a click that arrives anyway) changes
nothing it accepts.

## OpenUINative reads it for bundles only

`LiveProgram` disabled controls while `isStreaming` (the app's request, a
stream preview or a host suspension) or while its own copy of the bundle
rule said so. The copy is gone: the live program now publishes the
session's flag from each evaluation and uses it for a response bundle. For a
bare program the session's flag is the streaming request at the last draw,
and the app can clear that request without a redraw; `isStreaming` already
reports it as it is now, so a bare program keeps being gated by
`isStreaming` alone. `isBlocked` is the union, and `acceptsInteractions` is
its negation.

## The package counts response state in bytes

The WebAssembly guard counted the response state against `maxJsonBytes` in
UTF-16 units, and the scripts JSON it builds for Rust (names escaped) not at
all, so that JSON could reach about six times the limit before Rust's own
count refused the names (#108). It now counts what Rust counts, in UTF-8
bytes as the strings cross, and also holds the scripts JSON to
`maxJsonBytes` bytes. A text's UTF-16 length is checked before it is
measured, so no text longer than the limit is walked. Every input the guard
refused before it still refuses. Near the limit it refuses some states the
native binding accepts (the JSON's quotes, commas and escapes count there);
Rust's count in `prepare` stays the bound both bindings share.
