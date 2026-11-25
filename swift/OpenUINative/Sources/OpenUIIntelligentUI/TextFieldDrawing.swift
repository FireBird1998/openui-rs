import AppKit
import OpenUINative
import SwiftUI

struct IntelligentTextField: View {
    let input: TextInput
    @Environment(\.liveProgram) private var live
    @Environment(\.formName) private var form
    @Environment(\.formContext) private var context
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.intelligentTheme) private var theme
    @State private var focused = false
    @State private var focusRequest = false

    private static let font = NSFont.systemFont(ofSize: 16)
    private var validation: FormValidation? { context?.validation }

    var body: some View {
        let enabled = isEnabled && input.isEnabled(in: live)
        let required = input.isRequired(in: validation)
        let error = input.error(in: validation)
        let shape = RoundedRectangle(cornerRadius: theme.radiusMd, style: .circular)
        VStack(alignment: .leading, spacing: theme.controlGap) {
            if let label = input.label {
                Text(verbatim: label + (required ? " *" : ""))
                    .font(layoutFont(size: 14, weight: 560))
                    .lineHeight(.exact(points: 19.6))
                    .foregroundStyle(theme.ink.color)
                    .onTapGesture { if enabled { focusRequest.toggle() } }
                    .accessibilityHidden(true)
            }
            TextEditorBridge(
                text: input.text(in: live, form: form), label: input.label, placeholder: input.placeholder,
                error: error, required: required, enabled: enabled, multiline: input.rows != nil,
                numeric: input.numeric, focusRequest: focusRequest, font: Self.font,
                ink: NSColor(theme.ink.color), muted: NSColor(theme.muted.color),
                edit: { text in
                    guard isEnabled, input.edit(text, in: live, form: form, validation: validation) else { return nil }
                    return input.text(in: live, form: form)
                },
                blur: { input.validate(in: live, form: form, validation: validation) },
                focus: { focused = $0 }
            )
            .frame(height: input.rows.map { CGFloat($0) * 24 + 16 } ?? 40)
            .frame(idealWidth: 200, maxWidth: .infinity)
            .clipContainer(to: shape.inset(by: 1))
            .background(theme.ink.mixedWithTransparent(0.02).color, in: shape)
            .overlay {
                shape.strokeBorder(
                    (error != nil ? theme.danger : focused && enabled ? theme.focus : theme.ink.mixedWithTransparent(0.12)).color
                )
                .allowsHitTesting(false)
            }
            .opacity(enabled ? 1 : 0.55)
            if let error {
                HStack(spacing: 6) {
                    if let icon = LucideGlyph.image(named: "circle-alert") {
                        Image(nsImage: icon).resizable().frame(width: 14, height: 14).accessibilityHidden(true)
                    }
                    Text(verbatim: error).font(.system(size: 13)).lineHeight(.exact(points: 18))
                }
                .foregroundStyle(theme.danger.color)
            }
        }
    }
}

private struct TextEditorBridge: NSViewRepresentable {
    let text: String
    let label: String?
    let placeholder: String
    let error: String?
    let required: Bool
    let enabled: Bool
    let multiline: Bool
    let numeric: Bool
    let focusRequest: Bool
    let font: NSFont
    let ink: NSColor
    let muted: NSColor
    let edit: (String) -> String?
    let blur: () -> Void
    let focus: (Bool) -> Void

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        scroll.hasVerticalScroller = multiline
        scroll.autohidesScrollers = true
        scroll.verticalScrollElasticity = .none
        scroll.horizontalScrollElasticity = .none

        let editor = IntelligentTextEditor(frame: NSRect(x: 0, y: 0, width: 160, height: 40))
        editor.isRichText = false
        editor.allowsUndo = false
        editor.importsGraphics = false
        editor.usesAdaptiveColorMappingForDarkAppearance = false
        editor.drawsBackground = false
        editor.isAutomaticQuoteSubstitutionEnabled = false
        editor.isAutomaticDashSubstitutionEnabled = false
        editor.isAutomaticTextReplacementEnabled = false
        editor.isAutomaticSpellingCorrectionEnabled = false
        editor.isAutomaticTextCompletionEnabled = false
        editor.isHorizontallyResizable = !multiline
        editor.isVerticallyResizable = multiline
        editor.autoresizingMask = [.width, .height]
        editor.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        editor.textContainer?.widthTracksTextView = multiline
        editor.textContainer?.containerSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        editor.textContainer?.lineFragmentPadding = 0
        editor.textContainerInset = NSSize(width: 12, height: 8)
        let paragraph = NSMutableParagraphStyle()
        paragraph.minimumLineHeight = 24
        paragraph.maximumLineHeight = 24
        editor.defaultParagraphStyle = paragraph
        editor.multiline = multiline
        scroll.documentView = editor
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let editor = scroll.documentView as? IntelligentTextEditor else { return }
        editor.edit = edit
        editor.blur = blur
        editor.focus = focus
        if editor.font != font { editor.font = font }
        if editor.textColor != ink { editor.textColor = ink }
        if editor.insertionPointColor != ink { editor.insertionPointColor = ink }
        editor.updatePlaceholder(placeholder, color: muted)
        editor.numeric = numeric
        editor.setAccessibilityLabel(label ?? placeholder)
        editor.setAccessibilityPlaceholderValue(placeholder)
        editor.setAccessibilityRequired(required)
        editor.setAccessibilityEnabled(enabled)
        editor.setAccessibilityHelp(error)
        editor.synchronize(text, enabled: enabled)
        if editor.focusRequest != focusRequest {
            editor.focusRequest = focusRequest
            Task { @MainActor [weak editor] in
                guard let editor, editor.isEditable else { return }
                editor.window?.makeFirstResponder(editor)
            }
        }
    }
}

/// Keeps AppKit's selection, scrolling and IME handling. Only a completed,
/// admitted edit advances the committed display and undo history.
final class IntelligentTextEditor: NSTextView {
    var edit: ((String) -> String?)?
    var blur: (() -> Void)?
    var focus: ((Bool) -> Void)?
    var multiline = false
    var numeric = false
    var focusRequest = false

    private struct Replacement {
        let range: NSRange
        let text: String

        init(from current: String, to previous: String) {
            let a = current.unicodeScalars
            let b = previous.unicodeScalars
            var prefix = 0
            var start = 0
            for (left, right) in zip(a, b) {
                guard left == right else { break }
                prefix += 1
                start += left.value > 0xffff ? 2 : 1
            }
            let remaining = b.dropFirst(prefix)
            var suffix = 0
            var tail = 0
            for (left, right) in zip(a.dropFirst(prefix).reversed(), remaining.reversed()) {
                guard left == right else { break }
                suffix += 1
                tail += left.value > 0xffff ? 2 : 1
            }
            range = NSRange(location: start, length: current.utf16.count - start - tail)
            text = String(remaining.dropLast(suffix))
        }

        func applying(to current: String) -> String? {
            let current = current as NSString
            guard range.location <= current.length, range.length <= current.length - range.location else { return nil }
            return current.replacingCharacters(in: range, with: text)
        }
    }

    // Register admitted edits with the window. AppKit's raw storage edits
    // would also include IME updates and edits the binding has refused.
    override var undoManager: UndoManager? { nil }
    private var history: UndoManager? { window?.undoManager }
    var canUndoEdit: Bool { isEditable && (composition || history?.canUndo == true) }

    @objc func undo(_ sender: Any?) {
        guard canUndoEdit else { return }
        if composition {
            let selection = beforeEdit ?? committedSelection
            cancelComposition()
            show(committedText, selection: selection)
        } else {
            history?.undo()
        }
    }

    @objc func redo(_ sender: Any?) {
        if canRedoEdit { history?.redo() }
    }

    private var canRedoEdit: Bool { isEditable && !composition && history?.canRedo == true }

    override func validateUserInterfaceItem(_ item: NSValidatedUserInterfaceItem) -> Bool {
        switch item.action {
        case #selector(undo(_:)): canUndoEdit
        case #selector(redo(_:)): canRedoEdit
        default: super.validateUserInterfaceItem(item)
        }
    }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        let modifiers = event.modifierFlags.intersection([.command, .shift, .control, .option])
        guard window?.firstResponder === self, isEditable, event.charactersIgnoringModifiers?.lowercased() == "z",
              modifiers == .command || modifiers == [.command, .shift] else {
            return super.performKeyEquivalent(with: event)
        }
        if modifiers.contains(.shift) { redo(nil) } else { undo(nil) }
        return true
    }

    private var modelText = ""
    private var committedText = ""
    private var committedSelection = NSRange(location: 0, length: 0)
    private var hasNumericDraft = false
    // Range edits apply to the last history value even when blur or a binding
    // transform changed its presentation. Only the current value needs a base.
    private var historyText: String?
    private var beforeEdit: NSRange?
    private var updatingMarkedText = false
    private var composition = false
    private var discardingComposition = false
    private var showingValue = false
    private var placeholder = ""
    private var placeholderColor = NSColor.placeholderTextColor

    func synchronize(_ text: String, enabled: Bool) {
        let text = normalized(text)
        if !text.utf8.elementsEqual(modelText.utf8) {
            modelText = text
            history?.removeAllActions(withTarget: self)
            historyText = nil
            if !hasNumericDraft || composition {
                cancelComposition()
                show(hasNumericDraft ? committedText : text, selection: selectedRange())
            }
        }
        if !enabled, isEditable {
            cancelComposition()
            endDraft()
        }
        // NSTextView can change editability when selectability changes.
        isSelectable = enabled
        isEditable = enabled
    }

    func updatePlaceholder(_ text: String, color: NSColor) {
        if !placeholder.utf8.elementsEqual(text.utf8) || placeholderColor != color {
            placeholder = text
            placeholderColor = color
            needsDisplay = true
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        if string.isEmpty, !placeholder.isEmpty, let font {
            let origin = textContainerOrigin
            let rect = NSRect(x: origin.x, y: origin.y,
                              width: max(0, bounds.width - 2 * origin.x),
                              height: max(0, bounds.height - 2 * origin.y))
            (placeholder as NSString).draw(in: rect, withAttributes: [.font: font, .foregroundColor: placeholderColor])
        }
    }

    override func becomeFirstResponder() -> Bool {
        guard isEditable, super.becomeFirstResponder() else { return false }
        notifyFocus()
        return true
    }

    override func resignFirstResponder() -> Bool {
        guard super.resignFirstResponder() else { return false }
        if composition { unmarkText() }
        endDraft()
        blur?()
        notifyFocus()
        return true
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        if newWindow !== window {
            history?.removeAllActions(withTarget: self)
            historyText = nil
        }
        super.viewWillMove(toWindow: newWindow)
    }

    private func endDraft() {
        hasNumericDraft = false
        show(modelText, selection: selectedRange())
    }

    private func notifyFocus() {
        Task { @MainActor [weak self] in
            guard let self else { return }
            focus?(window?.firstResponder === self)
        }
    }

    // Return leaves a single-line value alone. Tab and Shift-Tab traverse the
    // window's key loop, including when the editor accepts multiple lines.
    override func insertNewline(_ sender: Any?) {
        if multiline { super.insertNewline(sender) }
    }

    override func insertTab(_ sender: Any?) { window?.selectNextKeyView(self) }
    override func insertBacktab(_ sender: Any?) { window?.selectPreviousKeyView(self) }

    override func shouldChangeText(inRanges ranges: [NSValue], replacementStrings replacements: [String]?) -> Bool {
        guard let replacements, fits(ranges.map(\.rangeValue), replacements: replacements),
              super.shouldChangeText(inRanges: ranges, replacementStrings: replacements) else { return false }
        if !composition, !updatingMarkedText { beforeEdit = selectedRange() }
        return true
    }

    private func fits(_ ranges: [NSRange], replacements: [String]) -> Bool {
        guard isEditable, replacements.count == ranges.count else { return false }
        // Refuse large pastes before AppKit edits storage. The completed edit
        // also checks exact UTF-8/JSON bytes and the runtime's store limits.
        let length = (string as NSString).length
        var retained = length
        for range in ranges {
            guard range.location <= length, range.length <= length - range.location,
                  range.length <= retained else { return false }
            retained -= range.length
        }
        var remaining = defaultHostByteLimit - retained
        for replacement in replacements {
            let length = replacement.utf16.count
            guard length <= remaining else { return false }
            remaining -= length
        }
        return remaining >= 0
    }

    override func didChangeText() {
        if !updatingMarkedText, !composition, !showingValue { finishEdit() }
        super.didChangeText()
        needsDisplay = true
    }

    override func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        guard canInsert(string, replacing: replacementRange) else { return }
        if !composition { beforeEdit = self.selectedRange() }
        updatingMarkedText = true
        super.setMarkedText(string, selectedRange: selectedRange, replacementRange: replacementRange)
        updatingMarkedText = false
        composition = hasMarkedText()
    }

    override func insertText(_ string: Any, replacementRange: NSRange) {
        guard canInsert(string, replacing: replacementRange) else { return }
        if composition {
            updatingMarkedText = true
            super.insertText(string, replacementRange: replacementRange)
            updatingMarkedText = false
            if !hasMarkedText() {
                composition = false
                finishEdit()
            }
        } else {
            super.insertText(string, replacementRange: replacementRange)
        }
    }

    private func canInsert(_ text: Any, replacing range: NSRange) -> Bool {
        guard let text = (text as? NSAttributedString)?.string ?? (text as? String) else { return false }
        let target = range.location == NSNotFound ? (composition ? markedRange() : selectedRange()) : range
        return fits([target], replacements: [text])
    }

    override func unmarkText() {
        super.unmarkText()
        guard composition, !updatingMarkedText else { return }
        composition = false
        finishEdit()
    }

    private func cancelComposition() {
        guard composition else { return }
        // AppKit may insert the marked text while discarding it.
        discardingComposition = true
        composition = false
        inputContext?.discardMarkedText()
        super.unmarkText()
        composition = false
        discardingComposition = false
        beforeEdit = nil
    }

    private func finishEdit() {
        guard !discardingComposition else { return }
        let previous = committedText
        let previousSelection = beforeEdit ?? committedSelection
        beforeEdit = nil
        let candidate = normalized(string)
        if candidate.utf8.elementsEqual(previous.utf8) {
            show(candidate, selection: selectedRange())
            return
        }
        if apply(candidate, selection: selectedRange()) {
            recordUndo(from: committedText, to: historyText ?? previous, selection: previousSelection)
        } else {
            show(previous, selection: previousSelection)
        }
    }

    private func apply(_ candidate: String, selection: NSRange) -> Bool {
        guard isEditable, let text = edit?(candidate) else { return false }
        modelText = normalized(text)
        hasNumericDraft = numeric
        show(hasNumericDraft ? candidate : modelText, selection: selection)
        return true
    }

    private func recordUndo(from current: String, to previous: String, selection: NSRange) {
        guard let history else { return }
        guard history.isUndoRegistrationEnabled else {
            history.removeAllActions(withTarget: self)
            historyText = nil
            return
        }
        let replacement = Replacement(from: current, to: previous)
        historyText = current
        history.registerUndo(withTarget: self) { editor in
            MainActor.assumeIsolated {
                guard let current = editor.historyText, let candidate = replacement.applying(to: current) else { return }
                let currentSelection = editor.committedSelection
                editor.historyText = candidate
                if editor.apply(candidate, selection: selection) {
                    editor.recordUndo(from: candidate, to: current, selection: currentSelection)
                }
            }
        }
    }

    private func show(_ text: String, selection: NSRange) {
        showingValue = true
        defer { showingValue = false }
        if !text.utf8.elementsEqual(string.utf8) { string = text }
        let count = (text as NSString).length
        let start = min(selection.location, count)
        let range = NSRange(location: start, length: min(selection.length, count - start))
        if range != selectedRange() { setSelectedRange(range) }
        committedText = text
        committedSelection = range
        needsDisplay = true
    }

    private func normalized(_ text: String) -> String {
        if multiline {
            return text.replacingOccurrences(of: "\r\n", with: "\n", options: .literal)
                .replacingOccurrences(of: "\r", with: "\n", options: .literal)
        }
        return text.replacingOccurrences(of: "\r", with: "", options: .literal)
            .replacingOccurrences(of: "\n", with: "", options: .literal)
    }
}
