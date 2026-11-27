import AppKit
import OpenUINative
import SwiftUI
import Testing
@testable import ChatKit

@Suite(.serialized) @MainActor struct ChatViewTests {
    @Test(arguments: [
        ("Compare breakfast nutrition",
         "Compare the nutrition of common breakfast foods: oatmeal, eggs, Greek yogurt, avocado toast, and a banana."),
        ("See how compound interest grows",
         "Explain how compound interest works using $10,000 invested at 7% for 30 years."),
        ("Help me pick a car",
         "Help me pick the right type of car for my lifestyle. Ask me a few questions first."),
    ])
    func theWelcomeShowsTemplateCopyAndAStarterSendsImmediately(label: String, prompt: String) async throws {
        let transport = RecordedTransport(try .named("streamed-answer"))
        let thread = ChatThread(library: try await ChatLibrary.reactUI(),
                                gateway: ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport))
        let view = MountedChatContent(ChatView(thread: thread), width: 700, height: 480)
        defer { view.close() }

        try #require(await eventually { view.button(named: label) != nil })
        let nodes = view.accessibilityNodes()
        let visibleText = nodes.flatMap { view.accessibilityText(of: $0) }
        #expect(visibleText.contains("What's on your mind today?"), "\(visibleText)")
        #expect(nodes.contains { $0.accessibilityPlaceholderValue?() == "Ask anything..." })
        let starter = try #require(view.button(named: label))

        #expect(starter.accessibilityPerformPress?() == true)
        await thread.waitUntilIdle()
        let request = try #require(transport.sent.first)
        let messages = try #require(try JSONValue.parse(request.bodyText)["messages"]?.array)
        #expect(transport.sent.count == 1)
        #expect(messages.last?["role"]?.string == "user")
        #expect(messages.last?["content"]?.string == prompt)
        #expect(thread.draft.isEmpty)
    }

    @Test(arguments: [ColorScheme.light, .dark])
    func themeToggleFollowsTheSystemThenPinsAndRemembersTheOppositeTheme(system: ColorScheme) async throws {
        let store = MemoryPreferences()
        let preferences = ChatPreferences(store: store)
        let opposite: ColorScheme = system == .light ? .dark : .light
        let initialLabel = system == .light ? "Switch to dark theme" : "Switch to light theme"
        let pinnedLabel = system == .light ? "Switch to light theme" : "Switch to dark theme"
        #expect(preferences.themeOverride == nil)

        let view = MountedChatContent(ChatThemeToggle(preferences: preferences)
            .environment(\.colorScheme, system), width: 200, height: 100)
        defer { view.close() }

        try #require(await eventually { view.button(named: initialLabel) != nil })
        let button = try #require(view.button(named: initialLabel))
        #expect(button.accessibilityPerformPress?() == true)
        #expect(await eventually { preferences.themeOverride == opposite })
        #expect(ChatPreferences(store: store).themeOverride == opposite)
        try #require(await eventually { view.button(named: pinnedLabel) != nil })
        let pinnedButton = try #require(view.button(named: pinnedLabel))
        #expect(pinnedButton.accessibilityPerformPress?() == true)
        #expect(await eventually { preferences.themeOverride == system })
        #expect(ChatPreferences(store: store).themeOverride == system)
    }

    @Test func theSharedThemeAppliesToChatAndSettings() async throws {
        let preferences = ChatPreferences(store: MemoryPreferences())
        let keySettings = KeySettings(store: InMemoryKeyStore(testKey))
        let transport = try RecordedTransport(.named("threads-token"), .named("workspace-list"))
        let chat = MountedChatContent(ChatWindow(keySettings: keySettings, preferences: preferences, transport: transport),
                                      width: 760, height: 480)
        defer { chat.close() }
        let settings = MountedChatContent(SettingsView(settings: keySettings, preferences: preferences), width: 440, height: 280)
        defer { settings.close() }

        try #require(await eventually { chat.button(named: "Help me pick a car") != nil })
        for scheme in [ColorScheme.dark, .light] {
            preferences.setTheme(scheme)
            #expect(await eventually { chat.colorScheme == scheme && settings.colorScheme == scheme })
        }
    }
}

@MainActor
final class MountedChatContent {
    private let window: NSWindow
    private let host: NSHostingView<AnyView>

    init(_ content: some View, width: CGFloat, height: CGFloat) {
        _ = NSApplication.shared
        host = NSHostingView(rootView: AnyView(content
            .environment(\.accessibilityEnabled, true)
            .frame(width: width, height: height)))
        window = NSWindow(contentRect: CGRect(x: -20000, y: -20000, width: width, height: height),
                          styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFrontRegardless()
    }

    func close() { window.close() }

    func layout() { host.layoutSubtreeIfNeeded() }

    var colorScheme: ColorScheme {
        host.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? .dark : .light
    }

    func button(named label: String) -> AnyObject? {
        accessibilityNodes().first {
            $0.accessibilityRole?() == .button && accessibilityText(of: $0).contains(label)
        }
    }

    func accessibilityNodes() -> [AnyObject] {
        layout()
        var pending: [AnyObject] = [host]
        var nodes: [AnyObject] = []
        while let node = pending.popLast() {
            nodes.append(node)
            pending.append(contentsOf: (node.accessibilityChildren?() ?? []).map { $0 as AnyObject })
        }
        return nodes
    }

    func accessibilityText(of node: AnyObject) -> [String] {
        let value = (node as? any NSAccessibilityProtocol)?.accessibilityValue() as? String
        return [node.accessibilityLabel?(), node.accessibilityTitle?(), value].compactMap { $0 }
    }
}
