import AppKit
@testable import OpenUIIntelligentUI
import OpenUINative
import SwiftUI
import SwiftTestSupport
import Testing

extension CollectionDrawingTests {
    @Suite(NativeTestWatchdog()) @MainActor struct CodeDrawingTests {
        private func copyLabels(in mounted: CollectionWindow) -> [String] {
            mounted.buttonLabels.filter { ["Copy code", "Copied", "Couldn't copy code"].contains($0) }
        }

        private func clickCopy(in mounted: CollectionWindow, labeled label: String) async throws {
            let frame = try buttonFrame(in: mounted, labeled: label)
            try await mounted.click(CGPoint(x: frame.midX, y: frame.midY))
        }

        private func buttonFrame(in mounted: CollectionWindow, labeled label: String) throws -> CGRect {
            let button = try #require(mounted.accessibleButtons.first { $0.accessibilityLabel?() == label })
            let frame = try #require(button.accessibilityFrame?())
            return mounted.host.convert(mounted.window.convertFromScreen(frame), from: nil)
        }

        private final class Board: CodePasteboardStorage {
            var pasteboardItems: [NSPasteboardItem]? = []
            var changeCount = 0
            var writes = 0
            var refuseData = false

            func clearContents() -> Int {
                pasteboardItems = []
                changeCount += 1
                return changeCount
            }

            func setData(_ data: Data?, forType type: NSPasteboard.PasteboardType) -> Bool {
                writes += 1
                guard !refuseData, let data else { return false }
                let item = NSPasteboardItem()
                guard item.setData(data, forType: type) else { return false }
                pasteboardItems = [item]
                return true
            }

            func writeObjects(_ objects: [any NSPasteboardWriting]) -> Bool {
                writes += 1
                pasteboardItems = objects.compactMap { $0 as? NSPasteboardItem }
                return pasteboardItems?.count == objects.count
            }
        }

        private func install(_ board: Board, clock: ManualClock? = nil, in mounted: CollectionWindow) async {
            var root = AnyView(mounted.host.rootView.environment(\.codePasteboards, CodePasteboards {
                #expect($0 == .general)
                return board
            }))
            if let clock { root = AnyView(root.environment(\.codeFeedbackClock, clock)) }
            mounted.host.rootView = root
            await mounted.settle()
        }

        /// Waits for the copy feedback's reset to sleep until exactly these
        /// controlled-clock deadlines, in milliseconds.
        private func waitForReset(_ clock: ManualClock, until deadlines: [Int64], in mounted: CollectionWindow) async throws {
            let armed = try await mounted.waitForLayout("the Code copy reset to sleep until \(deadlines) ms",
                                                        for: .seconds(5)) { clock.deadlines == deadlines }
            #expect(armed, "Reset deadlines \(clock.deadlines), expected \(deadlines)")
        }

        private func waitForIdleCopyLabel(in mounted: CollectionWindow) async throws {
            try await mounted.waitForLayout("the Code copy button to return to Copy code", for: .seconds(5)) {
                copyLabels(in: mounted) == ["Copy code"]
            }
            #expect(copyLabels(in: mounted) == ["Copy code"])
        }

        private func pixels(in mounted: CollectionWindow, rect: CGRect, darkInk: Bool = true) throws -> Set<Int> {
            let host = mounted.host
            let bitmap = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
            host.cacheDisplay(in: host.bounds, to: bitmap)
            let scale = CGFloat(bitmap.pixelsWide) / host.bounds.width
            let left = Int((rect.minX * scale).rounded())
            let top = Int(((host.isFlipped ? rect.minY : host.bounds.maxY - rect.maxY) * scale).rounded())
            let width = Int((rect.width * scale).rounded(.down))
            let height = Int((rect.height * scale).rounded(.down))
            try #require(left >= 0 && top >= 0 && left + width <= bitmap.pixelsWide && top + height <= bitmap.pixelsHigh)
            var ink: Set<Int> = []
            for y in 0..<height {
                for x in 0..<width {
                    guard let color = bitmap.colorAt(x: left + x, y: top + y)?.usingColorSpace(.sRGB),
                          color.alphaComponent > 0.5 else { continue }
                    let matches = darkInk ? max(color.redComponent, color.greenComponent, color.blueComponent) < 0.73
                                          : min(color.redComponent, color.greenComponent, color.blueComponent) > 0.73
                    if matches {
                        ink.insert(y * width + x)
                    }
                }
            }
            return ink
        }

        private func feedbackPixels(_ label: String, in mounted: CollectionWindow) throws -> (icon: Set<Int>, title: Set<Int>) {
            let frame = try buttonFrame(in: mounted, labeled: label)
            let icon = CGRect(x: frame.minX + 8, y: frame.midY - 7, width: 14, height: 14)
            let title = CGRect(x: frame.minX + 27, y: frame.midY - 7, width: 70, height: 14)
            // The title is right-aligned against the window edge, so extend the
            // sampled area toward that edge only as far as the button goes.
            let visibleTitle = title.intersection(CGRect(x: title.minX, y: title.minY,
                                                        width: frame.maxX - title.minX - 7, height: 14))
            return (try pixels(in: mounted, rect: icon), try pixels(in: mounted, rect: visibleTitle))
        }

        private func bodyPixels(in mounted: CollectionWindow, rows: CGFloat = 1, darkInk: Bool = true) throws -> Set<Int> {
            let scroll = try #require(mounted.scrollView)
            let clip = scroll.contentView
            let rect = mounted.host.convert(clip.bounds, from: clip)
            let height = 20.8 * rows
            let y = mounted.host.isFlipped ? rect.minY + 14 : rect.maxY - 14 - height
            return try pixels(in: mounted, rect: CGRect(x: rect.minX + 4, y: y, width: rect.width - 8, height: height),
                              darkInk: darkInk)
        }

        @Test func codeCopySendsVerbatimTextAndResets() async throws {
            let board = Board()
            let clock = ManualClock()
            let mounted = try await CollectionWindow(#"root = Col([Code("  **literal**\r\n\tprint(\"héllo 🐦 e\u0301\")\r\u0000\u001b\u007fאב\u202e\n","Swift")],0)"#, mode: .light)
            defer { mounted.window.closeForTest() }
            await install(board, clock: clock, in: mounted)
            let idle = try feedbackPixels("Copy code", in: mounted)

            try await clickCopy(in: mounted, labeled: "Copy code")
            #expect(board.pasteboardItems?.first?.data(forType: .string) == Data("  **literal**\r\n\tprint(\"héllo 🐦 e\u{0301}\")\r\u{0}\u{1b}\u{7f}אב\u{202e}\n".utf8))
            #expect(board.pasteboardItems?.first?.types == [.string])
            #expect(board.pasteboardItems?.count == 1)
            #expect(copyLabels(in: mounted) == ["Copied"])
            let copied = try feedbackPixels("Copied", in: mounted)
            #expect(copied.title.count > idle.title.count + 5)
            #expect(copied.icon.symmetricDifference(idle.icon).count > 15)
            try await waitForReset(clock, until: [1500], in: mounted)
            clock.advance(to: 1100)
            await mounted.settle()
            #expect(copyLabels(in: mounted) == ["Copied"])
            clock.advance(to: 1500)
            try await waitForIdleCopyLabel(in: mounted)
        }

        @Test func codeLongLinesStayUnwrappedInAHorizontalViewport() async throws {
            let short = try await CollectionWindow(#"root = Col([Code("let name = a\n  return name")],0)"#, width: 180, mode: .light)
            defer { short.window.closeForTest() }
            let longLine = "let name = " + String(repeating: "identifier + ", count: 20) + "end"
            let long = try await CollectionWindow(#"root = Col([Code("\#(longLine)\n  return name")],0)"#, width: 180, mode: .light)
            defer { long.window.closeForTest() }

            let shortScroll = try #require(short.scrollView)
            let longScroll = try #require(long.scrollView)
            let shortDocument = try #require(shortScroll.documentView)
            let longDocument = try #require(longScroll.documentView)
            let viewport = longScroll.contentView.bounds
            #expect(try long.rect("/children/0").width == 180)
            #expect(abs(viewport.width - 178) <= 1)
            #expect(longDocument.bounds.width > viewport.width * 4)
            #expect(abs(longDocument.bounds.height - shortDocument.bounds.height) <= 1)
            #expect(longDocument.bounds.height <= viewport.height + 1)

            longScroll.contentView.scroll(to: CGPoint(x: viewport.width, y: 0))
            #expect(longScroll.documentVisibleRect.minX > 0)
            #expect(longScroll.documentVisibleRect.minY == 0)
        }

        @Test func codeBodyUsesItsLineHeightAndEmptyCodeOnlyNeedsPadding() async throws {
            for (source, expectedHeight) in [("", 28.0), (#"one\ntwo"#, 69.6)] {
                let mounted = try await CollectionWindow(#"root = Col([Code("\#(source)")],0)"#, mode: .light)
                defer { mounted.window.closeForTest() }
                let scroll = try #require(mounted.scrollView)
                let document = try #require(scroll.documentView)
                #expect(abs(document.bounds.height - expectedHeight) <= 1)
            }
        }

        @Test func codeFitsAStartAlignedColumnUsingItsNaturalWidth() async throws {
            let longLine = String(repeating: "identifier + ", count: 40)
            let mounted = try await CollectionWindow(
                #"root = Col([Code("x"),Code("\#(longLine)")],0,"start")"#, width: 240, mode: .light)
            defer { mounted.window.closeForTest() }
            let short = try mounted.rect("/children/0")
            let long = try mounted.rect("/children/1")
            #expect(short.minX == 0)
            #expect(short.width < 240)
            #expect(long.minX == 0)
            #expect(long.width == 240)
        }

        @Test func codeUsesItsLanguageOrTheDefaultAndDrawsLiteralText() async throws {
            let snippet = "  **literal**  <tag>"
            for (argument, expected) in [("", "code"), (",null", "code"), (",\"\"", "code"), (",\"Swift 6\"", "Swift 6")] {
                let mounted = try await CollectionWindow(#"root = Col([Code("\#(snippet)"\#(argument))],0)"#, mode: .light)
                defer { mounted.window.closeForTest() }
                var views = [mounted.host as NSView]
                var labels: [String] = []
                var textView: NSView?
                while let view = views.popLast() {
                    if let label = view.accessibilityValue() as? String {
                        labels.append(label)
                        if label == snippet { textView = view }
                    }
                    views.append(contentsOf: view.subviews)
                }
                #expect(labels.contains(expected))
                let selectable = try #require(textView)
                let range = NSRange(location: 0, length: snippet.utf16.count)
                let text = try #require(selectable.accessibilityAttributedString(for: range))
                let font = try #require(text.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)
                #expect(text.string == snippet)
                #expect(font.pointSize == 13)
                #expect(try bodyPixels(in: mounted).count > 25)
            }
        }

        @Test func codeUsesTheSameAdvanceForWideAndNarrowCharacters() async throws {
            var widths: [CGFloat] = []
            for glyph: Character in ["W", "i"] {
                let line = String(repeating: glyph, count: 80)
                let mounted = try await CollectionWindow(#"root = Col([Code("\#(line)")],0)"#, width: 150, mode: .light)
                defer { mounted.window.closeForTest() }
                let scroll = try #require(mounted.scrollView)
                let document = try #require(scroll.documentView)
                #expect(document.bounds.width > scroll.documentVisibleRect.width)
                widths.append(document.bounds.width)
            }
            let wide = try #require(widths.first)
            let narrow = try #require(widths.last)
            #expect(abs(wide - narrow) <= 1)
        }

        @Test func codeCopyFailureGetsItsOwnResetInterval() async throws {
            let board = Board()
            let clock = ManualClock()
            let mounted = try await CollectionWindow(#"root = Col([Code("example")],0)"#, mode: .light)
            defer { mounted.window.closeForTest() }
            await install(board, clock: clock, in: mounted)
            let idle = try feedbackPixels("Copy code", in: mounted)

            try await clickCopy(in: mounted, labeled: "Copy code")
            #expect(copyLabels(in: mounted) == ["Copied"])
            try await waitForReset(clock, until: [1500], in: mounted)
            let previous = NSPasteboardItem()
            let other = NSPasteboardItem()
            try #require(previous.setString("previous", forType: .string))
            try #require(previous.setData(Data([0, 128, 255]), forType: .rtf))
            try #require(other.setString("another item", forType: .html))
            board.pasteboardItems = [previous, other]
            board.refuseData = true
            clock.advance(to: 700)
            try await clickCopy(in: mounted, labeled: "Copied")
            #expect(copyLabels(in: mounted) == ["Couldn't copy code"])
            // The second click cancels the first reset and starts its own.
            try await waitForReset(clock, until: [2200], in: mounted)
            let failed = try feedbackPixels("Couldn't copy code", in: mounted)
            #expect(failed.title.count > idle.title.count + 10)
            #expect(failed.icon.symmetricDifference(idle.icon).count > 15)
            #expect(board.pasteboardItems?.count == 2)
            #expect(board.pasteboardItems?.first?.string(forType: .string) == "previous")
            #expect(board.pasteboardItems?.first?.data(forType: .rtf) == Data([0, 128, 255]))
            #expect(board.pasteboardItems?.last?.string(forType: .html) == "another item")
            // After the first deadline but before the second.
            clock.advance(to: 1500)
            await mounted.settle()
            #expect(board.writes == 3)
            #expect(copyLabels(in: mounted) == ["Couldn't copy code"])
            clock.advance(to: 2200)
            try await waitForIdleCopyLabel(in: mounted)
        }

        @Test(.requiresDisplay)
        func codeCopyWritesToTheSystemPasteboard() async throws {
            NativeTestWatchdog.waitingFor("NSPasteboard.general")
            let board = NSPasteboard.general
            NativeTestWatchdog.waitingFor("the general pasteboard snapshot (items, types and promised data)")
            let saved = try #require(CodePasteboard.snapshot(board))
            // Only this defer restores the saved contents. If the watchdog
            // fires, its timer runs on a global queue while the main thread is
            // the one stuck, possibly inside an NSPasteboard call, and it ends
            // the process within 10 s. Restoring from there would call AppKit's
            // pasteboard off the main thread, concurrently with that call,
            // which AppKit doesn't document as safe. A developer's clipboard
            // can be lost only in that case.
            defer {
                NativeTestWatchdog.waitingFor("NSPasteboard.clearContents during restoration")
                board.clearContents()
                if !saved.isEmpty {
                    NativeTestWatchdog.waitingFor("NSPasteboard.writeObjects to restore the saved items")
                    #expect(board.writeObjects(saved))
                }
            }
            NativeTestWatchdog.waitingFor("the seed pasteboard write (change count, snapshot, clear, write or rollback)")
            try #require(CodePasteboard.write("previous", to: board))
            let mounted = try await CollectionWindow(#"root = Col([Code("  print(\"héllo 🐦\")\r\n\t\u0000","Swift")],0)"#, mode: .light)
            defer {
                NativeTestWatchdog.waitingFor("AppKit to close the Code copy window")
                mounted.window.closeForTest()
            }

            NativeTestWatchdog.waitingFor("AppKit to dispatch Copy and the system pasteboard write to return")
            try await clickCopy(in: mounted, labeled: "Copy code")
            NativeTestWatchdog.waitingFor("NSPasteboard.pasteboardItems and data(forType: .string) after Copy")
            #expect(board.pasteboardItems?.first?.data(forType: .string) == Data("  print(\"héllo 🐦\")\r\n\t\u{0}".utf8))
            NativeTestWatchdog.waitingFor("NSPasteboard.pasteboardItems and its item types after Copy")
            #expect(board.pasteboardItems?.first?.types == [.string])
            NativeTestWatchdog.waitingFor("NSPasteboard.pasteboardItems to count the items after Copy")
            #expect(board.pasteboardItems?.count == 1)
            NativeTestWatchdog.waitingFor("AppKit to expose the Copied accessibility label")
            #expect(copyLabels(in: mounted) == ["Copied"])
        }

        private func select(at point: CGPoint, in mounted: CollectionWindow, count: Int = 1) async throws {
            if !NSScreen.screens.isEmpty {
                try await mounted.click(point, count: count)
                return
            }
            // Headless NSWindow doesn't deliver synthetic mouse events to
            // embedded AppKit views. Resolve the same AppKit hit target first.
            let host = mounted.host
            let hit = try #require(host.hitTest(host.convert(point, to: host.superview)))
            let event = try #require(NSEvent.mouseEvent(with: .leftMouseDown, location: host.convert(point, to: nil),
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: mounted.window.windowNumber,
                context: nil, eventNumber: 0, clickCount: count, pressure: 1))
            hit.mouseDown(with: event)
        }

        // A user's double-click arrives as a single click and then a second
        // click. AppKit drops a lone second click in a window that isn't key
        // when it took the previous click to bring that window forward, as on
        // CI, where test windows never become key.
        private func doubleClick(at point: CGPoint, in mounted: CollectionWindow) async throws {
            try await select(at: point, in: mounted)
            try await select(at: point, in: mounted, count: 2)
        }

        @Test func codeOverflowCannotBeSelectedOutsideItsViewport() async throws {
            let glyphs = String(repeating: "W", count: 80)
            let mounted = try await CollectionWindow(
                #"root = Col([Grid([Code("\#(glyphs)"),Spacer()],2,null,0)],0)"#, width: 260, mode: .light)
            defer { mounted.window.closeForTest() }
            let scroll = try #require(mounted.scrollView)
            let document = try #require(scroll.documentView)
            let viewport = mounted.host.convert(scroll.contentView.bounds, from: scroll.contentView)
            let documentFrame = mounted.host.convert(document.bounds, from: document)
            let block = try mounted.rect("/children/0/children/0")
            let outside = CGPoint(x: viewport.maxX + 12, y: viewport.midY)
            let inside = CGPoint(x: viewport.midX, y: viewport.midY)
            try #require(documentFrame.contains(outside) && !block.contains(outside))

            // Whether or not AppKit delivers this click to a window that isn't
            // key, the Code body must not be the view under it.
            let host = mounted.host
            let outsideHit = try #require(host.hitTest(host.convert(outside, to: host.superview)))
            #expect(!outsideHit.isDescendant(of: scroll))
            try await select(at: outside, in: mounted, count: 2)
            #expect((mounted.window.firstResponder as? NSAccessibilityProtocol)?.accessibilitySelectedText()?.isEmpty ?? true)
            try await doubleClick(at: inside, in: mounted)
            #expect((mounted.window.firstResponder as? NSAccessibilityProtocol)?.accessibilitySelectedText() == glyphs)
        }

        @Test func codeDoubleClickPastARowEndSelectsThatRowsLastWord() async throws {
            let mounted = try await CollectionWindow(#"root = Col([Code("first\nsecond")],0)"#, mode: .light)
            defer { mounted.window.closeForTest() }
            let document = try #require(mounted.scrollView?.documentView)
            let pastFirstRow = CGPoint(x: document.bounds.maxX - 6, y: document.bounds.maxY - 14 - 20.8 / 2)

            try await select(at: mounted.host.convert(pastFirstRow, from: document), in: mounted, count: 2)
            let responder = try #require(mounted.window.firstResponder as? NSAccessibilityProtocol)
            #expect(responder.accessibilitySelectedText() == "first")
        }

        @Test func codeClipsCopyButtonWhereTheHeaderOverflows() async throws {
            let board = Board()
            let mounted = try await CollectionWindow(
                #"root = Col([Grid([Code("x"),Spacer(),Spacer(),Spacer()],4,null,0)],0)"#, width: 184, mode: .light)
            defer { mounted.window.closeForTest() }
            await install(board, in: mounted)

            let block = try mounted.rect("/children/0/children/0")
            let copy = try #require(mounted.accessibleButtons.first { $0.accessibilityLabel?() == "Copy code" })
            let screenFrame = try #require(copy.accessibilityFrame?())
            let windowFrame = mounted.window.convertFromScreen(screenFrame)
            let button = mounted.host.convert(windowFrame, from: nil)
            let visible = button.intersection(block.insetBy(dx: 1, dy: 1))
            let grid = try mounted.rect("/children/0")
            #expect(abs(block.width - grid.width / 4) <= 1)
            let inside = CGPoint(x: visible.midX, y: visible.midY)
            let outside = CGPoint(x: (block.maxX + button.maxX) / 2, y: button.midY)
            let scroll = try #require(mounted.scrollView)
            #expect(abs(scroll.contentView.bounds.width - (block.width - 2)) <= 1)
            try #require(!visible.isEmpty && button.contains(outside) && !block.contains(outside))
            try await mounted.click(outside)
            #expect(board.writes == 0)
            try await mounted.click(inside)
            #expect(board.writes == 1)
        }

        private func accessibilityValues(in mounted: CollectionWindow) -> [String] {
            var views = [mounted.host as NSView]
            var values: [String] = []
            while let view = views.popLast() {
                if let value = view.accessibilityValue() as? String { values.append(value) }
                views.append(contentsOf: view.subviews)
            }
            return values
        }

        @Test func codeCoercesDynamicPropsAndCopiesTheCoercedValue() async throws {
            let cases = [
                (#"root = Col([Code($port,$port)],0)"# + "\n$port = 8080", "8080", "8080"),
                (#"root = Col([{type:"element",typeName:"Code",props:{code:42,language:false},partial:false,hasDynamicProps:false}],0)"#,
                 "42", "false"),
            ]
            for (program, source, label) in cases {
                let mounted = try await CollectionWindow(program, mode: .light)
                defer { mounted.window.closeForTest() }
                let board = Board()
                await install(board, in: mounted)
                #expect(accessibilityValues(in: mounted).contains(label))
                #expect(accessibilityValues(in: mounted).contains(source))
                #expect(try bodyPixels(in: mounted).count > 15)
                try await clickCopy(in: mounted, labeled: "Copy code")
                #expect(board.pasteboardItems?.first?.data(forType: .string) == Data(source.utf8))
            }
        }

        @Test func codeKeepsCollapsedAndUnbrokenLanguageLabelsOnOneLine() async throws {
            let control = try await CollectionWindow(#"root = Col([Code("x","Swift 6")],0)"#, mode: .light)
            defer { control.window.closeForTest() }
            let controlBlock = try control.rect("/children/0")
            let controlButton = try buttonFrame(in: control, labeled: "Copy code")
            let collapsed = try await CollectionWindow(#"root = Col([Code("x"," \tSwift\r\n  6 ")],0)"#, mode: .light)
            defer { collapsed.window.closeForTest() }
            #expect(accessibilityValues(in: collapsed).contains("Swift 6"))
            #expect(try collapsed.rect("/children/0").height == controlBlock.height)
            #expect(try buttonFrame(in: collapsed, labeled: "Copy code") == controlButton)

            let label = String(repeating: "W", count: 65_536)
            let overflowing = try await CollectionWindow(#"root = Col([Code("x","\#(label)")],0)"#, mode: .light)
            defer { overflowing.window.closeForTest() }
            let overflowBlock = try overflowing.rect("/children/0")
            let overflowButton = try buttonFrame(in: overflowing, labeled: "Copy code")
            #expect(overflowBlock.height == controlBlock.height)
            #expect(overflowButton.height == controlButton.height)
            #expect(overflowButton.minX >= overflowBlock.maxX)
            #expect(try bodyPixels(in: overflowing) == bodyPixels(in: control))
        }

        @Test func codeCapsTheLaidOutLanguageLabelAtFourKiB() async throws {
            let prefix = String(repeating: "W", count: 4096)
            let label = prefix + "suffix"
            let mounted = try await CollectionWindow(#"root = Col([Code("x","\#(label)")],0)"#, mode: .light)
            defer { mounted.window.closeForTest() }
            #expect(accessibilityValues(in: mounted).contains(prefix))
        }

        @Test func codeDrawsBidirectionalLinesInUpstreamsLeftToRightOrder() async throws {
            // The pinned <pre><code> uses an LTR paragraph. The reference lines
            // isolate runs in their Chromium visual order, preserving RTL shaping.
            let cases = [
                ("שלום world", "\u{2067}שלום\u{2069} world"),
                ("مرحبا 123 code", "\u{2066}123\u{2069} \u{2067}مرحبا\u{2069} code"),
                ("שלום עולם", "\u{2067}שלום עולם\u{2069}"),
            ]
            for (source, visualOrder) in cases {
                let mounted = try await CollectionWindow(#"root = Col([Code("\#(source)")],0)"#, mode: .light)
                defer { mounted.window.closeForTest() }
                let reference = try await CollectionWindow(#"root = Col([Code("\#(visualOrder)")],0)"#, mode: .light)
                defer { reference.window.closeForTest() }
                let painted = try bodyPixels(in: mounted)
                #expect(painted.count > 25)
                #expect(try painted == bodyPixels(in: reference), "Visual order of \(source)")
            }
        }

        @Test func codeRepaintsItsInkWhenTheMountedThemeChanges() async throws {
            let mounted = try await CollectionWindow("""
                root = Box([Code("theme ink")],null,0,0,$background)
                $background = "#ffffff"
                """, mode: .light)
            defer { mounted.window.closeForTest() }
            let scroll = try #require(mounted.scrollView)
            let dark = try bodyPixels(in: mounted)
            #expect(dark.count > 25)

            try mounted.live.set("$background", to: .string("#000000"))
            await mounted.settle()

            #expect(mounted.scrollView === scroll)
            let light = try bodyPixels(in: mounted, darkInk: false)
            #expect(light.intersection(dark).count > 25)
            #expect(light.count < dark.count * 2)
        }

        @Test func codePaintsPreformattedWhitespaceAndCopiesOriginalBytes() async throws {
            let cases: [(String, String, String, CGFloat)] = [
                (#"x\n"#, "x", "x\n", 1),
                (#"x\n\r"#, "x", "x\n\r", 1),
                (#"\r"#, "", "\r", 0),
                (#"x\r\ny\rz\nw"#, #"x\nyz\nw"#, "x\r\ny\rz\nw", 3),
                (#"x\ty"#, "x       y", "x\ty", 1),
                (String(repeating: #"\t"#, count: 20) + "X", String(repeating: " ", count: 160) + "X",
                 String(repeating: "\t", count: 20) + "X", 1),
            ]
            for (input, expected, original, rows) in cases {
                let mounted = try await CollectionWindow(#"root = Col([Code("\#(input)")],0)"#, mode: .light)
                let control = try await CollectionWindow(#"root = Col([Code("\#(expected)")],0)"#, mode: .light)
                defer { mounted.window.closeForTest(); control.window.closeForTest() }
                let scroll = try #require(mounted.scrollView)
                let document = try #require(scroll.documentView)
                let expectedSize = try #require(control.scrollView?.documentView).bounds.size
                #expect(abs(document.bounds.height - (28 + 20.8 * rows)) <= 1)
                #expect(abs(document.bounds.width - expectedSize.width) <= 1)
                if document.bounds.width > scroll.contentView.bounds.width {
                    for view in [scroll, try #require(control.scrollView)] {
                        view.contentView.scroll(to: view.contentView.constrainBoundsRect(
                            view.contentView.bounds.offsetBy(dx: 10000, dy: 0)).origin)
                    }
                }
                let rendered = try bodyPixels(in: mounted, rows: rows)
                if rows > 0 { #expect(!rendered.isEmpty) }
                #expect(try rendered == bodyPixels(in: control, rows: rows))
                let board = Board()
                await install(board, in: mounted)
                try await clickCopy(in: mounted, labeled: "Copy code")
                #expect(board.pasteboardItems?.first?.data(forType: .string) == Data(original.utf8))
            }
        }

        @Test func codePaintsBothEndsOfALongLineAndSelectsTheWholeSource() async throws {
            let control = try await CollectionWindow(
                #"root = Col([Code("\#(String(repeating: "x", count: 48))","Swift")],0)"#, mode: .light)
            defer { control.window.closeForTest() }
            let mounted = try await CollectionWindow(#"root = Col([Code($text,"Swift")],0)"# + "\n$text = \"x\"", mode: .light)
            defer { mounted.window.closeForTest() }
            let source = String(repeating: "x", count: 1_048_576)
            try mounted.live.set("$text", to: .string(source))
            let board = Board()
            await install(board, in: mounted)

            let controlPixels = try bodyPixels(in: control)
            #expect(controlPixels.count > 100)
            #expect(try bodyPixels(in: mounted) == controlPixels)
            let scroll = try #require(mounted.scrollView)
            let clip = scroll.contentView
            let origin = clip.bounds
            let maximum = clip.constrainBoundsRect(origin.offsetBy(dx: 1e10, dy: 0)).origin
            #expect(maximum.x > 1_000_000)
            clip.scroll(to: maximum)
            #expect(abs(scroll.documentVisibleRect.maxX - (scroll.documentView?.bounds.maxX ?? 0)) <= 1)
            #expect(try bodyPixels(in: mounted).count > 100)
            let minimum = clip.constrainBoundsRect(origin.offsetBy(dx: -1e10, dy: 0)).origin
            #expect(minimum.x == 0)
            clip.scroll(to: minimum)
            let viewport = mounted.host.convert(clip.bounds, from: clip)
            try await select(at: CGPoint(x: viewport.midX, y: viewport.midY), in: mounted)
            let responder = try #require(mounted.window.firstResponder)
            let selectAll = #selector(NSResponder.selectAll(_:))
            try #require(responder.responds(to: selectAll))
            try #require(NSApp.sendAction(selectAll, to: responder, from: nil))
            let selected = (mounted.window.firstResponder as? NSAccessibilityProtocol)?.accessibilitySelectedText()
            #expect(selected?.utf8.count == source.utf8.count)
            #expect(selected == source)
            let copy = #selector(NSText.copy(_:))
            try #require(responder.responds(to: copy))
            #expect(NSApp.sendAction(copy, to: responder, from: nil))
            #expect(board.pasteboardItems?.first?.data(forType: .string) == Data(source.utf8))
        }

        @Test func codePaintsRowsInsideAnOuterVerticalScrollViewport() async throws {
            let source = String(repeating: #"x\n"#, count: 262_144) + "ZZZZZZZZZZ"
            let mounted = try await CollectionWindow(#"root = Col([Code("\#(source)")],0)"#, scrollable: true, mode: .light)
            defer { mounted.window.closeForTest() }
            let outer = try #require(mounted.scrollView)
            let first = try pixels(in: mounted, rect: CGRect(x: 18, y: 51, width: 90, height: 22))
            #expect(first.count > 5)
            let viewport = outer.contentView
            let maximum = viewport.constrainBoundsRect(viewport.bounds.offsetBy(dx: 0, dy: 1e9)).origin
            #expect(maximum.y > 5_000_000)
            viewport.scroll(to: maximum)
            let last = try pixels(in: mounted, rect: CGRect(x: 18, y: 165, width: 90, height: 21))
            #expect(last.count > 25)
        }

        @Test func codeDrawingSurfaceStaysWithinTheViewportWhileScrolling() async throws {
            let row = String(repeating: "W", count: 160)
            let source = String(repeating: row + #"\n"#, count: 80)
            let mounted = try await CollectionWindow(#"root = Col([Code("\#(source)")],0)"#, scrollable: true, mode: .light)
            defer { mounted.window.closeForTest() }
            let outer = try #require(mounted.scrollView)
            var views = outer.subviews
            var codeScroll: CodeScrollView?
            while let view = views.popLast() {
                if let scroll = view as? CodeScrollView {
                    codeScroll = scroll
                    break
                }
                views.append(contentsOf: view.subviews)
            }
            let scroll = try #require(codeScroll)
            let document = try #require(scroll.documentView)
            let surface = try #require(document.subviews.first)
            let width = scroll.contentView.bounds.width
            let height = outer.contentView.bounds.height
            try #require(document.bounds.width > width * 3 && document.bounds.height > height * 3)

            let initial = surface.frame
            #expect(initial.width > 0 && initial.width <= width + 1)
            #expect(initial.height > 0 && initial.height <= height + 1)

            scroll.contentView.scroll(to: CGPoint(x: width, y: 0))
            outer.contentView.scroll(to: outer.contentView.bounds.offsetBy(dx: 0, dy: height).origin)
            await mounted.settle()

            let moved = surface.frame
            #expect(moved.width > 0 && moved.width <= width + 1)
            #expect(moved.height > 0 && moved.height <= height + 1)
            #expect(moved.minX > initial.minX)
            #expect(moved.minY < initial.minY)
        }

        @Test func codeDrawsStreamedPrefixesAndMatchesTheFinishedSource() async throws {
            let program = #"root = Col([Code("héllo 🐦\t\r\nprint(\"x\")","Swift\n6")],0)"#
            let streamed = try await CollectionWindow("", streaming: true, mode: .light)
            defer { streamed.window.closeForTest() }
            let bounds = streamed.host.bounds
            let bitmap = try #require(streamed.host.bitmapImageRepForCachingDisplay(in: bounds))
            for cut in 1...program.utf8.count {
                let prefix = String(decoding: program.utf8.prefix(cut), as: UTF8.self)
                try await streamed.live.show(response: prefix, streaming: true)
                streamed.host.layoutSubtreeIfNeeded()
                streamed.host.cacheDisplay(in: bounds, to: bitmap)
            }
            try await streamed.live.show(response: program, streaming: false)
            await streamed.settle()
            let control = try await CollectionWindow(program, mode: .light)
            defer { control.window.closeForTest() }
            let finished = try bodyPixels(in: streamed, rows: 2)
            #expect(finished.count > 25)
            #expect(try finished == bodyPixels(in: control, rows: 2))
        }

        @Test(.requiresDisplay)
        func codeHorizontalSwipeScrollsOnlyTheBody() async throws {
            let longLine = String(repeating: "identifier + ", count: 40)
            let mounted = try await CollectionWindow(#"root = Col([Code("\#(longLine)","Swift")],0)"#, width: 200, mode: .light)
            defer {
                NativeTestWatchdog.waitingFor("AppKit to close the Code scroll window")
                mounted.window.closeForTest()
            }
            NativeTestWatchdog.waitingFor("AppKit to expose the Code scroll view and Copy button frame")
            let scroll = try #require(mounted.scrollView as? CodeScrollView)
            let copy = try #require(mounted.accessibleButtons.first { $0.accessibilityLabel?() == "Copy code" })
            let headerBefore = try #require(copy.accessibilityFrame?())
            let clip = scroll.contentView
            #expect(scroll.documentVisibleRect.origin == .zero)

            // Apply the same four 36-point horizontal deltas to the clip view
            // AppKit scrolls during a swipe. A synthetic began event can enter
            // AppKit's tracking loop before the test can send the ended event.
            NativeTestWatchdog.waitingFor("AppKit to scroll and reflect the Code body clip view")
            for _ in 0..<4 {
                let proposed = clip.bounds.offsetBy(dx: 36, dy: 0)
                clip.scroll(to: clip.constrainBoundsRect(proposed).origin)
                scroll.reflectScrolledClipView(clip)
            }
            await mounted.settle()
            NativeTestWatchdog.waitingFor("AppKit to expose the scrolled body and Copy button frame")
            #expect(scroll.documentVisibleRect.minX > 0)
            #expect(scroll.documentVisibleRect.minY == 0)
            #expect(copy.accessibilityFrame?() == headerBefore)
        }

        @Test(.requiresDisplay)
        func codeWheelScrollThroughTheWindowScrollsOnlyTheBody() async throws {
            let longLine = String(repeating: "identifier + ", count: 40)
            let mounted = try await CollectionWindow(#"root = Col([Code("\#(longLine)","Swift")],0)"#, width: 200, mode: .light)
            defer {
                NativeTestWatchdog.waitingFor("AppKit to close the Code wheel window")
                mounted.window.closeForTest()
            }
            NativeTestWatchdog.waitingFor("AppKit to expose the Code scroll view and Copy button frame")
            let scroll = try #require(mounted.scrollView as? CodeScrollView)
            let copy = try #require(mounted.accessibleButtons.first { $0.accessibilityLabel?() == "Copy code" })
            let headerBefore = try #require(copy.accessibilityFrame?())
            let clip = scroll.contentView
            #expect(scroll.documentVisibleRect.origin == .zero)

            // A mouse wheel's horizontal line scroll: no precise deltas, no
            // gesture phase and no momentum. AppKit scrolls such events
            // discretely, without the gesture tracking loop that pulls the
            // rest of a trackpad swipe from the real event stream.
            NativeTestWatchdog.waitingFor("a horizontal scroll-wheel event for the Code body")
            let point = clip.convert(CGPoint(x: clip.bounds.midX, y: clip.bounds.midY), to: nil)
            let located = try #require(NSEvent.mouseEvent(with: .mouseMoved, location: point, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: mounted.window.windowNumber,
                context: nil, eventNumber: 0, clickCount: 0, pressure: 0))
            let wheel = try #require(located.cgEvent?.copy())
            wheel.type = .scrollWheel
            wheel.setIntegerValueField(.scrollWheelEventIsContinuous, value: 0)
            wheel.setIntegerValueField(.scrollWheelEventScrollPhase, value: 0)
            wheel.setIntegerValueField(.scrollWheelEventMomentumPhase, value: 0)
            wheel.setIntegerValueField(.scrollWheelEventDeltaAxis1, value: 0)
            wheel.setIntegerValueField(.scrollWheelEventDeltaAxis2, value: -3)
            let event = try #require(NSEvent(cgEvent: wheel))
            try #require(event.type == .scrollWheel && !event.hasPreciseScrollingDeltas)
            try #require(event.phase == [] && event.momentumPhase == [])
            try #require(event.scrollingDeltaX < 0 && event.scrollingDeltaY == 0)

            NativeTestWatchdog.waitingFor("NSWindow.sendEvent to deliver the scroll-wheel event to the Code body")
            mounted.window.sendEvent(event)
            await mounted.settle(waitingFor: "AppKit to lay out the scrolled Code window")
            NativeTestWatchdog.waitingFor("AppKit to expose the scrolled body and Copy button frame")
            #expect(scroll.documentVisibleRect.minX > 0)
            #expect(scroll.documentVisibleRect.minY == 0)
            #expect(copy.accessibilityFrame?() == headerBefore)
        }
    }
}
