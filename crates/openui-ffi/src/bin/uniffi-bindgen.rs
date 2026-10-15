//! UniFFI's bindings generator, run from the workspace so nothing is
//! installed globally:
//!
//!   cargo run -p openui-ffi --features bindgen --bin uniffi-bindgen -- generate …
fn main() {
    uniffi::uniffi_bindgen_main()
}
