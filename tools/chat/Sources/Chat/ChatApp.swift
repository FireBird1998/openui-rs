import AppKit
import ChatKit
import SwiftUI

@main
struct ChatApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var keySettings = KeySettings(store: KeychainKeyStore())

    var body: some Scene {
        WindowGroup("Chat") {
            ChatWindow(keySettings: keySettings)
        }
        Settings {
            SettingsView(settings: keySettings)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Run as a bare executable (`swift run`), the app still gets a Dock
        // icon, a menu bar and focus.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }
}
