/// Resource bounds for Intelligent UI photos. See the Image limits table in CONFORMANCE.md.
enum PhotoLimits {
    // Keep per-window load work, retained rasters, and pending continuations bounded.
    static let concurrentLoads = 4
    static let windowRasterBytes = 32 * 1024 * 1024
    static let decodedImageBytes = 8 * 1024 * 1024
    static let queuedLoads = 4_096

    // Limit buffered data and the lifetime and reach of each request.
    static let bodyBytes = 8 * 1024 * 1024
    static let inactivityTimeout = 5.0
    static let resourceTimeout = 15.0
    static let redirects = 20

    // Bound decoder work and downsample the retained raster.
    static let sourceAxis = 8_192
    static let sourcePixels = 16_777_216
    static let thumbnailAxis = 1_024

    // Limit animation work and storage, and normalize extreme timings.
    static let animationFrames = 60
    static let animationBytes = 8 * 1024 * 1024
    static let shortFrameDelay = 0.02
    static let replacementFrameDelay = 0.1
    static let longFrameDelay = 60.0
    static let animationRepeats = 1...65_535

    // Bound XML parsing, reference names, and the SVG viewport transform.
    static let svgInputBytes = 256 * 1024
    static let svgNodes = 512
    static let svgDepth = 32
    static let svgIDBytes = 128
    static let svgViewBoxMagnitude = 1_000_000.0
    static let svgStyleDeclarations = 53

    // Bound AppKit's clip-view observer walk. Visibility still uses the actual visible rect.
    static let viewportAncestors = 256
}
