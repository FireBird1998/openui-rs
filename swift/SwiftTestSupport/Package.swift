// swift-tools-version: 6.0
// SwiftTestSupport: what the repository's Swift test targets share, the
// per-case watchdog (NativeTestWatchdog) and the clock tests move
// (ManualClock). Test-only: OpenUINative's, the viewer's and the chat app's
// test targets depend on it through path dependencies, and no app or library
// target may (scripts/check-chat-linkage.py rejects any unapproved module in
// the Chat executable). It is its own package so that no shipping package
// lists it as a product.
import PackageDescription

let package = Package(
    name: "SwiftTestSupport",
    platforms: [.macOS("27.0")],
    products: [
        .library(name: "SwiftTestSupport", targets: ["SwiftTestSupport"]),
    ],
    targets: [
        // The watchdog's thread-backtrace capture, in C.
        .target(name: "TestThreadBacktraces"),
        .target(name: "SwiftTestSupport", dependencies: ["TestThreadBacktraces"]),
    ]
)
