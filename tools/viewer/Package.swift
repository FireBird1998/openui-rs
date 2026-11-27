// swift-tools-version: 6.0
// The viewer: plays OpenUI Lang programs as streams and draws them natively.
// A development tool for the core, built on the OpenUINative package. Build
// it with scripts/viewer.sh, which builds the core's binding first.
import PackageDescription

let package = Package(
    name: "Viewer",
    platforms: [.macOS("27.0")],
    products: [
        .executable(name: "Viewer", targets: ["Viewer"]),
    ],
    dependencies: [
        .package(path: "../../swift/OpenUINative"),
        // Test support only, for the test target below.
        .package(path: "../../swift/SwiftTestSupport"),
    ],
    targets: [
        // Everything but the app's entry point, so tests can reach it.
        .target(name: "ViewerKit", dependencies: [.product(name: "OpenUINative", package: "OpenUINative")],
                resources: [.copy("Samples")]),
        .executableTarget(name: "Viewer", dependencies: ["ViewerKit"]),
        .testTarget(name: "ViewerKitTests", dependencies: [
            "ViewerKit", .product(name: "SwiftTestSupport", package: "SwiftTestSupport"),
        ]),
    ]
)
