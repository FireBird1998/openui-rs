import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import Testing

extension CollectionDrawingTests {
    @Suite @MainActor struct TextInputDrawingTests {
        private func editor(_ label: String, in mounted: CollectionWindow) throws -> IntelligentTextEditor {
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

        @Test func textFieldsMountWithTheirPlaceholdersAndRowHeights() async throws {
            let mounted = try await CollectionWindow(#"""
            root = Col([Input("", "Single", null, "Type here"),
                TextArea("", "Short", "Multiple lines", 1), TextArea("", "Default"), TextArea("", "Long", null, 4.5)], 0)
            """#, height: 380)
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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

        @Test func drawingAndRedrawingInputsDoesNotInitializeFormFields() async throws {
            let mounted = try await CollectionWindow(#"""
            root = Col([form], 0)
            form = Form([Input("first", "Name"), TextArea("second", "Notes")])
            """#)
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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

        @Test(.requiresDisplay) func typingUpdatesBothEditorsAndBlockingPreventsNativeEdits() async throws {
            let mounted = try await CollectionWindow(#"""
            $name = ""
            $notes = ""
            root = Col([Input($name,"Name",null,"Your name"), TextArea($notes,"Notes","Add notes")], 0)
            """#)
            defer { mounted.window.close() }
            let name = try editor("Name", in: mounted)
            let notes = try editor("Notes", in: mounted)
            try await focus(name, in: mounted)
            let key = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: mounted.window.windowNumber,
                context: nil, characters: "a", charactersIgnoringModifiers: "a", isARepeat: false, keyCode: 0)
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

        @Test(.requiresDisplay) func compositionCommitsOnceAndNumberDraftResetsWhenFocusLeaves() async throws {
            let mounted = try await CollectionWindow(#"""
            $name = ""
            $amount = 3
            root = Col([Form([Input($name, "Name", null, null, {required: true}),
                Input($amount, "Amount", "number")])], 0)
            """#, height: 280)
            defer { mounted.window.close() }
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
            defer { mounted.window.close() }
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
