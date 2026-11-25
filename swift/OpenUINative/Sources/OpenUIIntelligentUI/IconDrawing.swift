import OpenUINative
import SwiftUI

extension EnvironmentValues {
    @Entry var intelligentForeground: ThemeColor? = nil
}

extension View {
    // Keep the foreground used by Canvas glyphs in step with SwiftUI's foreground.
    func intelligentForegroundStyle(_ color: ThemeColor?) -> some View {
        foregroundStyle(color.map { AnyShapeStyle($0.color) } ?? AnyShapeStyle(.foreground))
            .transformEnvironment(\.intelligentForeground) { foreground in
                if let color { foreground = color }
            }
    }
}

struct IconDrawing: View {
    let element: Element
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.intelligentForeground) private var foreground

    static func side(for element: Element) -> CGFloat {
        switch element.word("size") {
        case "sm": 14
        case "lg": 24
        case "xl": 32
        default: 18
        }
    }

    private var tint: Color {
        if case let .color(color)? = ColorValue.foreground(element.prop("color")).flatMap(theme.paint) {
            return color.color
        }
        return (foreground ?? theme.ink).color
    }

    var body: some View {
        let side = Self.side(for: element)
        ZStack {
            LucideDrawing(name: element.word("name"), tint: tint)
        }
        .frame(width: side, height: side)
        .accessibilityHidden(true)
        .allowsHitTesting(false)
    }
}
