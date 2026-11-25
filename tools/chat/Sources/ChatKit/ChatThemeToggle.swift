import SwiftUI

struct ChatThemeToggle: View {
    @Environment(\.colorScheme) private var colorScheme
    let preferences: ChatPreferences

    var body: some View {
        let isDark = (preferences.themeOverride ?? colorScheme) == .dark
        let label = isDark ? "Switch to light theme" : "Switch to dark theme"
        Button {
            preferences.setTheme(isDark ? .light : .dark)
        } label: {
            Label(label, systemImage: isDark ? "sun.max" : "moon")
                .labelStyle(.iconOnly)
        }
        .help(label)
    }
}
