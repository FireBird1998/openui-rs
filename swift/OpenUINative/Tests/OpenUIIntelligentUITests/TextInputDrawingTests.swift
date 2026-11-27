import AppKit
@testable import OpenUIIntelligentUI
@testable import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

extension CollectionDrawingTests {
    @Suite(NativeTestWatchdog()) @MainActor struct TextInputDrawingTests {
        private func editor(_ label: String, in mounted: CollectionWindow) throws -> IntelligentTextEditor {
            NativeTestWatchdog.beginWaiting("AppKit to find the text editor labelled \(label)")
            defer { NativeTestWatchdog.finishedWaiting() }
            var pending: [NSView] = [mounted.host]
            while let view = pending.popLast() {
                if let editor = view as? IntelligentTextEditor, editor.accessibilityLabel() == label { return editor }
                pending.append(contentsOf: view.subviews)
            }
            throw CollectionInputFailure(description: "No text editor labelled \(label)")
        }

        private func focus(_ editor: IntelligentTextEditor, in mounted: CollectionWindow) async throws {
            let clip = try #require(editor.enclosingScrollView?.contentView)
            let center = CGPoint(x: clip.bounds.midX, y: clip.bounds.midY)
            try await mounted.click(clip.convert(center, to: mounted.host))
            try #require(mounted.window.firstResponder === editor)
        }

        private func firstClickWindow(disabled: Bool = false, streaming: Bool = false,
                                      selectionText: Bool = false) async throws -> CollectionWindow {
            let name = selectionText ? "Alpha beta gamma" : "ABCDE"
            let notes = selectionText ? #"First\nAlpha beta gamma\nLast"# : #"First\nABCDE"#
            return try await CollectionWindow(#"""
            $name = "\#(name)"
            $amount = 12345
            root = Col([form], 0)
            form = Form([Input($name, "Name"), Input($amount, "Amount", "number"),
                TextArea("\#(notes)", "Notes")])
            """#, height: 280, streaming: streaming, makeKey: false, component: { element, path in
                guard disabled, element.typeName == "Form" else { return nil }
                return AnyView(IntelligentFormScope(element: element, path: path).disabled(true))
            })
        }

        private func click(_ field: IntelligentTextEditor, beforeCharacter index: Int,
                           in mounted: CollectionWindow) async throws {
            let target = try point(in: field, beforeCharacter: index, mounted: mounted)
            try #require(!mounted.window.isKeyWindow)
            try await mounted.click(target)
        }

        private func point(in field: IntelligentTextEditor, beforeCharacter index: Int,
                           mounted: CollectionWindow) throws -> CGPoint {
            NativeTestWatchdog.beginWaiting("AppKit to lay out the text editor and locate character \(index)")
            defer { NativeTestWatchdog.finishedWaiting() }
            let layout = try #require(field.layoutManager)
            let container = try #require(field.textContainer)
            layout.ensureLayout(for: container)
            let glyphs = layout.glyphRange(forCharacterRange: NSRange(location: index, length: 1),
                                           actualCharacterRange: nil)
            let rect = layout.boundingRect(forGlyphRange: glyphs, in: container)
            try #require(!rect.isEmpty)
            let origin = field.textContainerOrigin
            // The leading quarter of a glyph places the caret before it.
            let point = NSPoint(x: origin.x + rect.minX + rect.width / 4, y: origin.y + rect.midY)
            let target = field.convert(point, to: mounted.host)
            let clip = try #require(field.enclosingScrollView?.contentView)
            try #require(clip.bounds.contains(field.convert(point, to: clip)))
            return target
        }

        private func mouse(_ type: NSEvent.EventType, at point: CGPoint, clicks: Int = 1,
                           in mounted: CollectionWindow) throws -> NSEvent {
            try #require(NSEvent.mouseEvent(with: type, location: mounted.host.convert(point, to: nil),
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: mounted.window.windowNumber, context: nil, eventNumber: 0,
                clickCount: clicks, pressure: 1))
        }

        private func sendGesture(down: NSEvent, drag: NSEvent? = nil, up: NSEvent,
                                 in mounted: CollectionWindow, afterDown: () throws -> Void = {}) rethrows {
            NativeTestWatchdog.beginWaiting("AppKit to dispatch the text editor mouse gesture")
            defer { NativeTestWatchdog.finishedWaiting() }
            mounted.window.sendEvent(down)
            defer {
                if let drag { mounted.window.sendEvent(drag) }
                mounted.window.sendEvent(up)
            }
            try afterDown()
        }

        @Test(arguments: ["Name", "Amount", "Notes"])
        func firstClickFocusesTheEditorAndPlacesOnlyACaret(_ label: String) async throws {
            let mounted = try await firstClickWindow()
            defer { mounted.window.closeForTest() }
            let field = try editor(label, in: mounted)
            let index = label == "Notes" ? 8 : 2
            let version = mounted.live.version
            let state = mounted.live.state
            let text = field.string
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }
            #expect(mounted.live.acceptsInteractions)
            #expect(mounted.live.field(form: "form", name: "Notes") == nil)
            try #require(mounted.window.firstResponder !== field)
            #expect(field.selectedRange() == NSRange(location: 0, length: 0))

            try await click(field, beforeCharacter: index, in: mounted)

            #expect(mounted.window.firstResponder === field)
            #expect(field.selectedRange() == NSRange(location: index, length: 0))
            #expect(field.string == text)
            #expect(events.isEmpty)
            #expect(mounted.live.state == state)
            #expect(mounted.live.field(form: "form", name: "Notes") == nil)
            #expect(mounted.live.version == version)
            #expect(mounted.live.acceptsInteractions)
        }

        @Test(arguments: ["Name", "Amount", "Notes"], [false, true])
        func firstClickLeavesDisabledOrBlockedEditorsUnfocused(_ label: String, streaming: Bool) async throws {
            let mounted = try await firstClickWindow(disabled: !streaming, streaming: streaming)
            defer { mounted.window.closeForTest() }
            let field = try editor(label, in: mounted)
            let version = mounted.live.version
            let state = mounted.live.state
            let text = field.string
            let selection = field.selectedRange()
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }
            #expect(mounted.live.acceptsInteractions == !streaming)
            try #require(mounted.window.firstResponder !== field)

            try await click(field, beforeCharacter: label == "Notes" ? 8 : 2, in: mounted)

            #expect(mounted.window.firstResponder !== field)
            #expect(!field.isEditable && !field.isSelectable)
            #expect(field.selectedRange() == selection)
            #expect(field.string == text)
            #expect(events.isEmpty)
            #expect(mounted.live.state == state)
            #expect(mounted.live.field(form: "form", name: "Notes") == nil)
            #expect(mounted.live.version == version)
            #expect(mounted.live.acceptsInteractions == !streaming)
        }

        @Test(arguments: ["Name", "Amount", "Notes"], [1, 2, 3])
        func activatingDragAndMultiClickSelectText(_ label: String, clicks: Int) async throws {
            let mounted = try await firstClickWindow(selectionText: true)
            defer { mounted.window.closeForTest() }
            let field = try editor(label, in: mounted)
            let paragraphStart = label == "Notes" ? 6 : 0
            let index = paragraphStart + (label == "Amount" ? 0 : 7)
            let start = try point(in: field, beforeCharacter: index, mounted: mounted)
            let end = try point(in: field, beforeCharacter: index + 4, mounted: mounted)
            let down = try mouse(.leftMouseDown, at: start, clicks: clicks, in: mounted)
            let up = try mouse(.leftMouseUp, at: clicks == 1 ? end : start, clicks: clicks, in: mounted)
            let state = mounted.live.state
            let version = mounted.live.version
            let text = field.string
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }
            try #require(!mounted.window.isKeyWindow && mounted.window.firstResponder !== field)
            let drag = clicks == 1 ? try mouse(.leftMouseDragged, at: end, in: mounted) : nil
            try sendGesture(down: down, drag: drag, up: up, in: mounted)
            #expect(mounted.window.firstResponder === field)
            // WebKit's Source/WebCore/page/EventHandler.cpp:697–765 selects by word
            // or paragraph on multi-click, and lines 1117–1168 extend a drag selection.
            switch clicks {
            case 1:
                #expect(field.selectedRange() == NSRange(location: index, length: 4))
            case 2:
                let word = label == "Amount" ? NSRange(location: 0, length: 5)
                    : NSRange(location: paragraphStart + 6, length: 4)
                #expect(field.selectedRange() == word)
            default:
                let length = label == "Amount" ? 5 : (label == "Notes" ? 17 : 16)
                #expect(field.selectedRange() == NSRange(location: paragraphStart, length: length))
            }
            #expect(field.string == text)
            #expect(mounted.live.field(form: "form", name: "Notes") == nil)
            #expect(mounted.live.state == state && mounted.live.version == version)
            #expect(events.isEmpty && mounted.live.acceptsInteractions)
        }

        @Test func activatingAnotherEditorCommitsMarkedTextOnce() async throws {
            let mounted = try await firstClickWindow()
            defer { mounted.window.closeForTest() }
            let name = try editor("Name", in: mounted)
            let notes = try editor("Notes", in: mounted)
            try #require(mounted.window.makeFirstResponder(name))
            let state = mounted.live.state
            let version = mounted.live.version
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }
            name.setMarkedText("漢", selectedRange: NSRange(location: 1, length: 0),
                               replacementRange: NSRange(location: 0, length: 2))
            try #require(name.hasMarkedText() && name.string == "漢CDE")
            #expect(mounted.live.state == state && mounted.live.version == version)

            try await click(notes, beforeCharacter: 8, in: mounted)

            #expect(mounted.window.firstResponder === notes)
            #expect(notes.selectedRange() == NSRange(location: 8, length: 0))
            #expect(!name.hasMarkedText() && name.string == "漢CDE")
            #expect(name.canUndoEdit)
            #expect(mounted.live.value(of: "$name") == .string("漢CDE"))
            #expect(mounted.live.version == version + 1)
            #expect(events.isEmpty && mounted.live.acceptsInteractions)
        }

        @Test(arguments: [false, true])
        func activatingWithoutMovingFocusPreservesMarkedText(_ blocked: Bool) async throws {
            let mounted = try await firstClickWindow()
            defer { mounted.window.closeForTest() }
            let name = try editor("Name", in: mounted)
            let target = blocked ? try editor("Notes", in: mounted) : name
            try #require(mounted.window.makeFirstResponder(name))
            let markedSelection = NSRange(location: 0, length: 1)
            name.setMarkedText("漢", selectedRange: markedSelection,
                               replacementRange: NSRange(location: 0, length: 2))
            try #require(name.hasMarkedText() && name.string == "漢CDE")
            try #require(name.selectedRange() == markedSelection)
            try #require(!mounted.window.isKeyWindow)
            let point = try point(in: target, beforeCharacter: 3, mounted: mounted)
            let down = try mouse(.leftMouseDown, at: point, in: mounted)
            let up = try mouse(.leftMouseUp, at: point, in: mounted)
            let state = mounted.live.state
            let version = mounted.live.version
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }
            if blocked { mounted.live.isStreaming = true }

            try sendGesture(down: down, up: up, in: mounted)

            #expect(mounted.window.firstResponder === name)
            #expect(name.hasMarkedText() && name.string == "漢CDE")
            #expect(name.selectedRange() == (blocked ? markedSelection : NSRange(location: 1, length: 0)))
            #expect(mounted.live.state == state && mounted.live.version == version)
            #expect(events.isEmpty && mounted.live.isBlocked == blocked)
        }

        @Test(arguments: ["click", "Tab", "accessibility"])
        func aLiveBlockRefusesFocusBeforeRedraw(_ route: String) async throws {
            let mounted = try await firstClickWindow()
            defer { mounted.window.closeForTest() }
            let name = try editor("Name", in: mounted)
            let notes = try editor("Notes", in: mounted)
            mounted.window.autorecalculatesKeyViewLoop = false
            name.nextKeyView = notes
            notes.nextKeyView = name
            try #require(mounted.window.makeFirstResponder(name))
            if route == "Tab" {
                name.insertTab(nil)
                try #require(mounted.window.firstResponder === notes, "The enabled key loop reaches Notes")
                try #require(mounted.window.makeFirstResponder(name))
            }
            let point = try point(in: notes, beforeCharacter: 8, mounted: mounted)
            let down = try mouse(.leftMouseDown, at: point, in: mounted)
            let up = try mouse(.leftMouseUp, at: point, in: mounted)
            let selection = notes.selectedRange()
            let state = mounted.live.state
            let version = mounted.live.version
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }

            mounted.live.isStreaming = true
            try #require(notes.isEditable, "SwiftUI has yet to redraw")
            switch route {
            case "click":
                try sendGesture(down: down, up: up, in: mounted)
            case "Tab": name.insertTab(nil)
            default: notes.setAccessibilityFocused(true)
            }

            #expect(mounted.window.firstResponder !== notes)
            #expect(notes.selectedRange() == selection)
            #expect(mounted.live.state == state && mounted.live.version == version)
            #expect(events.isEmpty && mounted.live.isBlocked)
        }

        @Test(arguments: [false, true])
        func blockingBetweenMouseDownAndMouseUpRestoresFocusAndCaret(_ redrawBeforeUp: Bool) async throws {
            let mounted = try await firstClickWindow()
            defer { mounted.window.closeForTest() }
            let notes = try editor("Notes", in: mounted)
            let point = try point(in: notes, beforeCharacter: 8, mounted: mounted)
            let down = try mouse(.leftMouseDown, at: point, in: mounted)
            let up = try mouse(.leftMouseUp, at: point, in: mounted)
            let responder = mounted.window.firstResponder
            let selection = notes.selectedRange()
            try #require(responder !== notes)
            try #require(selection == NSRange(location: 0, length: 0))
            let state = mounted.live.state
            let version = mounted.live.version
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }

            NativeTestWatchdog.waitingFor("AppKit to dispatch mouse-down on Notes")
            mounted.window.sendEvent(down)
            do {
                defer {
                    NativeTestWatchdog.waitingFor("AppKit to dispatch mouse-up on Notes")
                    mounted.window.sendEvent(up)
                }
                try #require(mounted.window.firstResponder === notes)
                try #require(notes.selectedRange() == NSRange(location: 8, length: 0))
                mounted.live.isStreaming = true
                if redrawBeforeUp { await mounted.settle() }
            }
            #expect(notes.isEditable == !redrawBeforeUp)
            #expect(mounted.window.firstResponder === responder)
            #expect(notes.selectedRange() == selection)
            await mounted.settle()

            #expect(!notes.isEditable && !notes.isSelectable)
            #expect(mounted.window.firstResponder !== notes)
            #expect(mounted.window.firstResponder === responder)
            #expect(notes.selectedRange() == selection)
            #expect(mounted.live.state == state && mounted.live.version == version)
            #expect(events.isEmpty && mounted.live.isBlocked)
        }

        @Test(arguments: [false, true])
        func aMidClickBlockRefusesFocusTransferAndRestoresCarets(_ redrawBeforeUp: Bool) async throws {
            let mounted = try await firstClickWindow()
            defer { mounted.window.closeForTest() }
            let name = try editor("Name", in: mounted)
            let notes = try editor("Notes", in: mounted)
            try #require(mounted.window.makeFirstResponder(name))
            try #require(!mounted.window.isKeyWindow && mounted.window.firstResponder === name)
            let state = mounted.live.state
            let version = mounted.live.version
            var events: [HostEvent] = []
            mounted.live.onHostEvent = { events.append($0) }
            let markedSelection = NSRange(location: 0, length: 1)
            name.setMarkedText("漢", selectedRange: markedSelection,
                               replacementRange: NSRange(location: 0, length: 2))
            try #require(name.hasMarkedText() && name.selectedRange() == markedSelection)
            notes.setSelectedRange(NSRange(location: 2, length: 0))
            let notesSelection = notes.selectedRange()
            let point = try point(in: notes, beforeCharacter: 8, mounted: mounted)
            let down = try mouse(.leftMouseDown, at: point, in: mounted)
            let up = try mouse(.leftMouseUp, at: point, in: mounted)

            NativeTestWatchdog.waitingFor("AppKit to dispatch mouse-down on Notes while Name is composing")
            mounted.window.sendEvent(down)
            do {
                defer {
                    NativeTestWatchdog.waitingFor("AppKit to dispatch mouse-up on Notes while Name is composing")
                    mounted.window.sendEvent(up)
                }
                #expect(mounted.window.firstResponder === name)
                #expect(notes.selectedRange() == NSRange(location: 8, length: 0))
                mounted.live.isStreaming = true
                // A response block resigns the focused editor.
                if redrawBeforeUp {
                    #expect(mounted.window.firstResponder === name)
                    await mounted.settle()
                    #expect(!name.isEditable && !notes.isEditable)
                    #expect(mounted.window.firstResponder !== name)
                    #expect(mounted.window.firstResponder !== notes)
                }
            }

            #expect(name.isEditable == !redrawBeforeUp && notes.isEditable == !redrawBeforeUp)
            #expect((mounted.window.firstResponder === name) == !redrawBeforeUp)
            #expect(mounted.window.firstResponder !== notes)
            #expect(name.selectedRange() == markedSelection)
            #expect(notes.selectedRange() == notesSelection)
            if !redrawBeforeUp { #expect(name.hasMarkedText()) }
            #expect(mounted.live.state == state && mounted.live.version == version)
            #expect(events.isEmpty && mounted.live.isBlocked)

            await mounted.settle()

            #expect(!name.isEditable && !notes.isEditable)
            #expect(mounted.window.firstResponder !== name)
            #expect(mounted.window.firstResponder !== notes)
            #expect(name.selectedRange() == markedSelection)
            #expect(notes.selectedRange() == notesSelection)
            #expect(mounted.live.state == state && mounted.live.version == version)
            #expect(events.isEmpty && mounted.live.isBlocked)
        }

        @Test func textFieldsMountWithTheirPlaceholdersAndRowHeights() async throws {
            let mounted = try await CollectionWindow(#"""
            root = Col([Input("", "Single", null, "Type here"),
                TextArea("", "Short", "Multiple lines", 1), TextArea("", "Default"), TextArea("", "Long", null, 4.5)], 0)
            """#, height: 380)
            defer { mounted.window.closeForTest() }
            let single = try editor("Single", in: mounted)
            let short = try editor("Short", in: mounted)
            #expect(single.accessibilityPlaceholderValue() == "Type here")
            #expect(short.accessibilityPlaceholderValue() == "Multiple lines")
            for (label, expected) in [("Single", 40), ("Short", 64), ("Default", 88), ("Long", 136)] {
                let field = try editor(label, in: mounted)
                #expect(field.enclosingScrollView?.frame.height == CGFloat(expected))
                #expect(field.string.isEmpty)
            }
        }

        @Test func nativeEditsAndUndoRespectRefusalsAndExternalWrites() async throws {
            let mounted = try await CollectionWindow(#"""
            $name = "First"
            root = Col([Input($name, "Name")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let input = try editor("Name", in: mounted)
            func replace(with text: String) -> Bool {
                input.performValidatedReplacement(in: NSRange(location: 0, length: (input.string as NSString).length),
                    with: NSAttributedString(string: text))
            }
            #expect(replace(with: "Accepted"))
            #expect(mounted.live.value(of: "$name") == .string("Accepted"))
            let selection = input.selectedRange()
            let version = mounted.live.version
            #expect(!replace(with: String(repeating: "x", count: defaultHostByteLimit + 1)))
            #expect(input.selectedRange() == selection)
            #expect(input.string == "Accepted" && mounted.live.version == version)
            await mounted.settle()
            input.undo(nil)
            #expect(input.string == "First")
            #expect(mounted.live.value(of: "$name") == .string("First"))
            input.redo(nil)
            #expect(input.string == "Accepted")
            #expect(mounted.live.value(of: "$name") == .string("Accepted"))
            try mounted.live.set("$name", to: .string("External"))
            await mounted.settle()
            #expect(input.string == "External")
            #expect(!input.canUndoEdit)
        }

        @Test func aBindingTransformRefusalRestoresTheEditorAndLeavesNoUndoEntry() async throws {
            let mounted = try await CollectionWindow(#"""
            $name = "Seed"
            $padding = ""
            root = Col([Input($name = $value + $value + $padding, "Name")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let input = try editor("Name", in: mounted)
            #expect(input.performValidatedReplacement(in: NSRange(location: 0, length: 4),
                with: NSAttributedString(string: "a")))
            #expect(input.string == "aa")
            #expect(mounted.live.value(of: "$name") == .string("aa"))
            await mounted.settle()
            try mounted.live.set("$name", to: .string("Original"))
            try mounted.live.set("$padding", to: .string(String(repeating: "x", count: 5 << 20)))
            await mounted.settle()
            let before = mounted.live.version
            let selection = NSRange(location: 2, length: 3)
            input.setSelectedRange(selection)
            // A few input bytes make another copy of 5 MiB of existing
            // state. The session refuses that result under its total limit.
            _ = input.performValidatedReplacement(in: selection, with: NSAttributedString(string: "new"))
            #expect(input.string == "Original")
            #expect(input.selectedRange() == selection)
            #expect(!input.canUndoEdit)
            #expect(mounted.live.value(of: "$name") == .string("Original"))
            #expect(mounted.live.version == before)
        }

        @Test func windowUndoReversesUnicodeEditsAfterAnotherFieldChangesExternally() async throws {
            let mounted = try await CollectionWindow(#"""
            $name = "A👩🏽B"
            $other = "old"
            root = Col([Input($name, "Name"), TextArea($other, "Other")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let history = try #require(mounted.window.undoManager)
            let name = try editor("Name", in: mounted)
            let other = try editor("Other", in: mounted)
            name.insertText("A👩‍💻B", replacementRange: NSRange(location: 0, length: name.string.utf16.count))
            await mounted.settle()
            #expect(mounted.live.value(of: "$name") == .string("A👩‍💻B"))
            other.insertText("typed", replacementRange: NSRange(location: 0, length: other.string.utf16.count))
            await mounted.settle()
            try mounted.live.set("$other", to: .string("external"))
            await mounted.settle()
            history.undo()
            #expect(name.string == "A👩🏽B" && mounted.live.value(of: "$name") == .string("A👩🏽B"))
            #expect(other.string == "external" && mounted.live.value(of: "$other") == .string("external"))
            history.redo()
            #expect(name.string == "A👩‍💻B" && mounted.live.value(of: "$name") == .string("A👩‍💻B"))
        }

        @Test func editorKeepsMarkedTextLocalAndTracksOnlyTheCommittedEdit() async throws {
            let mounted = try await CollectionWindow(#"""
            $notes = ""
            root = Col([TextArea($notes, "Notes")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let notes = try editor("Notes", in: mounted)
            let automatic = NSRange(location: NSNotFound, length: 0)
            let caret = NSRange(location: 1, length: 0)
            let version = mounted.live.version
            notes.setMarkedText("k", selectedRange: caret, replacementRange: automatic)
            notes.setMarkedText("か", selectedRange: caret, replacementRange: automatic)
            #expect(notes.string == "か" && notes.hasMarkedText())
            #expect(mounted.live.version == version)
            notes.insertText("漢", replacementRange: automatic)
            #expect(notes.string == "漢" && !notes.hasMarkedText())
            #expect(mounted.live.value(of: "$notes") == .string("漢"))
            #expect(mounted.live.version == version + 1)
        }

        @Test func anExternalWriteDiscardsMarkedTextWithoutWritingItBack() async throws {
            let mounted = try await CollectionWindow(#"""
            $v = "base"
            root = Col([Input($v, "A")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let input = try editor("A", in: mounted)
            input.setSelectedRange(NSRange(location: 4, length: 0))
            let version = mounted.live.version
            input.setMarkedText("k", selectedRange: NSRange(location: 1, length: 0),
                                replacementRange: NSRange(location: NSNotFound, length: 0))
            #expect(input.string == "basek" && input.hasMarkedText())
            #expect(mounted.live.version == version)
            try mounted.live.set("$v", to: .string("EXT"))
            await mounted.settle()
            #expect(input.string == "EXT" && !input.hasMarkedText())
            #expect(mounted.live.value(of: "$v") == .string("EXT"))
            #expect(mounted.live.version == version + 1)
        }

        @Test func blockingAnEditorDiscardsCompositionAndMakesBothEditorsReadOnly() async throws {
            let mounted = try await CollectionWindow(#"""
            $v = "base"
            root = Col([Input($v, "A"), TextArea("", "Notes")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let input = try editor("A", in: mounted)
            let area = try editor("Notes", in: mounted)
            #expect(input.isEditable && area.isEditable)
            input.setMarkedText("k", selectedRange: NSRange(location: 1, length: 0),
                                replacementRange: NSRange(location: NSNotFound, length: 0))
            #expect(input.hasMarkedText())
            let version = mounted.live.version
            mounted.live.isStreaming = true
            await mounted.settle()
            #expect(!input.isEditable && !area.isEditable)
            #expect(input.string == "base" && !input.hasMarkedText())
            #expect(mounted.live.value(of: "$v") == .string("base"))
            #expect(mounted.live.version == version)
        }

        @Test func numericDraftSurvivesAnExternalWriteUntilBlur() async throws {
            let mounted = try await CollectionWindow(#"""
            $v = 3
            root = Col([Input($v, "Amount", "number")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let amount = try editor("Amount", in: mounted)
            try #require(mounted.window.makeFirstResponder(amount))
            amount.insertText("1.", replacementRange: NSRange(location: 0, length: 1))
            await mounted.settle()
            #expect(amount.string == "1." && mounted.live.value(of: "$v") == .number(1))
            try mounted.live.set("$v", to: .number(42))
            await mounted.settle()
            #expect(amount.string == "1." && mounted.live.value(of: "$v") == .number(42))
            let version = mounted.live.version
            try #require(mounted.window.makeFirstResponder(nil))
            #expect(amount.string == "42")
            #expect(mounted.live.version == version)
        }

        @Test func blurringTheMountedEditorChecksItsFormRules() async throws {
            let mounted = try await CollectionWindow(#"""
            root = Col([Form([Input("", "Email", null, null, {required: true})])], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let input = try editor("Email", in: mounted)
            try #require(mounted.window.makeFirstResponder(input))
            #expect(input.isAccessibilityRequired())
            #expect(input.accessibilityHelp() == nil)
            try #require(mounted.window.makeFirstResponder(nil))
            await mounted.settle()
            #expect(input.accessibilityHelp() == "This field is required")
            input.insertText("Ada", replacementRange: NSRange(location: 0, length: 0))
            await mounted.settle()
            #expect(input.accessibilityHelp() == nil)
        }

        @Test func aFieldErrorDrawsTheSmallDangerIcon() async throws {
            let validation = FormValidation()
            let rules = Validation.rules(.record([Prop(name: "required", value: .scalar(.bool(true)))]))
            validation.validate("Email", value: .string(""), rules: rules)
            let tint = IntelligentTheme.light.danger.color
            NativeTestWatchdog.waitingFor("SwiftUI to render the field error icon")
            let icon = try #require(ImageRenderer(content: IntelligentTextField.errorIcon(tint: tint)).nsImage)
            #expect(icon.size == CGSize(width: 14, height: 14))

            let picture = try await LayoutPicture(#"""
            root = Col([Input("", "Email", null, null, {required: true}), Icon("circle-alert", "sm")], 0, "start")
            """#, formContext: FormContext(validation: validation))
            let field = try picture.rect("/children/0")
            let reference = try picture.rect("/children/1")
            // The 18-point error line centres the icon vertically in the last row.
            let frame = CGRect(x: field.minX, y: field.maxY - (18 + reference.height) / 2,
                               width: reference.width, height: reference.height)
            let expected = try picture.alpha(in: reference)
            let actual = try picture.alpha(in: frame)
            #expect(actual.count == expected.count && !zip(actual, expected).contains { abs(Int($0) - Int($1)) > 1 })

            let strongest = try #require(expected.indices.max { expected[$0] < expected[$1] })
            let width = Int(frame.width)
            let paint = try picture.pixel(Int(frame.minX) + strongest % width, Int(frame.minY) + strongest / width)
            let danger = try LayoutPicture.swatch(tint)
            #expect(paint.alphaComponent > 0.35)
            #expect(abs(paint.redComponent - danger.redComponent) < 0.04)
            #expect(abs(paint.greenComponent - danger.greenComponent) < 0.04)
            #expect(abs(paint.blueComponent - danger.blueComponent) < 0.04)
        }

        @Test func drawingAndRedrawingInputsDoesNotInitializeFormFields() async throws {
            let mounted = try await CollectionWindow(#"""
            root = Col([form], 0)
            form = Form([Input("first", "Name"), TextArea("second", "Notes")])
            """#)
            defer { mounted.window.closeForTest() }
            #expect(try editor("Name", in: mounted).string == "first")
            #expect(try editor("Notes", in: mounted).string == "second")
            for name in ["Name", "Notes"] {
                #expect(mounted.live.field(form: "form", name: name) == nil)
                #expect(mounted.live.field(form: nil, name: name) == nil)
            }
            try mounted.live.setField(form: "form", name: "Name", to: .string("external"))
            let version = mounted.live.version
            await mounted.settle()
            #expect(try editor("Name", in: mounted).string == "external")
            #expect(mounted.live.field(form: "form", name: "Notes") == nil)
            #expect(mounted.live.version == version)
        }

        @Test func aDisabledAncestorRefusesTheViewEditWhileTheModelAcceptsEdits() async throws {
            let mounted = try await CollectionWindow(#"""
            root = Col([gate], 0)
            gate = Form([Input("original", "Name")])
            """#, component: { element, path in
                guard element.typeName == "Form" else { return nil }
                return AnyView(IntelligentFormScope(element: element, path: path).disabled(true))
            })
            defer { mounted.window.closeForTest() }
            let input = try editor("Name", in: mounted)
            #expect(mounted.live.acceptsInteractions)
            #expect(!input.isEditable)
            let edit = try #require(input.edit)
            let version = mounted.live.version
            // An edit delivered to the bridge must also respect the ancestor.
            #expect(edit("ignored") == nil)
            #expect(mounted.live.field(form: "gate", name: "Name") == nil)
            #expect(mounted.live.version == version)
        }

        @Test func typingUpdatesBothEditorsAndBlockingPreventsNativeEdits() async throws {
            let mounted = try await CollectionWindow(#"""
            $name = ""
            $notes = ""
            root = Col([Input($name,"Name",null,"Your name"), TextArea($notes,"Notes","Add notes")], 0)
            """#)
            defer { mounted.window.closeForTest() }
            let name = try editor("Name", in: mounted)
            let notes = try editor("Notes", in: mounted)
            try await focus(name, in: mounted)
            let key = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: mounted.window.windowNumber,
                context: nil, characters: "a", charactersIgnoringModifiers: "a", isARepeat: false, keyCode: 0)
            NativeTestWatchdog.waitingFor("AppKit to dispatch the typed key to Name")
            mounted.window.sendEvent(try #require(key))
            #expect(mounted.live.value(of: "$name") == .string("a"))
            name.insertText("da", replacementRange: NSRange(location: NSNotFound, length: 0))
            #expect(mounted.live.value(of: "$name") == .string("ada"))
            try await focus(notes, in: mounted)
            notes.insertText("One\nTwo", replacementRange: NSRange(location: NSNotFound, length: 0))
            #expect(mounted.live.value(of: "$notes") == .string("One\nTwo"))
            try mounted.live.set("$name", to: .string("Grace"))
            try mounted.live.set("$notes", to: .string("External"))
            await mounted.settle()
            #expect(name.string == "Grace" && notes.string == "External")
            mounted.live.isStreaming = true
            await mounted.settle()
            #expect(!name.isEditable && !notes.isEditable)
            let version = mounted.live.version
            for field in [name, notes] {
                field.insertText("ignored", replacementRange: NSRange(location: NSNotFound, length: 0))
            }
            #expect(name.string == "Grace" && notes.string == "External")
            #expect(mounted.live.version == version)
            mounted.live.isStreaming = false
            await mounted.settle()
            #expect(name.isEditable && notes.isEditable)
            try await focus(notes, in: mounted)
        }

        @Test func compositionCommitsOnceAndNumberDraftResetsWhenFocusLeaves() async throws {
            let mounted = try await CollectionWindow(#"""
            $name = ""
            $amount = 3
            root = Col([Form([Input($name, "Name", null, null, {required: true}),
                Input($amount, "Amount", "number")])], 0)
            """#, height: 280)
            defer { mounted.window.closeForTest() }
            let name = try editor("Name", in: mounted)
            let amount = try editor("Amount", in: mounted)
            try await focus(name, in: mounted)
            #expect(name.isAccessibilityRequired())
            let automatic = NSRange(location: NSNotFound, length: 0)
            let version = mounted.live.version
            name.setMarkedText("k", selectedRange: NSRange(location: 1, length: 0), replacementRange: automatic)
            name.setMarkedText("か", selectedRange: NSRange(location: 1, length: 0), replacementRange: automatic)
            #expect(name.hasMarkedText())
            #expect(mounted.live.version == version)
            name.insertText("漢", replacementRange: automatic)
            #expect(!name.hasMarkedText())
            #expect(mounted.live.value(of: "$name") == .string("漢"))
            #expect(mounted.live.version == version + 1)
            await mounted.settle()
            name.undo(nil)
            #expect(mounted.live.value(of: "$name") == .string(""))
            try await focus(amount, in: mounted)
            #expect(name.accessibilityHelp() == "This field is required")
            amount.insertText("1.", replacementRange: NSRange(location: 0, length: 1))
            await mounted.settle()
            #expect(amount.string == "1." && mounted.live.value(of: "$amount") == .number(1))
            try await focus(name, in: mounted)
            #expect(amount.string == "1")
            #expect(name.accessibilityHelp() == "This field is required")
            name.insertText("Valid", replacementRange: automatic)
            await mounted.settle()
            #expect(name.accessibilityHelp() == nil)
        }

        @Test func textViewportsRejectHitsOnOverflowingNativeChildren() async throws {
            let mounted = try await CollectionWindow(#"""
            root = Col([TextArea("Row 1\nRow 2\nRow 3\nRow 4\nRow 5\nRow 6\nRow 7\nRow 8","Scroll",null,2),
                Spacer(), Input("", "Single")], 16)
            """#, width: 280, height: 260)
            defer { mounted.window.closeForTest() }
            func hit(_ point: CGPoint) -> NSView? {
                mounted.host.hitTest(mounted.host.convert(point, to: mounted.host.superview))
            }
            let area = try editor("Scroll", in: mounted)
            let clip = try #require(area.enclosingScrollView?.contentView)
            let viewport = clip.convert(clip.bounds, to: mounted.host)
            let document = area.convert(area.bounds, to: mounted.host)
            #expect(hit(CGPoint(x: viewport.midX, y: viewport.midY)) === area)
            let outside = [viewport.maxY + 2, viewport.minY - 2]
                .map { CGPoint(x: viewport.midX, y: $0) }
                .first { document.contains($0) }
            let overflow = try #require(outside, "The document should extend past the viewport")
            #expect(hit(overflow) !== area)

            let single = try editor("Single", in: mounted)
            let singleClip = try #require(single.enclosingScrollView?.contentView)
            let singleBounds = singleClip.convert(singleClip.bounds, to: mounted.host)
            #expect(hit(CGPoint(x: singleBounds.midX, y: singleBounds.midY)) === single)
            #expect(hit(CGPoint(x: singleBounds.minX + 1, y: singleBounds.minY + 1)) !== single)
        }

    }
}
