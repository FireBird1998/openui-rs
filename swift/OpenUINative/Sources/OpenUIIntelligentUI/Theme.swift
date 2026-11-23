// The library's light and dark themes: every token of upstream's
// shared/tokens.ts, resolved as a page with no overrides resolves it. The
// library's ThemeProvider writes its own defaults (defaultThemes.ts: chart
// colours, spacing, radii, the mono font) and react-components' writes its
// default theme (defaultTheme.ts), which gives every colour a token falls
// back to. The views follow the system's light or dark setting unless the
// host forces one with `intelligentUIMode(_:)`. That default departs from
// upstream on purpose (#128, #159): with no mode set anywhere, upstream's
// ThemeProvider draws light.
//
// Fonts: upstream names the web font Inter, under a licence (OFL) this
// project has not approved, so the library uses the system font. Its mono
// stack starts with `ui-monospace`, which is the system's monospaced font.
import SwiftUI

/// A shadow layer, as CSS `box-shadow` writes it, in points.
public struct ThemeShadow: Sendable, Equatable {
    public let x: Double
    public let y: Double
    public let blur: Double
    public let spread: Double
    public let color: ThemeColor
}

/// The Intelligent UI library's theme tokens for one mode.
public struct IntelligentTheme: Sendable, Equatable {
    // Fonts.
    /// Upstream's `"Inter", sans-serif`: the system font here.
    public let font: Font.Design
    /// Upstream's `ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`.
    public let mono: Font.Design

    // Base colours.
    public let canvas: ThemeColor
    public let ink: ThemeColor
    public let inkHover: ThemeColor
    public let onInk: ThemeColor
    public let muted: ThemeColor
    public let faint: ThemeColor

    // Ink washes.
    public let hover: ThemeColor
    public let pressed: ThemeColor
    public let surface: ThemeColor
    public let surfaceStrong: ThemeColor
    public let border: ThemeColor
    public let borderStrong: ThemeColor
    public let rangeTrack: ThemeColor
    public let rangeThumb: ThemeColor
    public let focus: ThemeColor

    // Glass.
    public let glass: ThemeColor
    public let glassHover: ThemeColor
    public let glassRing: ThemeColor
    public let glassRingHover: ThemeColor

    // Shadows.
    public let shadow: [ThemeShadow]
    public let shadowSm: [ThemeShadow]

    // Status colours and their soft tints.
    public let info: ThemeColor
    public let infoSoft: ThemeColor
    public let success: ThemeColor
    public let successSoft: ThemeColor
    public let warning: ThemeColor
    public let warningSoft: ThemeColor
    public let danger: ThemeColor
    public let dangerSoft: ThemeColor
    public let purple: ThemeColor
    public let purpleSoft: ThemeColor
    public let pink: ThemeColor
    public let pinkSoft: ThemeColor

    // Badges.
    public let badgeNeutralSoft: ThemeColor
    public let badgeSuccess: ThemeColor
    public let badgeSuccessSoft: ThemeColor
    public let badgeWarning: ThemeColor
    public let badgeWarningSoft: ThemeColor
    public let badgeDanger: ThemeColor
    public let badgeDangerSoft: ThemeColor
    public let badgePurple: ThemeColor
    public let badgePurpleSoft: ThemeColor
    public let badgePink: ThemeColor
    public let badgePinkSoft: ThemeColor

    // Charts.
    public let chart1: ThemeColor
    public let chart2: ThemeColor
    public let chart3: ThemeColor
    public let chart4: ThemeColor
    public let chart5: ThemeColor
    public let chart6: ThemeColor

    // Spacing, in points.
    public let spaceTight: Double
    public let spaceInner: Double
    public let spaceGroup: Double
    public let spaceGroupLg: Double
    public let spaceSection: Double
    public let controlGap: Double
    public let fieldGap: Double
    public let actionGap: Double

    // Corner radii, in points.
    public let radiusSm: Double
    public let radiusMd: Double
    public let radiusLg: Double
    public let radiusXl: Double

    // Photo.
    public let photoEmpty: ThemeColor
    public let photoEmptyIcon: ThemeColor
}

extension IntelligentTheme {
    /// What react-components' default theme gives a mode.
    private struct Base {
        var canvas: ThemeColor, ink: ThemeColor, secondary: ThemeColor
        var success: ThemeColor, warning: ThemeColor, danger: ThemeColor, purple: ThemeColor, pink: ThemeColor
        var shadow: [ThemeShadow], shadowSm: [ThemeShadow]
    }

    // react-components' swatches.ts, shades used by its defaultTheme.ts.
    private static let neutral25 = ThemeColor.oklch(0.994, 0, 89.876)
    private static let neutral50 = ThemeColor.oklch(0.985, 0, 89.876)
    private static let neutral900 = ThemeColor.oklch(0.205, 0, 0)
    private static let neutral1000 = ThemeColor.oklch(0.097, 0, 0)
    // react-components' Separator reads borderDefault (6% overlay).
    static func separator(_ scheme: ColorScheme) -> ThemeColor {
        (scheme == .dark ? neutral25 : neutral1000).mixedWithTransparent(0.06)
    }

    // The soft tints are the same in both modes.
    private static let green600 = ThemeColor.oklch(0.627, 0.17, 149.214)
    private static let yellow500 = ThemeColor.oklch(0.795, 0.162, 86.047)
    private static let red600 = ThemeColor.oklch(0.577, 0.215, 27.325)
    private static let purple500 = ThemeColor.oklch(0.627, 0.233, 303.9)
    private static let pink600 = ThemeColor.oklch(0.592, 0.218, 0.584)

    private static func shadow(_ layers: [(Double, Double, Double, Double, Double)]) -> [ThemeShadow] {
        layers.map { ThemeShadow(x: $0.0, y: $0.1, blur: $0.2, spread: $0.3, color: .oklch(0, 0, 0, alpha: $0.4)) }
    }

    private static let lightBase = Base(
        canvas: neutral25,
        ink: neutral1000,
        secondary: neutral1000.mixedWithTransparent(0.6),
        success: .oklch(0.448, 0.108, 151.328),   // green 800
        warning: .oklch(0.476, 0.103, 61.907),    // yellow 800
        danger: .oklch(0.505, 0.19, 27.518),      // red 700
        purple: .oklch(0.438, 0.198, 303.724),    // purple 800
        pink: .oklch(0.459, 0.17, 3.815),         // pink 800
        shadow: shadow([(0, 4, 4, -2, 0.05), (0, 4, 8, -2, 0.04)]),      // shadowL
        shadowSm: shadow([(0, 1, 3, -2, 0.02), (0, 2, 5, -2, 0.04)])     // shadowS
    )

    private static let darkBase = Base(
        canvas: neutral900,
        ink: neutral50,
        secondary: neutral50.mixedWithTransparent(0.6),
        success: .oklch(0.871, 0.136, 154.449),   // green 300
        warning: .oklch(0.905, 0.166, 98.111),    // yellow 300
        danger: .oklch(0.808, 0.103, 19.571),     // red 300
        purple: .oklch(0.827, 0.108, 306.383),    // purple 300
        pink: .oklch(0.823, 0.11, 346.018),       // pink 300
        shadow: shadow([(0, 4, 4, -2, 0.12), (0, 4, 8, -2, 0.10)]),
        shadowSm: shadow([(0, 1, 3, -2, 0.06), (0, 2, 5, -2, 0.10)])
    )

    /// The light theme.
    public static let light = make(lightBase)
    /// The dark theme.
    public static let dark = make(darkBase)

    // shared/color.ts overrides the inherited text tokens. The mixes in
    // tokens.ts (including a Box's own border) resolve against that ink.
    static func forScheme(_ scheme: ColorScheme, ink: ThemeColor?) -> IntelligentTheme {
        guard let ink else { return forScheme(scheme) }
        return make(scheme == .dark ? darkBase : lightBase, contrast: ink)
    }

    /// The theme for a colour scheme.
    public static func forScheme(_ scheme: ColorScheme) -> IntelligentTheme {
        scheme == .dark ? .dark : .light
    }

    /// Every token as upstream's tokens.ts derives it from the base colours.
    private static func make(_ base: Base, contrast: ThemeColor? = nil) -> IntelligentTheme {
        let ink = contrast ?? base.ink, canvas = base.canvas
        let successSoft = green600.mixedWithTransparent(0.12)
        let warningSoft = yellow500.mixedWithTransparent(0.16)
        let dangerSoft = red600.mixedWithTransparent(0.12)
        let purpleSoft = purple500.mixedWithTransparent(0.12)
        let pinkSoft = pink600.mixedWithTransparent(0.12)
        return IntelligentTheme(
            font: .default,
            mono: .monospaced,
            canvas: canvas,
            ink: ink,
            inkHover: ink.mixed(inOKLCH: 0.82, with: canvas),
            onInk: contrast.map { $0 == .hex(0xffffff) ? .hex(0x0d0d0d) : .hex(0xffffff) } ?? canvas,
            muted: contrast?.mixedWithTransparent(0.72) ?? base.secondary,
            faint: contrast?.mixedWithTransparent(0.55) ?? base.secondary,
            hover: ink.mixedWithTransparent(0.05),
            pressed: ink.mixedWithTransparent(0.08),
            surface: ink.mixedWithTransparent(0.04),
            surfaceStrong: ink.mixedWithTransparent(0.08),
            border: ink.mixedWithTransparent(0.08),
            borderStrong: ink.mixedWithTransparent(0.10),
            rangeTrack: ink.mixedWithTransparent(0.06),
            rangeThumb: ink,
            focus: ink,
            glass: canvas.mixedWithTransparent(0.70),
            glassHover: canvas.mixedWithTransparent(0.85),
            glassRing: canvas.mixedWithTransparent(0.70),
            glassRingHover: canvas.mixedWithTransparent(0.85),
            shadow: base.shadow,
            shadowSm: base.shadowSm,
            info: ink,
            infoSoft: ink.mixedWithTransparent(0.06),
            success: base.success,
            successSoft: successSoft,
            warning: base.warning,
            warningSoft: warningSoft,
            danger: base.danger,
            dangerSoft: dangerSoft,
            purple: base.purple,
            purpleSoft: purpleSoft,
            pink: base.pink,
            pinkSoft: pinkSoft,
            badgeNeutralSoft: ink.mixedWithTransparent(0.06),
            badgeSuccess: base.success,
            badgeSuccessSoft: successSoft,
            badgeWarning: base.warning,
            badgeWarningSoft: warningSoft,
            badgeDanger: base.danger,
            badgeDangerSoft: dangerSoft,
            badgePurple: base.purple,
            badgePurpleSoft: purpleSoft,
            badgePink: base.pink,
            badgePinkSoft: pinkSoft,
            chart1: .hex(0x0285ff),
            chart2: .hex(0x00a67d),
            chart3: .hex(0x8b5cf6),
            chart4: .hex(0xf4a340),
            chart5: .hex(0xe46d91),
            chart6: .hex(0x4b9ca6),
            spaceTight: 4,
            spaceInner: 8,
            spaceGroup: 12,
            spaceGroupLg: 16,
            spaceSection: 24,
            controlGap: 8,
            fieldGap: 24,
            actionGap: 40,
            radiusSm: 6,
            radiusMd: 10,
            radiusLg: 12,
            radiusXl: 14,
            photoEmpty: ink.mixedWithTransparent(0.04),
            photoEmptyIcon: ink.mixedWithTransparent(0.10)
        )
    }
}

// MARK: Resolving prop values

extension IntelligentTheme {
    /// A colour token's value.
    func color(_ token: ColorToken) -> ThemeColor {
        switch token {
        case .ink: ink
        case .muted: muted
        case .faint: faint
        case .canvas: canvas
        case .surface: surface
        case .surfaceStrong: surfaceStrong
        case .border: border
        case .info: info
        case .success: success
        case .warning: warning
        case .danger: danger
        case .purple: purple
        case .pink: pink
        case .infoSoft: infoSoft
        case .successSoft: successSoft
        case .warningSoft: warningSoft
        case .dangerSoft: dangerSoft
        case .purpleSoft: purpleSoft
        case .pinkSoft: pinkSoft
        case .chart1: chart1
        case .chart2: chart2
        case .chart3: chart3
        case .chart4: chart4
        case .chart5: chart5
        case .chart6: chart6
        }
    }

    /// What a Color prop paints: its token's colour, or what its raw CSS
    /// names (nil when it names no colour, as the page would ignore it).
    func paint(_ value: ColorValue) -> CSSPaint? {
        switch value {
        case let .token(token): .color(color(token))
        case let .css(text): CSSPaint.parse(text)
        }
    }

    /// A Radius prop in points.
    func radius(_ value: Radius) -> Double {
        switch value {
        case .token(.sm): radiusSm
        case .token(.md): radiusMd
        case .token(.lg): radiusLg
        case .token(.xl): radiusXl
        case let .points(p): p
        }
    }
}

// MARK: Light or dark

/// A mode the host forces on the library's views.
public enum IntelligentUIMode: Sendable, Equatable {
    case light
    case dark

    var colorScheme: ColorScheme { self == .dark ? .dark : .light }
}

extension View {
    /// Draws the Intelligent UI library's views in `mode`, and the system
    /// controls inside them with it, as upstream's ThemeProvider sets
    /// `color-scheme`. With nil, they follow the system's setting.
    public func intelligentUIMode(_ mode: IntelligentUIMode?) -> some View {
        modifier(IntelligentUIModeModifier(mode: mode))
    }
}

private struct IntelligentUIModeModifier: ViewModifier {
    let mode: IntelligentUIMode?

    func body(content: Content) -> some View {
        if let mode {
            content.environment(\.colorScheme, mode.colorScheme)
        } else {
            content
        }
    }
}

extension EnvironmentValues {
    @Entry var intelligentInk: ThemeColor? = nil

    /// The library's theme here: the forced mode's, else the system's.
    public var intelligentTheme: IntelligentTheme { .forScheme(colorScheme, ink: intelligentInk) }
}
