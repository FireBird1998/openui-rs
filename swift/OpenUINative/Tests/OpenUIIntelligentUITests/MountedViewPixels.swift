import AppKit
@testable import OpenUIIntelligentUI
import SwiftTestSupport
import Testing

extension NSBitmapImageRep {
    /// The pixel at (x, y) as sRGB. `colorAt` returns the raw components
    /// tagged as generic calibrated (or device) RGB whatever the bitmap's
    /// profile is. A window on a wide-gamut display caches in that display's
    /// profile, so read the components in `colorSpace` before converting.
    func sRGBColor(x: Int, y: Int) -> NSColor? {
        guard let raw = colorAt(x: x, y: y) else { return nil }
        var components = [CGFloat](repeating: 0, count: raw.numberOfComponents)
        raw.getComponents(&components)
        guard components.count == colorSpace.numberOfColorComponents + 1 else { return nil }
        return NSColor(colorSpace: colorSpace, components: components, count: components.count).usingColorSpace(.sRGB)
    }
}

extension ThemeColor {
    /// The token's declared sRGB value, independent of any renderer.
    var sRGB: NSColor { NSColor(srgbRed: red, green: green, blue: blue, alpha: alpha) }
}

/// The colour space a test window draws and caches in. Window pixel tests run
/// in both, so a wide-gamut Mac and an sRGB runner check the same thing.
enum WindowColorSpace: String, CaseIterable, Sendable, CustomTestStringConvertible {
    case sRGB, displayP3

    var colorSpace: NSColorSpace { self == .sRGB ? .sRGB : .displayP3 }
    var testDescription: String { rawValue }
}

/// Cache the hosted view rather than taking a screenshot of the desktop.
/// Sample in view points, using the actual bitmap dimensions on each axis.
@MainActor struct MountedViewPixels {
    let bitmap: NSBitmapImageRep
    let bounds: CGRect
    let backingScale: CGFloat
    private let operation: String
    private let isFlipped: Bool

    /// Pass the window's forced colour space to check the capture used it.
    init(capturing view: NSView, for operation: String, in expected: WindowColorSpace? = nil) throws {
        let bounds = view.bounds
        let backingScale = view.window?.backingScaleFactor ?? 1
        NativeTestWatchdog.beginWaiting("AppKit to prepare a bitmap of \(operation)")
        defer { NativeTestWatchdog.finishedWaiting() }
        let candidate = bounds.isEmpty ? nil : view.bitmapImageRepForCachingDisplay(in: bounds)
        guard let bitmap = candidate, bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0 else {
            let size = candidate.map { "\($0.pixelsWide)x\($0.pixelsHigh) px" } ?? "no bitmap"
            throw CollectionInputFailure(description:
                "Capturing \(operation) at view bounds \(bounds) pt returned no usable image "
                + "(\(size), backing scale \(backingScale), window frame \(String(describing: view.window?.frame)))")
        }
        NativeTestWatchdog.waitingFor("AppKit to draw \(operation)")
        view.cacheDisplay(in: bounds, to: bitmap)
        if let expected, bitmap.colorSpace.cgColorSpace?.name == nil
            || bitmap.colorSpace.cgColorSpace?.name != expected.colorSpace.cgColorSpace?.name {
            throw CollectionInputFailure(description:
                "Capturing \(operation) in \(expected) returned a \(bitmap.colorSpace) bitmap "
                + "(window colour space \(String(describing: view.window?.colorSpace)))")
        }
        self.bitmap = bitmap
        self.bounds = bounds
        self.backingScale = backingScale
        self.operation = operation
        isFlipped = view.isFlipped
    }

    var scaleX: CGFloat { CGFloat(bitmap.pixelsWide) / bounds.width }
    var scaleY: CGFloat { CGFloat(bitmap.pixelsHigh) / bounds.height }

    var description: String {
        "\(bitmap.pixelsWide)x\(bitmap.pixelsHigh) px for \(bounds) pt "
            + "(image scale \(scaleX)x\(scaleY), backing scale \(backingScale), alpha \(bitmap.hasAlpha))"
    }

    func pixel(at point: CGPoint) throws -> NSColor {
        guard bounds.contains(point) else {
            throw samplingFailure(at: point, "point is outside the captured bounds")
        }
        let x = Int((point.x - bounds.minX) * scaleX)
        let row = Int((point.y - bounds.minY) * scaleY)
        let y = isFlipped ? row : bitmap.pixelsHigh - 1 - row
        guard (0..<bitmap.pixelsWide).contains(x), (0..<bitmap.pixelsHigh).contains(y),
              let colour = bitmap.sRGBColor(x: x, y: y) else {
            throw samplingFailure(at: point, "no sRGB pixel at (\(x), \(y))")
        }
        return colour
    }

    private func samplingFailure(at point: CGPoint, _ reason: String) -> CollectionInputFailure {
        CollectionInputFailure(description: "Sampling \(operation) at \(point) pt in a \(description) image failed (\(reason))")
    }
}
