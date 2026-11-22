//! Exports the wasm shadow stack pointer, `__stack_pointer`, so that the
//! package guard can put it back after an exception escapes a wasm frame and
//! skips the epilogues that would have (#208).

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        println!("cargo::rustc-link-arg-cdylib=--export=__stack_pointer");
    }
}
