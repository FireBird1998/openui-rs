# 01 — How well does UniFFI work on Android today?

Type: research
Status: open
Blocked by: none

## Question

Can Kotlin on Android call the core through UniFFI comfortably today? Find out:
- the build setup (cross-compiling for Android ABIs, Gradle integration)
- how much a Rust library adds to the APK
- how a streaming parse result, a tree rebuilt after every chunk, gets to Kotlin
- the cost of crossing that boundary per chunk
- any alternatives worth comparing
