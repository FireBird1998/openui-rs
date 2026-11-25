import Observation

/// Persists completed turns and coordinates their writes with sidebar deletion.
@MainActor @Observable
final class ConversationLink {
    private(set) var id: String?
    private let storage: ThesysConversations
    private let gateway: ThesysGateway
    private let didCreate: (Conversation) -> Void

    // A newly created row stays out of the sidebar until storage holds items.
    private var unpublished: Conversation?
    private var needsRead = false
    private var writing: Task<Bool, any Error>?
    private var deleting: Task<Void, any Error>?
    private var deleted = false

    init(id: String?, storage: ThesysConversations, gateway: ThesysGateway,
         didCreate: @escaping (Conversation) -> Void) {
        self.id = id
        self.storage = storage
        self.gateway = gateway
        self.didCreate = didCreate
    }

    func credential() async throws -> ConversationCredential {
        try await storage.scopedCredential()
    }

    nonisolated func answer(_ request: ChatRequest, credential: ConversationCredential) -> ThesysGateway.Answer {
        gateway.answer(request, credential: credential)
    }

    /// Re-read an uncertain write before using the conversation again. A
    /// failed read or restore leaves it unresolved, so the next send tries
    /// the read again before sending any history to the model.
    @discardableResult
    func reconcile(credential: ConversationCredential,
                   apply: ([ChatMessage]) async throws -> Bool) async throws -> Bool {
        if needsRead, let target = id ?? unpublished?.id {
            let stored = try await storage.messages(id: target, credential: credential)
            let matches = try await apply(stored)
            needsRead = false
            if stored.isEmpty { try await removeUnpublished(credential) }
            else { publish() }
            return matches
        }
        try await removeUnpublished(credential)
        return false
    }

    /// False means a pending DELETE succeeded and the turn was discarded.
    func save(_ messages: [ChatMessage], title: String, credential: ConversationCredential,
              applyStored: @escaping @MainActor ([ChatMessage]) async throws -> Bool) async throws -> Bool {
        // Wait outside the write task. If DELETE is already waiting for a
        // write, it must be able to finish before a new write can begin.
        if let deleting { _ = await deleting.result }
        guard !deleted else { return false }
        let task = Task {
            try await write(messages, title: title, credential: credential, applyStored: applyStored)
        }
        writing = task
        defer { writing = nil }
        return try await task.value
    }

    private func write(_ messages: [ChatMessage], title: String, credential: ConversationCredential,
                       applyStored: ([ChatMessage]) async throws -> Bool) async throws -> Bool {
        if id == nil {
            unpublished = try await storage.create(firstMessage: title, credential: credential)
        }
        guard let target = id ?? unpublished?.id else { return false }
        do {
            try await storage.append(id: target, messages: messages, credential: credential)
        } catch {
            let appendError = error
            let status = (appendError as? GatewayError)?.status
            // A timeout, conflict, transport loss or server error can arrive
            // after a write. Check the items, and never blindly repeat the POST.
            let refused = status.map { (400..<500).contains($0) && $0 != 408 && $0 != 409 } ?? false
            needsRead = !refused
            do {
                if try await reconcile(credential: credential, apply: applyStored) { return true }
            } catch {
                throw GatewayError("\(ChatThread.message(for: appendError)) \(ChatThread.message(for: error))")
            }
            throw appendError
        }
        publish()
        return true
    }

    func delete(_ target: String) async throws {
        let task = Task {
            _ = await writing?.result
            try await storage.delete(id: target)
            deleted = true
        }
        deleting = task
        defer { deleting = nil }
        try await task.value
    }

    private func publish() {
        guard let unpublished else { return }
        self.unpublished = nil
        id = unpublished.id
        didCreate(unpublished)
    }

    private func removeUnpublished(_ credential: ConversationCredential) async throws {
        guard let unpublished else { return }
        do {
            try await storage.delete(id: unpublished.id, credential: credential)
        } catch let failure as GatewayError where failure.status == 404 {
            // A previous cleanup may have succeeded before losing its response.
        } catch {
            throw GatewayError("Could not remove the empty conversation. \(ChatThread.message(for: error))")
        }
        self.unpublished = nil
    }
}

/// Sidebar state and the open chat. Sidebar operations serialize with one
/// another. Switching waits for the outgoing turn's final write so reopening
/// it cannot read an older version.
@MainActor @Observable
final class ChatThreads {
    private(set) var threads: [Conversation] = []
    private(set) var thread: ChatThread
    private(set) var storageError: String?
    private(set) var isBusy = false
    var selectedID: String? { thread.conversation?.id }

    private let library: ChatLibrary
    private let gateway: ThesysGateway
    private let storage: ThesysConversations

    init(library: ChatLibrary, gateway: ThesysGateway, storage: ThesysConversations) {
        self.library = library
        self.gateway = gateway
        self.storage = storage
        thread = ChatThread(library: library, gateway: gateway)
        attach(thread, id: nil)
    }

    func loadThreads() async {
        await change {
            let listed = try await storage.list()
            threads = Self.newestFirst(listed)
        }
    }

    func open(_ id: String) async {
        guard id != selectedID, threads.contains(where: { $0.id == id }) else { return }
        await change {
            let messages = try await storage.messages(id: id)
            let restored = ChatThread(library: library, gateway: gateway)
            try await restored.restore(messages)
            try Task.checkCancellation()
            await stopCurrent()
            attach(restored, id: id)
            thread = restored
        }
    }

    func newChat() async {
        await change {
            await stopCurrent()
            showNewChat()
        }
    }

    func rename(_ id: String, to value: String) async {
        let title = ThesysConversations.trim(value)
        guard let current = threads.first(where: { $0.id == id }), !title.isEmpty, title != current.title else { return }
        await change {
            let updated = try await storage.rename(id: id, title: title)
            threads = threads.map { $0.id == id ? updated : $0 }
        }
    }

    func delete(_ id: String) async {
        guard threads.contains(where: { $0.id == id }) else { return }
        await change {
            if selectedID == id, let link = thread.conversation {
                try await link.delete(id)
                await stopCurrent()
                showNewChat()
            } else {
                try await storage.delete(id: id)
            }
            threads.removeAll { $0.id == id }
        }
    }

    private func change(_ operation: () async throws -> Void) async {
        guard !isBusy else { return }
        isBusy = true
        storageError = nil
        defer { isBusy = false }
        do {
            try await operation()
        } catch is CancellationError {
        } catch {
            storageError = ChatThread.message(for: error)
        }
    }

    private func stopCurrent() async {
        thread.stop()
        await thread.waitUntilIdle()
    }

    private func showNewChat() {
        let next = ChatThread(library: library, gateway: gateway)
        attach(next, id: nil)
        thread = next
    }

    private func attach(_ chat: ChatThread, id: String?) {
        chat.conversation = ConversationLink(id: id, storage: storage, gateway: gateway) { [weak self] new in
            guard let self else { return }
            threads.removeAll { $0.id == new.id }
            threads = Self.newestFirst(threads + [new])
        }
    }

    private static func newestFirst(_ conversations: [Conversation]) -> [Conversation] {
        conversations.sorted { $0.createdAt > $1.createdAt }
    }
}
