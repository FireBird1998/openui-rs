import AppKit
import ChatKit
import SwiftUI

@main
struct ChatApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup("Chat") {
            ChatWindow()
        }
        Settings {
            SettingsView()
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
