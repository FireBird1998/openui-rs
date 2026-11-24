// The Intelligent UI library: upstream's @openuidev/intelligent-ui-library
// 0.0.2 (thesysdev/openui 7b7a519, MIT) as the model sees it, declared in
// Swift (ADR 0017). The same 52 components in the same six groups with root
// `Col`, the same props in the same order, the same descriptions and prompt
// options; IntelligentUIAdditions lists the only differences. Every
// component draws a placeholder naming it until it is drawn natively. The
// 59 functions are in Functions.swift.
import Foundation
import OpenUINative

public enum IntelligentUI {
    /// The root component.
    public static let root = "Col"

    /// The 52 components, in upstream's order (`intelligentComponents`).
    public static let components: [DefinedComponent] = layout + text + inputs + mediaAndData + play + drawing

    /// The prompt's component groups, as upstream's `componentGroups`.
    public static let componentGroups: [ComponentGroup] = [
        ComponentGroup(
            name: "Layout",
            components: names(layout),
            notes: [
                #"- gap and padding count 4px steps. Length is px or a CSS length like "50%". Radius is "sm", "md", "lg", "xl", "full" or px."#,
                #"- Color is "ink", "muted", "faint", "surface", "info", "success", "warning", "danger", "purple", "pink", "chart1" to "chart6", or a CSS color. As a background, status colors are soft tints."#,
            ]
        ),
        ComponentGroup(name: "Text", components: names(text)),
        ComponentGroup(
            name: "Inputs",
            components: names(inputs),
            notes: [
                "- An input's first argument is its value, usually a $variable that other statements read.",
                #"- Every input either changes something shown on the page, or sits in a Form whose submit Button sends a short first-person @ToAssistant ("Help me choose a bike"); the form values go with it. No dead inputs."#,
                #"- In a Form, a field's last argument rules is checked before the form sends, and an error shows under the field. Rules is {required?, min?, max?, minLength?, maxLength?, email?, url?, numeric?, pattern?}: Input($email, "Email", "text", null, {required: true, email: true})."#,
            ]
        ),
        ComponentGroup(name: "Media and data", components: names(mediaAndData)),
        ComponentGroup(name: "Play", components: names(play)),
        // The group lists G right after Svg; the declarations put it after the shapes.
        ComponentGroup(name: "Drawing", components: ["Svg", "G"] + shapeNames + ["HtmlView"]),
    ]

    private static func names(_ components: [DefinedComponent]) -> [String] {
        components.map(\.name)
    }
}

extension Core {
    /// The Intelligent UI library with a host-supplied source of random draws in [0, 1).
    public func intelligentUILibrary(random: @escaping @Sendable () -> Double) async throws(OpenUIError) -> Library {
        try await intelligentUILibrary(clock: Date.init, timeZone: .autoupdatingCurrent, random: random)
    }

    /// Builds the library with the host's clock and viewer time zone.
    public func intelligentUILibrary(clock: @escaping @Sendable () -> Date, timeZone: TimeZone,
                                     random: @escaping @Sendable () -> Double = { Double.random(in: 0..<1) }) async throws(OpenUIError) -> Library {
        try await createLibrary(components: IntelligentUI.components, groups: IntelligentUI.componentGroups,
                                functions: IntelligentUI.functions(clock: clock, timeZone: timeZone, random: random),
                                root: IntelligentUI.root)
    }

    /// The Intelligent UI library, built on this core.
    public func intelligentUILibrary() async throws(OpenUIError) -> Library {
        try await createLibrary(components: IntelligentUI.components, groups: IntelligentUI.componentGroups,
                                functions: IntelligentUI.functions, root: IntelligentUI.root)
    }
}
