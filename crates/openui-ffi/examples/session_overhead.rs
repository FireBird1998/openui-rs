//! Measurement only: there are no wall-clock assertions in tests.
use openui_ffi::{ComponentLibrary, RuntimeSession};
use std::{hint::black_box, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let library = ComponentLibrary::new(r#"{"$defs":{}}"#.into())?;
    let session = RuntimeSession::new(library, None)?;
    let store = openui_runtime::Store::new();
    for _ in 0..1000 {
        black_box(session.version()?);
    }
    let count = 20_000;
    let start = Instant::now();
    for _ in 0..count {
        black_box(black_box(&store).version());
    }
    let direct = start.elapsed().as_nanos() as f64 / count as f64;
    let start = Instant::now();
    for _ in 0..count {
        black_box(session.version()?);
    }
    let binding = start.elapsed().as_nanos() as f64 / count as f64;
    println!(
        "{count} calls; direct version {direct:.1} ns/call; binding version {binding:.1} ns/call; dispatch overhead {:.1} ns/call",
        binding - direct
    );
    Ok(())
}
