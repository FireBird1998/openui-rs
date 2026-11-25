import Foundation

public struct SwiftTestLimits: Decodable, Sendable {
    public static let shared = try! JSONDecoder().decode(
        Self.self, from: Data(contentsOf: Bundle.module.url(forResource: "Limits", withExtension: "json")!)
    )

    public let nativeSeconds: Int
    public let ordinarySeconds: Int
    public let stressSeconds: Int
    public let chartPollSeconds: Int
    let exitChildSeconds: Int
    public let exitParentMinutes: Int
    let captureSeconds: Int
    public let schedulingSeconds: Int
    let chartStressVariable: String
    let carouselRepetitionVariables: [String]

    public var chartStress: Bool { ProcessInfo.processInfo.environment[chartStressVariable] == "1" }

    public var carouselStress: Bool {
        carouselRepetitionVariables.contains { carouselRepetitions($0) > 1 }
    }

    public func carouselRepetitions(_ variable: String) -> Int {
        Self.repetitionCount(ProcessInfo.processInfo.environment[variable])
    }

    // Match scripts/check-swift-tests.py: optional +, ASCII decimal, signed 64-bit
    // range, and a minimum of one. Leading zeroes do not affect the range.
    public static func repetitionCount(_ input: String?) -> Int {
        max(1, Int64(input ?? "").flatMap(Int.init(exactly:)) ?? 1)
    }
}
