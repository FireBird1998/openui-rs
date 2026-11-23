// shared/color.ts recognizes fewer spellings than a CSS paint does.
@testable import OpenUIIntelligentUI
import SwiftUI
import Testing

@Suite struct OnColorTests {
    @Test func rawHexAndRGBUseUpstreamsLuminanceThreshold() {
        let white = ThemeColor.hex(0xffffff), dark = ThemeColor.hex(0x0d0d0d)
        for value in [" #Fff ", "#a2a2a2", "RGB(255 255 255)", "rgba(255, 255, 255, 0)"] {
            #expect(onColor(value) == dark)
        }
        for value in ["#000", "#a1a1a1", "rgb(255,0,0)"] {
            #expect(onColor(value) == white)
        }
        for value in ["white", "info", "#fff0", "#ffffffff", "rgb(100% 100% 100%)", "hsl(0 0% 100%)"] {
            #expect(onColor(value) == nil)
        }
    }

    @Test func onColorChangesInkMutedFaintAndOnInkInEitherMode() {
        for scheme in [ColorScheme.light, .dark] {
            for (raw, expected, inverse) in [("#fff", 0x0d0d0d, 0xffffff), ("#000", 0xffffff, 0x0d0d0d)] {
                let ink = ThemeColor.hex(UInt32(expected))
                let theme = IntelligentTheme.forScheme(scheme, ink: onColor(raw))
                #expect(theme.ink == ink)
                #expect(theme.muted == ink.mixedWithTransparent(0.72))
                #expect(theme.faint == ink.mixedWithTransparent(0.55))
                #expect(theme.onInk == .hex(UInt32(inverse)))
                #expect(theme.surface == ink.mixedWithTransparent(0.04))
                #expect(theme.canvas == IntelligentTheme.forScheme(scheme).canvas)
            }
        }
    }
}
