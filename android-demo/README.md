# Android demo

A small Compose app that runs the core on Android. It streams the seven benchmark samples (`conformance/imported/cases/bench/`) through the core six characters at a time, as a model would send them, and draws each result natively as it arrives. The core is reached through the UniFFI binding (`crates/openui-ffi`), with Kotlin bindings generated from it.

```bash
scripts/android-demo.sh --run
```

This builds the binding for 64-bit ARM, generates the Kotlin bindings, builds the app, installs it on the running emulator or phone, and starts it playing every sample in turn. Without `--run` it only builds. `--serial` picks a device.

It needs:
- the Android SDK and NDK;
- cargo-ndk and the `aarch64-linux-android` Rust target;
- Java 17 or later;
- Gradle 9.4 or later.

The generated bindings, native libraries and copied samples aren't tracked.

It's a demo, not an app: the renderer covers the components the samples use, and draws anything else as a labelled box. Elements still arriving get an amber outline.
