// The theme against upstream's own source: conformance/real-libraries/
// intelligent-ui-theme.json, written by reference/intelligent-ui-theme.ts
// from upstream's tokens.ts, defaultThemes.ts and react-components'
// defaultTheme.ts at the library pin. Every token, resolved in light and in
// dark as a page with no overrides resolves it, must equal this library's.
import Foundation
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftUI
import Testing

private let repository: URL = {
    var directory = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { directory.deleteLastPathComponent() }
    return directory
}()

func themeFixture() throws -> JSONValue {
    let url = repository.appendingPathComponent("conformance/real-libraries/intelligent-ui-theme.json")
    return try JSONValue.parse(String(contentsOf: url, encoding: .utf8))
}

/// A token's value, in the form the fixture can be read into.
private enum TokenValue {
    case color(ThemeColor)
    case shadow([ThemeShadow])
    case points(Double)
    case font(Font.Design)
}

/// This library's value for each of upstream's token names.
private func tokens(_ t: IntelligentTheme) -> [String: TokenValue] {
    var out: [String: TokenValue] = [
        "font": .font(t.font), "mono": .font(t.mono),
        "shadow": .shadow(t.shadow), "shadowSm": .shadow(t.shadowSm),
    ]
    let colors: [(String, ThemeColor)] = [
        ("canvas", t.canvas), ("hover", t.hover), ("pressed", t.pressed), ("surface", t.surface),
        ("surfaceStrong", t.surfaceStrong), ("ink", t.ink), ("inkHover", t.inkHover), ("onInk", t.onInk),
        ("muted", t.muted), ("faint", t.faint), ("border", t.border), ("borderStrong", t.borderStrong),
        ("rangeTrack", t.rangeTrack), ("rangeThumb", t.rangeThumb), ("focus", t.focus), ("glass", t.glass),
        ("glassHover", t.glassHover), ("glassRing", t.glassRing), ("glassRingHover", t.glassRingHover),
        ("info", t.info), ("infoSoft", t.infoSoft), ("success", t.success), ("successSoft", t.successSoft),
        ("warning", t.warning), ("warningSoft", t.warningSoft), ("danger", t.danger), ("dangerSoft", t.dangerSoft),
        ("purple", t.purple), ("purpleSoft", t.purpleSoft), ("pink", t.pink), ("pinkSoft", t.pinkSoft),
        ("badgeNeutralSoft", t.badgeNeutralSoft), ("badgeSuccess", t.badgeSuccess),
        ("badgeSuccessSoft", t.badgeSuccessSoft), ("badgeWarning", t.badgeWarning),
        ("badgeWarningSoft", t.badgeWarningSoft), ("badgeDanger", t.badgeDanger),
        ("badgeDangerSoft", t.badgeDangerSoft), ("badgePurple", t.badgePurple),
        ("badgePurpleSoft", t.badgePurpleSoft), ("badgePink", t.badgePink), ("badgePinkSoft", t.badgePinkSoft),
        ("chart1", t.chart1), ("chart2", t.chart2), ("chart3", t.chart3), ("chart4", t.chart4),
        ("chart5", t.chart5), ("chart6", t.chart6), ("photoEmpty", t.photoEmpty),
        ("photoEmptyIcon", t.photoEmptyIcon),
    ]
    for (name, color) in colors { out[name] = .color(color) }
    let points: [(String, Double)] = [
        ("spaceTight", t.spaceTight), ("spaceInner", t.spaceInner), ("spaceGroup", t.spaceGroup),
        ("spaceGroupLg", t.spaceGroupLg), ("spaceSection", t.spaceSection), ("controlGap", t.controlGap),
        ("fieldGap", t.fieldGap), ("actionGap", t.actionGap), ("radiusSm", t.radiusSm), ("radiusMd", t.radiusMd),
        ("radiusLg", t.radiusLg), ("radiusXl", t.radiusXl),
    ]
    for (name, value) in points { out[name] = .points(value) }
    return out
}

func close(_ a: ThemeColor, _ b: ThemeColor, within tolerance: Double = 1e-9) -> Bool {
    abs(a.red - b.red) <= tolerance && abs(a.green - b.green) <= tolerance
        && abs(a.blue - b.blue) <= tolerance && abs(a.alpha - b.alpha) <= tolerance
}

/// `0 4px 4px -2px oklch(0 0 0 / 0.05), 0 4px 8px -2px oklch(...)`.
private func shadows(_ css: String) -> [ThemeShadow]? {
    var layers: [ThemeShadow] = []
    for layer in css.components(separatedBy: "), ") {
        let text = layer.hasSuffix(")") ? layer : layer + ")"
        guard let colorStart = text.range(of: "oklch(") else { return nil }
        let lengths = text[..<colorStart.lowerBound].split(separator: " ").map { part in
            Double(part.hasSuffix("px") ? String(part.dropLast(2)) : String(part))
        }
        guard lengths.count == 4, case let .color(color)? = CSSPaint.parse(String(text[colorStart.lowerBound...]))
        else { return nil }
        let l = lengths.compactMap { $0 }
        guard l.count == 4 else { return nil }
        layers.append(ThemeShadow(x: l[0], y: l[1], blur: l[2], spread: l[3], color: color))
    }
    return layers
}

@Suite struct ThemeTests {
    @Test(arguments: ["light", "dark"])
    func everyTokenIsUpstreamsValue(mode: String) throws {
        let fixture = try themeFixture()
        guard case let .object(resolved)? = fixture[mode] else {
            Issue.record("no \(mode) tokens in the fixture")
            return
        }
        let ours = tokens(mode == "dark" ? .dark : .light)
        // The same tokens, no more and no fewer.
        #expect(Set(resolved.map(\.key)) == Set(ours.keys))
        #expect(resolved.count == 66)
        for (name, value) in resolved {
            let css = try #require(value.string)
            switch ours[name] {
            case let .color(color)?:
                guard case let .color(theirs)? = CSSPaint.parse(css) else {
                    Issue.record("\(mode) \(name): cannot read upstream's \(css)")
                    continue
                }
                #expect(close(color, theirs), "\(mode) \(name): \(color) is not upstream's \(css)")
            case let .shadow(layers)?:
                let theirs = try #require(shadows(css), "\(mode) \(name): \(css)")
                #expect(theirs.count == layers.count)
                for (a, b) in zip(layers, theirs) {
                    #expect(a.x == b.x && a.y == b.y && a.blur == b.blur && a.spread == b.spread)
                    #expect(close(a.color, b.color))
                }
            case let .points(p)?:
                #expect(css == "\(Int(p))px", "\(mode) \(name)")
            case let .font(design)?:
                // Upstream's web font, Inter, is not approved (OFL): the
                // system font stands in. Its mono stack is the system's.
                if name == "font" {
                    #expect(css == #""Inter", sans-serif"#)
                    #expect(design == .default)
                } else {
                    #expect(css == "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace")
                    #expect(design == .monospaced)
                }
            case nil:
                Issue.record("\(mode) \(name): no such token here")
            }
        }
    }

    @Test func theModesDifferWhereUpstreamsDo() {
        let light = IntelligentTheme.light, dark = IntelligentTheme.dark
        #expect(light != dark)
        #expect(!close(light.ink, dark.ink) && !close(light.canvas, dark.canvas))
        // The soft tints, charts, spacing and radii are shared.
        #expect(light.successSoft == dark.successSoft && light.chart1 == dark.chart1)
        #expect(light.radiusLg == dark.radiusLg && light.spaceSection == dark.spaceSection)
        #expect(IntelligentTheme.forScheme(.dark) == dark && IntelligentTheme.forScheme(.light) == light)
    }

    @Test func aTokenOrRadiusPropResolvesThroughTheTheme() {
        let light = IntelligentTheme.light
        #expect(light.paint(.token(.infoSoft)) == .color(light.infoSoft))
        #expect(light.paint(.token(.chart3)) == .color(light.chart3))
        #expect(light.paint(.css("#0285ff")) == .color(light.chart1))
        #expect(light.paint(.css("not a colour")) == nil)
        #expect(light.paint(.css("currentColor")) == .currentColor)
        for token in ColorToken.allCases {
            #expect(IntelligentTheme.dark.paint(.token(token)) == .color(IntelligentTheme.dark.color(token)))
        }
        #expect(light.radius(.token(.sm)) == 6 && light.radius(.token(.xl)) == 14)
        #expect(light.radius(.points(999)) == 999)
    }
}

// MARK: Light or dark

/// Reads the theme a view sees.
private struct Probe: View {
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.colorScheme) private var scheme
    let seen: (IntelligentTheme, ColorScheme) -> Void

    var body: some View {
        seen(theme, scheme)
        return Color.clear.frame(width: 4, height: 4)
    }
}

@MainActor
private func themeSeen(system: ColorScheme, host: IntelligentUIMode?) -> (IntelligentTheme, ColorScheme)? {
    var seen: (IntelligentTheme, ColorScheme)?
    let view = Probe { seen = ($0, $1) }
        .intelligentUIMode(host)
        // What the system's appearance sets for the app.
        .environment(\.colorScheme, system)
    _ = ImageRenderer(content: view).nsImage
    return seen
}

@Suite @MainActor struct ThemeModeTests {
    @Test(arguments: [ColorScheme.light, .dark])
    func withoutAModeTheSystemSettingIsFollowed(system: ColorScheme) throws {
        let (theme, scheme) = try #require(themeSeen(system: system, host: nil))
        #expect(scheme == system)
        #expect(theme == IntelligentTheme.forScheme(system))
    }

    @Test(arguments: [ColorScheme.light, .dark])
    func theHostCanForceLight(system: ColorScheme) throws {
        let (theme, scheme) = try #require(themeSeen(system: system, host: .light))
        #expect(theme == .light)
        // System controls inside follow it, as upstream sets color-scheme.
        #expect(scheme == .light)
    }

    @Test(arguments: [ColorScheme.light, .dark])
    func theHostCanForceDark(system: ColorScheme) throws {
        let (theme, scheme) = try #require(themeSeen(system: system, host: .dark))
        #expect(theme == .dark)
        #expect(scheme == .dark)
    }
}
