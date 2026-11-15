//! Thread CPU time on the two CI platforms, wall time elsewhere.
#[cfg(any(target_os = "linux", target_vendor = "apple"))]
pub fn now() -> f64 {
    use std::ffi::{c_int, c_long};
    #[repr(C)]
    struct Timespec {
        seconds: c_long,
        nanoseconds: c_long,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: c_int, result: *mut Timespec) -> c_int;
    }
    #[cfg(target_vendor = "apple")]
    const CLOCK: c_int = 16;
    #[cfg(target_os = "linux")]
    const CLOCK: c_int = 3;
    let mut t = Timespec {
        seconds: 0,
        nanoseconds: 0,
    };
    // SAFETY: Timespec has the platform's C long layout and remains writable
    // throughout clock_gettime. No pointer escapes this call.
    assert_eq!(unsafe { clock_gettime(CLOCK, &mut t) }, 0);
    t.seconds as f64 + t.nanoseconds as f64 * 1e-9
}
#[cfg(not(any(target_os = "linux", target_vendor = "apple")))]
pub fn now() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}
