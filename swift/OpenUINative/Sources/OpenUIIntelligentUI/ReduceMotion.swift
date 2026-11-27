import SwiftUI

// The one Reduce Motion value the library's views read. It follows the
// system's accessibilityReduceMotion unless a test pins it: SwiftUI makes
// accessibilityReduceMotion read-only, and GitHub's macOS runners turn
// Reduce Motion on, so without this a test could only ever take whichever
// path its machine's setting picks.
//
// Both members are internal. Only this module and `@testable` imports can
// read or set them, and nothing public exposes them.
extension EnvironmentValues {
    /// Set only by tests. Nil, the default, follows the system.
    @Entry var reduceMotionOverride: Bool? = nil

    /// Whether to take the reduced path. Every Reduce Motion branch reads
    /// this and never `accessibilityReduceMotion` directly.
    var reducesMotion: Bool {
        ReduceMotion.resolve(override: reduceMotionOverride, system: accessibilityReduceMotion)
    }
}

enum ReduceMotion {
    /// The system's value unless a test pinned one.
    static func resolve(override: Bool?, system: Bool) -> Bool {
        override ?? system
    }
}
