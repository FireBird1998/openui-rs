import Foundation
import Testing
@testable import ChatKit

private let request = ChatRequest(model: "anthropic/claude-sonnet-5.5", messages: [
    ChatMessage(.system, "the config block"),
    ChatMessage(.user, "What is the capital of France?"),
])

private func collect(_ stream: any AsyncSequence<String, any Error>) async -> (text: String, error: (any Error)?) {
    var text = ""
    do {
        for try await delta in stream { text += delta }
        return (text, nil)
    } catch {
        return (text, error)
    }
}

@Suite struct TheGatewayClient {
    @Test func sendsTheWholeThreadToChatCompletionsWithTheKeyAsABearer() async throws {
        let transport = RecordedTransport(try .named("streamed-answer"))
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport)
        _ = await collect(gateway.answer(request))
        let sent = try #require(transport.sent.first)
        #expect(sent.url?.absoluteString == "https://api.thesys.dev/v1/embed/chat/completions")
        #expect(sent.httpMethod == "POST")
        #expect(sent.value(forHTTPHeaderField: "Authorization") == "Bearer \(testKey)")
        #expect(sent.value(forHTTPHeaderField: "Content-Type") == "application/json")
        #expect(sent.bodyText == #"{"model":"anthropic/claude-sonnet-5.5","messages":["#
            + #"{"role":"system","content":"the config block"},"#
            + #"{"role":"user","content":"What is the capital of France?"}],"stream":true}"#)
    }

    @Test func aStreamedAnswerArrivesAsItsTextDeltas() async throws {
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: RecordedTransport(try .named("streamed-answer")))
        let (text, error) = await collect(gateway.answer(request))
        #expect(error == nil)
        #expect(text == "root = Stack([title, card])\n"
            + "title = TextContent(\"Paris\", \"large-heavy\")\n"
            + "card = Card([TextContent(\"Paris is the capital of France.\")])")
    }

    @Test func anHTTPErrorCarriesTheGatewaysMessageAndStatus() async throws {
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: RecordedTransport(try .named("http-error")))
        let (text, error) = await collect(gateway.answer(request))
        #expect(text.isEmpty)
        #expect(error as? GatewayError == GatewayError("Invalid API key provided.", status: 401))
    }

    @Test func anErrorMidStreamEndsTheAnswerAfterItsText() async throws {
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: RecordedTransport(try .named("error-mid-stream")))
        let (text, error) = await collect(gateway.answer(request))
        #expect(text.hasPrefix("root = Stack([title, card])\n"))
        #expect(error as? GatewayError == GatewayError("The model is overloaded. Try again later."))
    }

    @Test func withNoKeyNothingIsSent() async throws {
        let transport = RecordedTransport(try .named("streamed-answer"))
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(), transport: transport)
        let (_, error) = await collect(gateway.answer(request))
        #expect(error as? GatewayError == GatewayError("Add your Thesys API key in Settings."))
        #expect(transport.sent.isEmpty)
    }

    @Test func anErrorBodyThatIsNotJSONFallsBackToTheStatus() async throws {
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: RecordedTransport(
            Recording(status: 502, chunks: ["<html>Bad gateway</html>"])))
        let (_, error) = await collect(gateway.answer(request))
        #expect(error as? GatewayError == GatewayError("Request failed: 502", status: 502))
    }

    @Test func upstreamsOtherErrorShapesAreRead() async throws {
        for (body, message) in [(#"{"error":"plain words"}"#, "plain words"),
                                (#"{"message":"top level"}"#, "top level"),
                                (#"{"error":{"message":"  "}}"#, "Request failed: 400")] {
            let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                        transport: RecordedTransport(Recording(status: 400, chunks: [body])))
            let (_, error) = await collect(gateway.answer(request))
            #expect(error as? GatewayError == GatewayError(message, status: 400))
        }
    }

    @Test func theKeyNeverReachesAShownMessage() async throws {
        let echoed = #"{"error":{"message":"Key \#(testKey) is not valid"}}"#
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: RecordedTransport(Recording(status: 401, chunks: [echoed])))
        let (_, error) = await collect(gateway.answer(request))
        let message = try #require(error as? GatewayError).message
        #expect(!message.contains(testKey))
        #expect(message == "Key [your API key] is not valid")
    }

    @Test func aLostConnectionIsReported() async throws {
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: RecordedTransport(
            Recording(status: 200, chunks: ["data: {\"choices\":[{\"delta\":{\"content\":\"root\"}}]}\n"],
                      failAfter: "The network connection was lost.")))
        let (text, error) = await collect(gateway.answer(request))
        #expect(text == "root")
        #expect(error as? GatewayError == GatewayError("The network connection was lost."))
    }

    @Test func linesThatAreNotJSONOrNotDataAreSkippedAsUpstreamSkipsThem() async throws {
        let body = ": comment\nevent: message\ndata: not json\ndata:{\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n"
            + "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\r\n"
            + "data: {\"choices\":[]}\ndata: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}"
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: RecordedTransport(Recording(status: 200, chunks: [body])))
        let (text, error) = await collect(gateway.answer(request))
        #expect(error == nil)
        #expect(text == "ab")
    }

    @Test func anOverlongLineIsRefusedNotBuffered() async throws {
        let long = "data: " + String(repeating: "x", count: ThesysGateway.maxLineBytes + 1)
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: RecordedTransport(Recording(status: 200, chunks: [long])))
        let (_, error) = await collect(gateway.answer(request))
        #expect(error as? GatewayError == GatewayError("The Gateway sent a line longer than 1 MiB."))
    }

    @Test func anErrorBodyIsReadOnlyUpToItsLimit() async throws {
        let huge = String(repeating: "y", count: ThesysGateway.maxErrorBodyBytes * 4)
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey),
                                    transport: RecordedTransport(Recording(status: 500, chunks: [huge])))
        let (_, error) = await collect(gateway.answer(request))
        #expect(error as? GatewayError == GatewayError("Request failed: 500", status: 500))
    }

    @Test func anAnswerIsReadOnlyAsFastAsItIsTakenAndLettingItGoCancelsTheRequest() async throws {
        let transport = EndlessTransport()
        let gateway = ThesysGateway(keyStore: InMemoryKeyStore(testKey), transport: transport)
        var taken: [String] = []
        for try await delta in gateway.answer(request) {
            taken.append(delta)
            if taken.count == 3 { break }
        }
        #expect(taken.first == EndlessTransport.head)
        // Three events and nothing read ahead of them.
        #expect(transport.bytesRead.count < 1024)
        #expect(await eventually { transport.released.isSet })
    }
}
