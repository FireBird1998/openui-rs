import AppKit

/// Cache the hosted view rather than taking a screenshot of the desktop.
/// Sample in view points, using the actual bitmap dimensions on each axis.
/// Pixel comparisons use sRGB after conversion from the cached bitmap's colour space.
@MainActor struct MountedViewPixels {
    let bitmap: NSBitmapImageRep
    let bounds: CGRect
    let backingScale: CGFloat
    private let operation: String
    private let isFlipped: Bool

    init(capturing view: NSView, for operation: String) throws {
        let bounds = view.bounds
        let backingScale = view.window?.backingScaleFactor ?? 1
        let candidate = bounds.isEmpty ? nil : view.bitmapImageRepForCachingDisplay(in: bounds)
        guard let bitmap = candidate, bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0 else {
            let size = candidate.map { "\($0.pixelsWide)x\($0.pixelsHigh) px" } ?? "no bitmap"
            throw CollectionInputFailure(description:
                "Capturing \(operation) at view bounds \(bounds) pt returned no usable image "
                + "(\(size), backing scale \(backingScale), window frame \(String(describing: view.window?.frame)))")
        }
        view.cacheDisplay(in: bounds, to: bitmap)
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
              let colour = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else {
            throw samplingFailure(at: point, "no sRGB pixel at (\(x), \(y))")
        }
        return colour
    }

    private func samplingFailure(at point: CGPoint, _ reason: String) -> CollectionInputFailure {
        CollectionInputFailure(description: "Sampling \(operation) at \(point) pt in a \(description) image failed (\(reason))")
    }
}
