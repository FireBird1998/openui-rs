import Foundation
import Observation
import SwiftUI

@MainActor
protocol ChatPreferenceStore {
    func string(forKey key: String) -> String?
    func set(_ value: String?, forKey key: String)
}

extension UserDefaults: ChatPreferenceStore {
    func set(_ value: String?, forKey key: String) { set(value as Any?, forKey: key) }
}

enum ChatPromptMode {
    case cloud, full
}

/// One set of preferences shared by the app's windows and conversations.
@MainActor @Observable
public final class ChatPreferences {
    @ObservationIgnored private let store: any ChatPreferenceStore
    private static let modelKey = "selectedModel"
    private static let themeKey = "themeMode"
    private static let fallbackModel = "google/gemini-3.6-flash-free"

    final class ModelSelection {
        let modelID: String

        init(_ modelID: String) { self.modelID = modelID }
    }

    private(set) var modelSelection: ModelSelection
    public var modelID: String { modelSelection.modelID }
    var promptMode: ChatPromptMode = .cloud

    /// No override until the person uses the toggle.
    private(set) var themeOverride: ColorScheme?

    public convenience init() {
        self.init(store: UserDefaults.standard)
    }

    init(store: any ChatPreferenceStore) {
        self.store = store
        if let saved = store.string(forKey: Self.modelKey), ChatModel.contains(saved) {
            modelSelection = ModelSelection(saved)
        } else {
            modelSelection = ModelSelection(Template.defaultModel)
        }
        themeOverride = switch store.string(forKey: Self.themeKey) {
        case "light": .light
        case "dark": .dark
        default: nil
        }
    }

    func setTheme(_ scheme: ColorScheme) {
        themeOverride = scheme
        store.set(scheme == .dark ? "dark" : "light", forKey: Self.themeKey)
    }

    func defaultWasRefused(_ requestedSelection: ModelSelection) {
        // Selecting the same model again still replaces the request's selection.
        guard requestedSelection.modelID == Template.defaultModel, modelSelection === requestedSelection else { return }
        selectModel(Self.fallbackModel)
    }

    public func selectModel(_ id: String) {
        guard ChatModel.contains(id) else { return }
        modelSelection = ModelSelection(id)
        store.set(id, forKey: Self.modelKey)
    }
}
