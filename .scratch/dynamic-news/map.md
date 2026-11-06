# Map: Dynamic News v1

Label: wayfinder:map

## Destination

A spec for Dynamic News v1, ready to hand to build sessions: a showcase Android app whose story screens are written by an LLM in OpenUI Lang, parsed on the phone by the core, and drawn with Jetpack Compose.

## Notes

- **Parked until the core can stream.** You decided on 2026-10-06 to build the core first, under its own spec (`../openui-lang/spec.md`). This map picks up once openui-lang ticket 07 (streaming parse matches) is done. We design the app together first, then build it.
- **Settled while charting (2026-10-06):**
  - The core is the main project, and Dynamic News is its first real user. App features that don't exercise the core are cut.
  - The core is a personal project in this repo, built so it could be upstreamed to OpenUI later.
  - The app is a sideloaded or open-source showcase, not a Play Store product.
  - The core covers the language layer only (parse, stream, validate, prompt).
  - This map lives in `.scratch/`.
- **Skills:** /grilling and /domain-modeling for discussion tickets, /prototype for sketches, /research for research tickets.
- **Glossary:** `GLOSSARY.md` at the repo root (named `CONTEXT.md` until 2026-11-07).
- **Research tickets** (01–03) haven't been started. Start them when this map comes off the shelf.
- **Mac viewer first (decided 2026-10-15):** before the testing phase (`../testing/spec.md`) and before this map comes off the shelf, a small SwiftUI Mac app that draws programs natively as they stream, calling the core through UniFFI. It is both a playground for the core and a rehearsal for this app's binding: what it learns about what crosses the boundary per chunk feeds tickets 01 and 06 here. Not a Mac version of Dynamic News.

## Decisions so far

<!-- one line per resolved ticket: [title](issues/NN-slug.md) — gist -->

## Not yet specified

- The backend's shape, if there is one, and whether it checks model output with the core before sending it.
- The feed screen: what the list of stories looks like, and how a reader gets from it to a story screen.
- What a reader sees when the model's output is broken or stops halfway.
- How to test the app: screenshot tests, or replaying recorded streams through the real renderer.
- How the app gets installed: sideloaded APK, GitHub releases or F-Droid.
- Theming and dark mode.
- Drawing charts in Compose.
- Offline reading and caching story screens.

## Out of scope

- iOS app.
- Python bindings, and any binding besides Kotlin and the Mac viewer's Swift one.
- Play Store release.
- Evaluator, state and queries in the core.
- Replacing the web parser with WebAssembly.
- Getting OpenUI to adopt the core.
