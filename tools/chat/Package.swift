// swift-tools-version: 6.0
// The chat app: a sandboxed Mac app that asks the Thesys Gateway for answers
// in OpenUI Lang and draws them natively with OpenUINative. Build it with
// scripts/chat.sh, which builds the core's binding first.
import PackageDescription

let package = Package(
    name: "Chat",
    platforms: [.macOS("27.0")],
    products: [
        .executable(name: "Chat", targets: ["Chat"]),
    ],
    dependencies: [
        .package(path: "../../swift/OpenUINative"),
    ],
    targets: [
        // Everything but the app's entry point, so tests can reach it.
        .target(name: "ChatKit", dependencies: [.product(name: "OpenUINative", package: "OpenUINative")],
                resources: [.copy("Library")]),
        .executableTarget(name: "Chat", dependencies: ["ChatKit"]),
        .testTarget(name: "ChatKitTests", dependencies: ["ChatKit"], resources: [.copy("Recordings")]),
    ]
)
