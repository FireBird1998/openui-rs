import AppKit
import SwiftUI
import ViewerKit

@main
struct ViewerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        WindowGroup("Viewer") {
            CorpusView()
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Run from `swift run` or Xcode as a bare executable, the app still
        // gets a Dock icon, a menu bar and focus.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate()
    }
}
