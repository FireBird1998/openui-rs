import OpenUINative
import SwiftUI

struct BadgeDrawing: View {
    let element: Element
    @Environment(\.intelligentTheme) private var theme

    private var colors: (ink: ThemeColor, fill: ThemeColor) {
        switch element.word("color") {
        case "info": (theme.info, theme.infoSoft)
        case "success": (theme.badgeSuccess, theme.badgeSuccessSoft)
        case "warning": (theme.badgeWarning, theme.badgeWarningSoft)
        case "danger": (theme.badgeDanger, theme.badgeDangerSoft)
        case "purple": (theme.badgePurple, theme.badgePurpleSoft)
        case "pink": (theme.badgePink, theme.badgePinkSoft)
        default: (theme.ink, theme.badgeNeutralSoft)
        }
    }

    var body: some View {
        HStack(spacing: 5) {
            if let name = element.word("icon"), !JSWhitespace.trim(name).isEmpty, let image = LucideGlyph.image(named: name) {
                Image(nsImage: image).renderingMode(.template).resizable().scaledToFit().frame(width: 14, height: 14)
                    .padding(.leading, -2).accessibilityHidden(true)
            }
            StyledWords(words: AttributedString(element.word("text") ?? ""),
                        metrics: TextMetrics(13, 18 / 13, weight: 560, tracking: -0.13), color: colors.ink)
                .lineLimit(1)
        }
        .foregroundStyle(colors.ink.color)
        .padding(.horizontal, 10).padding(.vertical, 3)
        .frame(minHeight: 26)
        .background(colors.fill.color, in: Capsule())
        .clipContainer(to: Capsule())
    }
}

struct QuoteDrawing: View {
    let element: Element
    @Environment(\.intelligentTheme) private var theme

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            StyledWords(words: InlineText.parse(element.word("text") ?? ""),
                        metrics: TextMetrics(16, 1.6), color: theme.ink, strong: 700)
            if let cite = element.word("cite"), !cite.isEmpty {
                StyledWords(words: InlineText.parse(cite), metrics: TextMetrics(13), color: theme.muted, strong: 700)
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
        .clipContainer()
        .padding(.vertical, 14).padding(.leading, 19).padding(.trailing, 16)
        .background(theme.surface.color, in: UnevenRoundedRectangle(bottomTrailingRadius: 12, topTrailingRadius: 12))
        .overlay(alignment: .leading) { Rectangle().fill(theme.ink.color).frame(width: 3).allowsHitTesting(false) }
    }
}
