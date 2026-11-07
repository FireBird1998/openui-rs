//! Check the fatal path in a disposable copy of this test binary.
#[global_allocator]
static ALLOCATOR: test_alloc::Allocator = test_alloc::Allocator::with_cap(test_alloc::DEFAULT);

#[test]
fn over_allocation_names_the_test_cap_and_peak() {
    if std::env::var_os("OPENUI_ALLOCATION_SELF_TEST").is_some() {
        std::hint::black_box(vec![0_u8; test_alloc::DEFAULT + 1]);
        panic!("allocation unexpectedly survived");
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "over_allocation_names_the_test_cap_and_peak"])
        .env("OPENUI_ALLOCATION_SELF_TEST", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101));
    let message = String::from_utf8(output.stderr).unwrap();
    let prefix = format!(
        "allocation cap exceeded: test over_allocation_names_the_test_cap_and_peak; cap {} bytes; peak ",
        test_alloc::DEFAULT
    );
    let peak = message
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|rest| rest.strip_suffix(" bytes"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| panic!("missing allocation diagnostic: {message}"));
    assert!(peak > test_alloc::DEFAULT, "{message}");
}

#[test]
fn alignment_reallocation_and_cross_thread_free() {
    #[repr(align(4096))]
    struct Aligned([u8; 4096]);
    let value = Box::new(Aligned([7; 4096]));
    assert_eq!(value.0[0], 7);
    assert_eq!((&*value as *const Aligned as usize) % 4096, 0);
    let before = test_alloc::peak();
    let mut bytes = vec![1_u8; 8192];
    bytes.resize(32768, 2);
    assert!(test_alloc::peak() >= before + 32768);
    std::thread::spawn(move || {
        assert_eq!(bytes[0], 1);
        assert_eq!(bytes[32767], 2);
        drop((bytes, value));
    })
    .join()
    .unwrap();
    // The owner record also survives its thread, not just a concurrent free.
    let returned = std::thread::spawn(|| vec![3_u8; 8192]).join().unwrap();
    assert_eq!(returned[8191], 3);
    drop(returned);
}
