---
status: accepted
---

# The Kotlin library is Material 3 on the shared core, for Android only

Decided by the maintainer in spec #257 (2026-11-28), built from #258. This
record says why, and what the first ticket assumed where the spec leaves a
later ticket to decide.

## What happened

An Android developer had nothing in this repository to draw OpenUI Lang
with. The core reached Kotlin through the generated binding, but the only
Kotlin code was `android-demo`, with its own drawing code and no state,
forms, actions or tool calls. On the Mac the same gap was closed by
OpenUINative.

## The decision

OpenUICompose is an Android library module in `kotlin/OpenUICompose`,
package `dev.openui.compose`, Android 14 (API 34) or later. It does for
Jetpack Compose what OpenUINative does for SwiftUI and upstream's
`react-lang` does for React, with public names following OpenUINative's
where Kotlin style allows.

- **The look is Material 3.** The base components are built from Material 3
  and follow the app's Material theme, dark mode included. They are not drawn
  to upstream's sizes and colours, unlike the Swift Intelligent UI library.
  This is a difference in pixels, not in meaning, so it is not a conformance
  difference.
- **Behaviour is the shared core's.** What a program means, and every limit
  and rule that governs it, comes from the Rust core through the generated
  binding, so a program behaves the same on Android as on the Mac and as
  upstream. The Kotlin library holds no evaluation or scheduling rule. Where
  a behaviour rule lives in the Swift binding rather than the core, Kotlin
  copies upstream, with the Swift implementation as the worked example.
- **Android only.** Jetpack Compose on Android, not Compose Multiplatform:
  no desktop or iOS targets.

Compose Multiplatform was rejected: the spec's users are Android apps, and
the Mac already has OpenUINative. Drawing to upstream's pixels was rejected:
a drawn response should match the rest of the Android app it sits in.

## How the binding is kept out of the public API

The Kotlin binding is generated from `crates/openui-ffi` and compiled as part
of the library. `scripts/kotlin-binding.py` makes every top-level declaration
in the generated file `internal`, and the module builds in Kotlin's explicit
API mode, so the compiler refuses any public declaration of the library that
would expose a generated type, and any declaration that doesn't state its
visibility. The generated code is hidden from Kotlin callers only: `internal`
is still public in JVM bytecode, so Java code can call the generated `parse`
or construct a `RuntimeSession`. The maintainer accepts this for now; how to
close it is an open question on the follow-up list (#268).

The file is generated with `java.lang.ref.Cleaner` (Android has it since API
33) rather than Android's `SystemCleaner`, so the same file also runs on an
ordinary JVM, where the JVM tests load the core built for the host.

The generated binding calls the core through JNA, which ships with the
library. JNA's POM offers LGPL-2.1-or-later or Apache-2.0; the maintainer
accepted it by name on 2026-11-28, version 5.17.0, under its Apache-2.0
option, and the licence check's multi-licence allowlist records that ruling.

## What the first ticket assumed

The spec leaves the live program's threading rule to its first live-program
ticket (#260). Until then, a live program is used from one thread of control
(the main thread in an app, the test's thread in a test): `show` is a
suspending function that runs the core's work and decodes the evaluated tree
off the calling thread, and changes `root`, which is Compose state, on the
calling thread when that is done. One show runs at a time; a second waits
for the first. #260 decides the rule and records it here or in its own ADR.

A live program also reports when the core cut what it shows at a limit
(`sizeLimited`, `depthLimited`, as OpenUINative's does), and `ElementView`
draws a notice above the tree when either is set ("Cut at the size limit",
"Cut at the depth limit", the Viewer's words), so a cut drawn that way is
never silent. Drawing a single element directly draws no notice; the caller
reads the two flags.

Errors are an `OpenUIException` with a `kind` (the core's kinds, one to one,
and the library's own) and a message, as OpenUINative's `OpenUIError` is.

## Consequences

The glossary gains **OpenUICompose** and **Android viewer**.
`scripts/kotlin-check.sh` builds and tests the library, and the Kotlin job on
GitHub runs it.
