import Foundation
import ImageIO

struct PhotoSize: Equatable, Sendable {
    let width: Int
    let height: Int
    var cover: Bool

    init?(points: CGSize, scale: CGFloat, cover: Bool = false) {
        let width = points.width * scale
        let height = points.height * scale
        guard width.isFinite, height.isFinite, width > 0, height > 0 else { return nil }
        self.width = Int(min(Double(PhotoLimits.thumbnailAxis), ceil(width)))
        self.height = Int(min(Double(PhotoLimits.thumbnailAxis), ceil(height)))
        self.cover = cover
    }

    func longestEdge(width: Int, height: Int, transposed: Bool = false, upscale: Bool = false) -> Int {
        let x = Double(self.width) / Double(transposed ? height : width)
        let y = Double(self.height) / Double(transposed ? width : height)
        let fitScale = cover ? max(x, y) : min(x, y)
        let scale = upscale ? fitScale : min(1, fitScale)
        return Int(min(Double(PhotoLimits.thumbnailAxis), ceil(Double(max(width, height)) * scale)))
    }
}

struct PhotoRaster: Sendable {
    let first: CGImage
    let animation: Animation?
    let bytes: Int

    struct Frame: Sendable {
        let image: CGImage
        let end: TimeInterval
    }

    struct Animation: Sendable {
        let frames: [Frame]
        let duration: TimeInterval
        let playCount: Int?

        func frame(at elapsed: TimeInterval) -> CGImage? {
            if let playCount, elapsed >= Double(playCount) * duration {
                return frames.last?.image
            }
            let phase = max(0, elapsed).truncatingRemainder(dividingBy: duration)
            return frames.first { phase < $0.end }?.image
        }
    }

    private struct Timing {
        let dictionary: CFString
        let delay: CFString
        let loopCount: CFString

        static func forType(_ type: String) -> Self? {
            switch type {
            case "com.compuserve.gif":
                Self(dictionary: kCGImagePropertyGIFDictionary, delay: kCGImagePropertyGIFUnclampedDelayTime,
                     loopCount: kCGImagePropertyGIFLoopCount)
            case "public.png":
                Self(dictionary: kCGImagePropertyPNGDictionary, delay: kCGImagePropertyAPNGUnclampedDelayTime,
                     loopCount: kCGImagePropertyAPNGLoopCount)
            case "org.webmproject.webp":
                Self(dictionary: kCGImagePropertyWebPDictionary, delay: kCGImagePropertyWebPUnclampedDelayTime,
                     loopCount: kCGImagePropertyWebPLoopCount)
            case "public.heic", "public.heics":
                Self(dictionary: kCGImagePropertyHEICSDictionary, delay: kCGImagePropertyHEICSUnclampedDelayTime,
                     loopCount: kCGImagePropertyHEICSLoopCount)
            default: nil
            }
        }
    }

    private static let rasterFormats: Set<String> = [
        "public.avif", "public.avis", "com.microsoft.bmp", "com.compuserve.gif",
        "public.heic", "public.heics", "public.heif", "com.microsoft.ico",
        "public.jpeg", "public.jpeg-2000", "public.jpeg-xl", "public.png",
        "public.tiff", "org.webmproject.webp",
    ]

    static func decode(_ data: Data, size: PhotoSize) -> PhotoRaster? {
        let metadataOptions = [kCGImageSourceShouldCache: false] as CFDictionary
        if let source = CGImageSourceCreateWithData(data as CFData, metadataOptions),
           let identifier = CGImageSourceGetType(source), rasterFormats.contains(identifier as String) {
            return decodeRaster(source, type: identifier as String, size: size, metadataOptions: metadataOptions)
        }
        return PhotoSVG.decode(data, size: size).map { still($0) }
    }

    private static func still(_ image: CGImage) -> Self {
        Self(first: image, animation: nil, bytes: image.bytesPerRow * image.height)
    }

    private static func decodeRaster(_ source: CGImageSource, type: String, size: PhotoSize,
                                     metadataOptions: CFDictionary) -> Self? {
        guard let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, metadataOptions) as? [String: Any],
              let pixels = dimensions(properties), let first = decodeFrame(source, 0, size: size, properties: properties,
                                                                           pixels: pixels) else { return nil }
        let fallback = still(first)
        let count = CGImageSourceGetCount(source)
        guard (2...PhotoLimits.animationFrames).contains(count),
              pixels.width * pixels.height <= PhotoLimits.sourcePixels / count,
              let timing = Timing.forType(type) else { return fallback }

        let container = CGImageSourceCopyProperties(source, metadataOptions) as? [String: Any]
        let format = container?[timing.dictionary as String] as? [String: Any]
        let repeats = format?[timing.loopCount as String] as? Int
        // GIF expresses repeats after the initial play. APNG and WebP store total plays.
        let playCount = repeats.flatMap { value -> Int? in
            guard value != 0 else { return nil }
            let clamped = max(PhotoLimits.animationRepeats.lowerBound, min(PhotoLimits.animationRepeats.upperBound, value))
            return clamped + (type == "com.compuserve.gif" ? 1 : 0)
        }
        var duration: TimeInterval = 0
        var frames: [Frame] = []
        var bytes = 0
        for index in 0..<count {
            guard !Task.isCancelled,
                  let properties = CGImageSourceCopyPropertiesAtIndex(source, index, metadataOptions) as? [String: Any],
                  let framePixels = dimensions(properties),
                  framePixels.width * framePixels.height <= PhotoLimits.sourcePixels / count,
                  let format = properties[timing.dictionary as String] as? [String: Any],
                  let delay = format[timing.delay as String] as? Double, delay.isFinite,
                  let image = index == 0 ? first : decodeFrame(source, index, size: size,
                                                               properties: properties, pixels: framePixels)
            else { return fallback }
            bytes += image.bytesPerRow * image.height
            guard bytes <= PhotoLimits.animationBytes else { return fallback }
            duration += delay < PhotoLimits.shortFrameDelay ? PhotoLimits.replacementFrameDelay
                : min(delay, PhotoLimits.longFrameDelay)
            frames.append(Frame(image: image, end: duration))
        }
        return Self(first: first,
                    animation: Animation(frames: frames, duration: duration, playCount: repeats == nil ? 1 : playCount),
                    bytes: bytes)
    }

    private static func dimensions(_ properties: [String: Any]) -> (width: Int, height: Int)? {
        guard let width = properties[kCGImagePropertyPixelWidth as String] as? Int,
              let height = properties[kCGImagePropertyPixelHeight as String] as? Int,
              (1...PhotoLimits.sourceAxis).contains(width), (1...PhotoLimits.sourceAxis).contains(height),
              width * height <= PhotoLimits.sourcePixels else { return nil }
        return (width, height)
    }

    private static func decodeFrame(_ source: CGImageSource, _ index: Int, size: PhotoSize,
                                    properties: [String: Any], pixels: (width: Int, height: Int)) -> CGImage? {
        let orientation = properties[kCGImagePropertyOrientation as String] as? Int ?? 1
        let maxSize = size.longestEdge(width: pixels.width, height: pixels.height,
                                       transposed: (5...8).contains(orientation))
        let options = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxSize,
            kCGImageSourceShouldCacheImmediately: true,
        ] as CFDictionary
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options),
              (1...PhotoLimits.thumbnailAxis).contains(image.width), (1...PhotoLimits.thumbnailAxis).contains(image.height)
        else { return nil }
        if image.bitsPerPixel == 32, image.bytesPerRow * image.height <= PhotoLimits.decodedImageBytes { return image }
        guard let context = bitmap(width: image.width, height: image.height) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return context.makeImage()
    }

    static func bitmap(width: Int, height: Int) -> CGContext? {
        guard let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) else { return nil }
        return CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                         space: colorSpace, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    }
}
