import SwiftUI

struct ThreadWorkspace: View {
    let chats: ChatThreads
    let preferences: ChatPreferences
    @State private var renaming: Conversation?

    var body: some View {
        NavigationSplitView {
            VStack(spacing: 0) {
                Button {
                    Task { await chats.newChat() }
                } label: {
                    Label("New Chat", systemImage: "square.and.pencil")
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(12)
                List {
                    Section("Threads") {
                        ForEach(chats.threads) { thread in row(thread) }
                    }
                }
                .listStyle(.sidebar)
            }
            .disabled(chats.isBusy)
            .navigationSplitViewColumnWidth(min: 175, ideal: 205, max: 320)
        } detail: {
            VStack(spacing: 0) {
                if let error = chats.storageError {
                    ErrorCallout(message: error).padding([.top, .horizontal], 12)
                }
                ChatView(thread: chats.thread)
                    .id(ObjectIdentifier(chats.thread))
                    .disabled(chats.isBusy)
            }
            .overlay {
                if chats.isBusy { ProgressView().controlSize(.small) }
            }
            .toolbar {
                ToolbarItem(placement: .navigation) { ChatModelSwitcher(preferences: preferences) }
                ToolbarItem { ChatThemeToggle(preferences: preferences) }
            }
        }
        .frame(minWidth: 250)
        .sheet(item: $renaming) { conversation in
            RenameThreadDialog(conversation: conversation) { title in
                Task { await chats.rename(conversation.id, to: title) }
            }
        }
    }

    private func row(_ thread: Conversation) -> some View {
        HStack {
            Button {
                Task { await chats.open(thread.id) }
            } label: {
                Label(thread.title, systemImage: "bubble.left")
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(thread.id == chats.selectedID ? .isSelected : [])
            Menu {
                Button("Rename") { renaming = thread }
                Button("Delete", role: .destructive) {
                    Task { await chats.delete(thread.id) }
                }
            } label: {
                Image(systemName: "ellipsis")
            }
            .labelStyle(.iconOnly)
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel("Thread actions")
        }
        .listRowBackground(thread.id == chats.selectedID ? Color.accentColor.opacity(0.15) : Color.clear)
    }
}

private struct RenameThreadDialog: View {
    let conversation: Conversation
    let save: (String) -> Void
    @State private var title: String
    @FocusState private var isFocused: Bool
    @Environment(\.dismiss) private var dismiss

    init(conversation: Conversation, save: @escaping (String) -> Void) {
        self.conversation = conversation
        self.save = save
        _title = State(initialValue: conversation.title)
    }

    var body: some View {
        let trimmed = ThesysConversations.trim(title)
        VStack(alignment: .leading, spacing: 14) {
            Text("Rename chat").font(.headline)
            TextField("Chat title", text: $title)
                .focused($isFocused)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") {
                    save(trimmed)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(trimmed.isEmpty || trimmed == conversation.title)
            }
        }
        .frame(minWidth: 300)
        .padding(20)
        .onAppear { isFocused = true }
    }
}
