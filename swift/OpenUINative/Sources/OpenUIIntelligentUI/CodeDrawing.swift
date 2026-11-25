import AppKit
import OpenUINative
import SwiftUI

@MainActor protocol CodePasteboardStorage {
    var changeCount: Int { get }
    var pasteboardItems: [NSPasteboardItem]? { get }
    func clearContents() -> Int
    func setData(_ data: Data?, forType type: NSPasteboard.PasteboardType) -> Bool
    func writeObjects(_ objects: [any NSPasteboardWriting]) -> Bool
}

extension NSPasteboard: CodePasteboardStorage {}

enum CodePasteboard {
    @MainActor static func snapshot(_ board: any CodePasteboardStorage) -> [NSPasteboardItem]? {
        guard let items = board.pasteboardItems else { return nil }
        var saved: [NSPasteboardItem] = []
        for item in items {
            let copy = NSPasteboardItem()
            for type in item.types {
                guard let data = item.data(forType: type), copy.setData(data, forType: type) else { return nil }
            }
            saved.append(copy)
        }
        return saved
    }

    @MainActor static func write(_ text: String, to board: any CodePasteboardStorage) -> Bool {
        let data = Data(text.utf8)
        let previous = board.changeCount
        // Reads can be refused too. Before clearing, make independent copies
        // of every type on every item, including data promised by another app.
        guard previous >= 0, let saved = snapshot(board), board.changeCount == previous else { return false }
        let cleared = board.clearContents()
        guard cleared > previous, board.changeCount == cleared else { return false }
        if board.setData(data, forType: .string) { return true }

        // Restore the saved items through the multi-item API while this write
        // still owns the board, so a simultaneous copy wins.
        if !saved.isEmpty, board.changeCount == cleared {
            _ = board.writeObjects(saved)
        }
        return false
    }
}

struct CodePasteboards {
    let open: @MainActor (NSPasteboard.Name) -> any CodePasteboardStorage
}

extension EnvironmentValues {
    @Entry var codePasteboards = CodePasteboards { NSPasteboard(name: $0) }
}

private func codeString(_ value: PropValue?) -> String {
    guard let value else { return "" }
    if case .scalar(.string(let text)) = value { return text }
    // JavaScript String(array) joins recursively. Keep the traversal iterative
    // and limited by the evaluated props already admitted by the runtime.
    var work = [([value][...], false)]
    var result = ""
    while let (values, comma) = work.popLast() {
        guard let first = values.first else { continue }
        if comma { result.append(",") }
        if values.count > 1 { work.append((values.dropFirst(), true)) }
        switch first {
        case .scalar(.string(let text)): result += text
        case .scalar(.number(let number)): result += JavaScript.numberString(number)
        case .scalar(.bool(let flag)): result += flag ? "true" : "false"
        case .scalar(.null): break
        case .list(let values): work.append((values[...], false))
        default: result += "[object Object]"
        }
    }
    return result
}

/// A code block with a language label and a copy button. The body shapes
/// 256-byte segments and uses a 512-entry width cache while indexing. The
/// header lays out a 4 KiB label prefix on one line, even for labels with spaces
/// that Chromium wraps. On lines longer than 256 bytes, combining marks, joined
/// emoji and right-to-left runs can draw incorrectly across segment boundaries.
struct CodeDrawing: View {
    let element: Element
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.codePasteboards) private var pasteboards
    @State private var feedback = CopyFeedback.idle
    @State private var reset: Task<Void, Never>?
    @State private var hovering = false

    private enum CopyFeedback {
        case idle, copied, failed

        var presentation: (title: String, label: String, icon: String) {
            switch self {
            case .idle: ("Copy", "Copy code", "copy")
            case .copied: ("Copied", "Copied", "check")
            case .failed: ("Couldn't copy", "Couldn't copy code", "x")
            }
        }
    }

    private var code: String { codeString(element.prop("code")) }

    private func write(_ text: String) -> Bool {
        CodePasteboard.write(text, to: pasteboards.open(.general))
    }

    private func copyCode() {
        feedback = write(code) ? .copied : .failed
        reset?.cancel()
        reset = Task {
            try? await Task.sleep(for: .milliseconds(1500))
            guard !Task.isCancelled else { return }
            feedback = .idle
            reset = nil
        }
    }

    private var header: some View {
        let language = codeString(element.prop("language"))
        let label = language.isEmpty ? "code" : Self.languageLabel(language)
        let copy = feedback.presentation
        return HStack(spacing: 0) {
            Text(verbatim: label)
                .lineLimit(1)
                .fixedSize()
                .textSelection(.enabled)
            Spacer(minLength: 0)
            Button(action: copyCode) {
                HStack(spacing: 5) {
                    if let icon = LucideGlyph.image(named: copy.icon) {
                        Image(nsImage: icon).resizable().frame(width: 14, height: 14)
                            .accessibilityHidden(true)
                    }
                    Text(verbatim: copy.title)
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .foregroundStyle((hovering ? theme.ink : theme.muted).color)
                .background(hovering ? theme.hover.color : .clear,
                            in: RoundedRectangle(cornerRadius: 6))
                .contentShape(Rectangle())
            }
            .fixedSize()
            .buttonStyle(.plain)
            .onHover { hovering = $0 }
            .accessibilityLabel(Text(verbatim: copy.label))
        }
        .font(.system(size: 12))
        .foregroundStyle(theme.muted.color)
        .padding(.leading, 14)
        .padding(.trailing, 8)
        .padding(.vertical, 6)
        // A narrow cell can be smaller than the header's minimum size.
        // Keep the scroll viewport and the outer clip at the allocated width.
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
    }

    private static func languageLabel(_ language: String) -> String {
        // Keep Text's one-line layout bounded even when the label pushes Copy
        // past the enclosing clip. Strip CSS-collapsible whitespace.
        var bytes: [UInt8] = []
        var space = false
        for byte in language.utf8 {
            if byte == 32 || byte == 9 || byte == 10 || byte == 13 {
                space = !bytes.isEmpty
            } else {
                if bytes.count >= 4096 && byte & 0xc0 != 0x80 { break }
                if space { bytes.append(32); space = false }
                bytes.append(byte)
            }
        }
        return String(decoding: bytes, as: UTF8.self)
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.radiusLg, style: .circular)
        VStack(spacing: 0) {
            header
            theme.border.color.frame(height: 1).accessibilityHidden(true)
            CodeBody(source: code, ink: theme.ink, write: write)
        }
        .padding(1)
        .clipContainer(to: shape.inset(by: 1))
        .background(theme.surface.color, in: shape)
        .overlay { shape.strokeBorder(theme.border.color, lineWidth: 1).allowsHitTesting(false) }
        .onDisappear {
            reset?.cancel()
            reset = nil
            feedback = .idle
        }
    }
}
