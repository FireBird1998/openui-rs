import OpenUIFFI
@testable import OpenUINative
import SwiftTestSupport
import Synchronization
import Testing

@Suite(.timeLimit(.minutes(1))) @MainActor struct RawResponses {
    private let schema = #"{"$defs":{"Text":{"properties":{"text":{}}},"Button":{"properties":{"label":{},"action":{}}}}}"#
    private let header = "]]>openui:content?library=bench&name=City+desk\n"
    private let section = "\n]]>openui:scripts {\"count\":1}\n"
    private let script = #"[{"name":"edition","code":"v1"}]"#
    private let query = #"q=Query("edition",{},"default")"#

    // Dispatch creates the tasks synchronously. These providers return immediately.
    private func delivered(_ live: LiveProgram) async {
        for task in Array(live.queryDriver?.calls.values ?? [:].values) { await task.value }
    }

    @Test func aSettledRawResponseReportsTheReaderRoundLimitInQueryStatus() async throws {
        let library = try await Core().library(schema: schema)
        let live = try LiveProgram(library: library)
        let calls = Mutex(0)
        try live.attachToolProvider(clock: ManualClock()) { name, args in
            calls.withLock { $0 += 1 }
            let one = args?["v"] == .number(1)
            return .number((name == "A" ? !one : one) ? 1 : 0)
        }
        let anchors = "\na1=Query(\"A\",{v:1})\nb0=Query(\"B\",{v:0})"
        let seed = "a=Query(\"A\",{v:0})\nb=Query(\"B\",{v:1})" + anchors
        try await live.show(response: seed + "\nroot=Text(a)", streaming: false)
        await delivered(live)
        #expect(calls.withLock { $0 } == 4)
        let cycle = "a=Query(\"A\",{v:b})\nb=Query(\"B\",{v:a})" + anchors
            + "\nholdA=Query(\"A\",{v:0})\nholdB=Query(\"B\",{v:1})\nroot=Text(a)"
        try await live.show(response: cycle, streaming: false)
        #expect(live.queries.evaluationError == "Query readers were still changing at the reader round limit.")
        #expect(live.queries.loading.isEmpty)
        #expect(calls.withLock { $0 } == 4)
        try await live.show(response: "root=Text(\"done\")", streaming: false)
        #expect(live.queries.evaluationError == nil)
    }

    @Test func bareResponseMatchesStreamSetAndFinishAndDrawsTheToolOutcome() async throws {
        let core = Core()
        let library = try await core.library(schema: schema)
        let live = try LiveProgram(library: library, root: "Text")
        let parser = try await core.session(library: library, root: "Text")
        let text = query + "\nview=Text(q)\nother=Text($name"
        let calls = Mutex<[String]>([])
        try live.attachToolProvider(clock: ManualClock()) { name, _ in
            calls.withLock { $0.append(name) }
            return .string("edition one")
        }
        let preview = try await live.show(response: text, streaming: true)
        #expect(preview.json == (try await parser.set(text)).json)
        #expect(!live.acceptsInteractions)
        await delivered(live)
        #expect(calls.withLock { $0.isEmpty })
        let end = try await live.show(response: text, streaming: false)
        #expect(end.json == (try await parser.finish()).json)
        #expect(end.json != preview.json)
        await delivered(live)
        #expect(calls.withLock { $0 } == ["edition"])
        #expect(drawnText(live) == "edition one")
        #expect(live.acceptsInteractions)
    }

    @Test func winningMessageHidesPartialMarkersAndPublishesItsOwnMetadataAndQuery() async throws {
        let core = Core()
        let library = try await core.library(schema: schema)
        let live = try LiveProgram(library: library)
        try live.attachToolProvider(clock: ManualClock()) { name, _ in .string(name) }
        let parser = try await core.session(library: library)
        try await live.show(response: String(header.dropLast()), streaming: true)
        #expect(live.root == nil && live.response?.name == nil)
        let first = header + #"root=Text("before")"#
        let held = try await live.show(response: first + "]]>openui:con", streaming: true)
        #expect(held.json == (try await parser.set(#"root=Text("before")"#)).json)
        #expect(held.program == #"root=Text("before")"#)
        #expect(drawnText(live) == "before")
        let newHeader = "]]>openui:content?name=Night+desk&library=local%2F1\n"
        let program = query + "\nroot=Text(q)"
        let end = try await live.show(response: first + newHeader + program +
            "\n]]>openui:context\n{\"region\":7}\n]]>openui:end", streaming: false)
        _ = try await parser.set(program)
        #expect(end.json == (try await parser.finish()).json)
        #expect(end.program == program)
        #expect(live.response?.name == "Night desk")
        #expect(live.response?.attributes == ["name":"Night desk", "library":"local/1"])
        #expect(live.response?.context["region"] == .number(7))
        await delivered(live)
        #expect(drawnText(live) == "edition")
        try live.show(try await core.parseOneShot(#"root=Text("plain")"#, library: library))
        #expect(live.response == nil)
    }

    @Test func scriptsStayOutsideTheParserAndOnlyAChangedSettledScriptRefetches() async throws {
        let core = Core()
        let library = try await core.library(schema: schema)
        let live = try LiveProgram(library: library)
        let parser = try await core.session(library: library)
        let calls = Mutex<[String]>([])
        try live.attachToolProvider(clock: ManualClock()) { name, _ in
            let count = calls.withLock { $0.append(name); return $0.filter { $0 == name }.count }
            return .string("\(name)\(count)")
        }
        let program = query + "\nr=Query(\"desk\",{})\nroot=Text(q + \"/\" + r)"
        let raw = header + program
        for suffix in ["\n]]>openui:scr", section, section + "[{", section + script + "\n]]>openui:end\n"] {
            let step = try await live.show(response: raw + suffix, streaming: true)
            #expect(step.program == program)
            #expect(step.json == (try await parser.set(program)).json)
            #expect(live.isQueryLoading && live.parserError == nil && live.response?.error == nil)
            #expect(!live.acceptsInteractions)
            await delivered(live)
            #expect(calls.withLock { $0.isEmpty })
        }
        try await live.show(response: raw + section + script, streaming: false)
        await delivered(live)
        #expect(drawnText(live) == "edition1/desk1")
        let edit = raw + section + script.replacingOccurrences(of: "v1", with: "v2")
        try await live.show(response: edit, streaming: true)
        try live.retryFailedQueries()
        await delivered(live)
        #expect(drawnText(live) == "edition1/desk1")
        #expect(calls.withLock { $0.count } == 2)
        try await live.show(response: edit, streaming: false)
        await delivered(live)
        #expect(drawnText(live) == "edition2/desk1")
        #expect(!live.isQueryLoading && calls.withLock { $0.count } == 3)
    }

    @Test func invalidBundleKeepsDrawingAndBlocksActionsAndRetryAcrossRedraws() async throws {
        let core = Core()
        var events: [String] = []
        let live = try LiveProgram(library: await core.library(schema: schema)) { events.append($0.type) }
        let calls = Mutex(0)
        try live.attachToolProvider(clock: ManualClock()) { _, _ in calls.withLock { $0 += 1 }; return .null }
        let raw = header + query + "\nroot=Button(\"Send\", @ToAssistant(\"sent\"))" + section
        try await live.show(response: raw + script, streaming: false)
        await delivered(live)
        try await live.show(response: raw + "[", streaming: true)
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events.isEmpty)
        try await live.show(response: raw + "[]", streaming: false)
        try live.set("$edit", to: .number(1))
        #expect(drawnText(live) == "Send")
        #expect(live.response?.error == "Invalid scripts bundle")
        #expect(live.parserError?["message"] == .string("Invalid scripts bundle"))
        #expect(live.parserError?["code"] == .string("parse-failed"))
        #expect(live.parserError?["source"] == .string("parser"))
        #expect(!live.isStreaming && !live.acceptsInteractions)
        let button = try #require(live.root)
        try live.run("action", of: button, message: "Send")
        try live.retryFailedQueries()
        await delivered(live)
        #expect(events.isEmpty && calls.withLock { $0 } == 1)
        try await live.show(response: raw + script, streaming: false)
        #expect(live.parserError == nil && live.acceptsInteractions)
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events == ["continue_conversation"])
    }

    @Test func theSessionsBlockedFlagDisablesControlsUntilTheResponseSettles() async throws {
        let core = Core()
        var events: [String] = []
        let live = try LiveProgram(library: await core.library(schema: schema)) { events.append($0.type) }
        let program = #"root=Button("Send", @ToAssistant("sent"))"#
        let bundle = header + program + section
        try await live.show(response: bundle + script, streaming: true)
        #expect(live.isBlocked && !live.acceptsInteractions)
        try await live.show(response: bundle + script, streaming: false)
        #expect(!live.isBlocked && live.acceptsInteractions)
        // An invalid settled bundle stays blocked, and its click is ignored.
        try await live.show(response: bundle + "[", streaming: false)
        #expect(live.isBlocked && !live.isStreaming && !live.acceptsInteractions)
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events.isEmpty)
        // A refused show leaves the flag as it was.
        do {
            try await live.show(response: bundle + script + String(repeating: " ", count: 1 << 20), streaming: false)
            Issue.record("The oversized response was accepted")
        } catch let error where error.kind == .limitExceeded {}
        #expect(live.isBlocked && !live.acceptsInteractions)
        // A bare program is blocked only while the app says it streams.
        try await live.show(response: program, streaming: false)
        #expect(!live.isBlocked && live.acceptsInteractions)
        live.isStreaming = true
        #expect(live.isBlocked && !live.acceptsInteractions)
        live.isStreaming = false
        #expect(!live.isBlocked && live.acceptsInteractions)
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events == ["continue_conversation"])
        try await live.show(response: bundle + script, streaming: false)
        #expect(!live.isBlocked && live.acceptsInteractions)
    }

    @Test func eachDrawCarriesTheRenderersBlockedFlag() throws {
        let session = try RuntimeSession(library: ComponentLibrary(schema: schema), promptSpec: nil)
        defer { session.dispose() }
        for (state, blocked) in [
            (ResponseState(), false),
            (ResponseState(streaming: true), true),
            (ResponseState(isBundle: true, complete: false), true),
            (ResponseState(isBundle: true, error: "Invalid scripts bundle"), true),
            (ResponseState(isBundle: true), false),
        ] {
            #expect(try session.evaluateWithResponse(responseState: state).blocked == blocked)
        }
        #expect(try session.evaluate().blocked == false)
    }

    @Test func clearingAppStreamingAndDiscardingABundlePreviewRestoreProviderFreeActions() async throws {
        let core = Core()
        var events: [String] = []
        let live = try LiveProgram(library: await core.library(schema: schema)) { events.append($0.type) }
        let program = query + "\nroot=Button(\"Send\", @ToAssistant(\"sent\"))"
        let raw = header + program + section + script
        live.isStreaming = true
        try await live.show(response: raw, streaming: false)
        live.isStreaming = false
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events.count == 1 && !live.isQueryLoading)

        try await live.show(response: program, streaming: false)
        try await live.show(response: raw, streaming: true)
        try live.set("$edit", to: .number(1))
        live.discardPreview()
        try live.run("action", of: #require(live.root), message: "Send")
        #expect(events.count == 2 && !live.isQueryLoading)
    }

    @Test func discardingAnInFlightReadPreventsItFromReplacingTheDrawing() async throws {
        let core = Core()
        let live = try LiveProgram(library: await core.library(schema: schema))
        try await live.show(response: header + #"root=Text("City")"#, streaming: false)
        let edit = Task.immediate { try await live.show(response: #"root=Text("Late")"#, streaming: true) }
        live.discardPreview()
        let superseded = await #expect(throws: OpenUIError.self) { try await edit.value }
        #expect(superseded?.kind == .cancelled)
        #expect(drawnText(live) == "City" && live.response?.name == "City desk")
    }

    @Test func aRefusedShowStillSupersedesTheReadInFlight() async throws {
        let core = Core()
        let live = try LiveProgram(library: await core.library(schema: schema))
        let raw = header + #"root=Text("City")"#
        try await live.show(response: raw, streaming: false)
        let oversized = raw + String(repeating: " ", count: 1 << 20)
        let edit = Task.immediate { try await live.show(response: #"root=Text("Late")"#, streaming: true) }
        let refused = await #expect(throws: OpenUIError.self) {
            try await live.show(response: oversized, streaming: false)
        }
        #expect(refused?.kind == .limitExceeded)
        let superseded = await #expect(throws: OpenUIError.self) { try await edit.value }
        #expect(superseded?.kind == .cancelled)
        #expect(drawnText(live) == "City" && live.response?.name == "City desk")
    }

    @Test func oversizedRawResponsePreservesThePreviousDrawingAndMetadata() async throws {
        let core = Core()
        let live = try LiveProgram(library: await core.library(schema: schema))
        let raw = header + #"root=Text("City")"#
        try await live.show(response: raw, streaming: false)
        let oversized = raw + section + String(repeating: " ", count: 1 << 20)
        do {
            try await live.show(response: oversized, streaming: false)
            Issue.record("The oversized response was accepted")
        } catch let error where error.kind == .limitExceeded {
            #expect(error.message == "response bundle exceeds the program-size limit")
        }
        #expect(drawnText(live) == "City" && live.response?.name == "City desk")
    }

    /// A piece that takes the response past the program-size limit is
    /// refused as `show` refuses the whole text, and the session stays at
    /// what it accepted: the next piece goes on from there.
    @Test func aPiecePastTheLimitIsRefusedAndTheSessionKeepsWhatItAccepted() async throws {
        let core = Core()
        let live = try LiveProgram(library: await core.library(schema: schema))
        let raw = header + #"root=Text("City")"#
        try await live.append(response: raw, streaming: true)
        do {
            try await live.append(response: section + String(repeating: " ", count: 1 << 20), streaming: true)
            Issue.record("The oversized piece was accepted")
        } catch let error where error.kind == .limitExceeded {
            #expect(error.message == "response bundle exceeds the program-size limit")
        }
        try await live.append(response: "\nx=1\n", streaming: true)
        let drawn = try await live.showResponse(streaming: false)
        let control = try LiveProgram(library: await core.library(schema: schema))
        let expected = try await control.show(response: raw + "\nx=1\n", streaming: false)
        #expect(drawn.json == expected.json && drawn.program == expected.program)
        #expect(drawn.response?.sourceByteCount == (raw + "\nx=1\n").utf8.count)
        #expect(drawnText(live) == "City" && live.response?.name == "City desk")
    }
}

@Suite(.timeLimit(.minutes(3))) @MainActor struct AppendedRawResponses {
    /// A driver that hands the session each piece of a stream and draws now
    /// and then (the chat app, #206) draws, every time, what a driver that
    /// showed every whole prefix draws for the same text: the parse, the
    /// response's metadata and the evaluated tree, then the settled draw.
    @Test func appendedPiecesDrawWhatWholeReadsDraw() async throws {
        let core = Core()
        let library = try await core.library(schema: #"{"$defs":{"Text":{"properties":{"text":{}}},"Stack":{"properties":{"children":{}}}}}"#)
        let appending = try LiveProgram(library: library)
        let whole = try LiveProgram(library: library)
        let header = "]]>openui:content?library=bench&name=City+desk\n"
        let lines = 1...18
        var statements = lines.map { #"t\#($0)=Text("Line \#($0): é😀 \\"quoted\\" ]] text")"# }
        statements.insert("root=Stack([" + lines.map { "t\($0)" }.joined(separator: ",") + "])", at: 0)
        let text = header + statements.joined(separator: "\r\n") + "\n]]>openui:context\n{\"edition\":2}\n"
        var pieces: [String] = []
        var rest = Substring(text)
        var size = 1
        while !rest.isEmpty {
            pieces.append(String(rest.prefix(size)))
            rest = rest.dropFirst(size)
            size = size % 13 + 1
        }
        var accepted = ""
        var draws = 0
        for (index, piece) in pieces.enumerated() {
            try await appending.append(response: piece, streaming: true)
            accepted += piece
            guard index % (index % 5 + 1) == 0 else { continue }
            let expected = try await whole.show(response: accepted, streaming: true)
            let drawn = try await appending.showResponse(streaming: true)
            #expect(drawn.json == expected.json, "\(accepted.debugDescription)")
            #expect(drawn.program == expected.program)
            #expect(drawn.response?.name == expected.response?.name)
            #expect(drawn.response?.context == expected.response?.context)
            #expect(drawn.response?.attributes == expected.response?.attributes)
            #expect(drawn.response?.sourceByteCount == expected.response?.sourceByteCount)
            #expect(drawn.response?.state == expected.response?.state)
            #expect(appending.rootJSON == whole.rootJSON, "\(accepted.debugDescription)")
            #expect(appending.errors == whole.errors)
            draws += 1
        }
        #expect(draws > 50 && draws < pieces.count)
        let settled = try await appending.showResponse(streaming: false)
        let expected = try await whole.show(response: text, streaming: false)
        #expect(settled.json == expected.json)
        #expect(settled.response?.context == .object([("edition", .number(2))]))
        #expect(appending.rootJSON == whole.rootJSON)
        #expect(appending.acceptsInteractions && whole.acceptsInteractions)
    }
}
