@testable import OpenUIIntelligentUI
import Foundation
import SwiftTestSupport
import Testing

@Suite(NativeTestWatchdog()) @MainActor struct PhotoLoaderTests {
    private func load(_ source: PhotoSource) async -> PhotoRaster? {
        guard let size = PhotoSize(points: CGSize(width: 128, height: 128), scale: 1) else { return nil }
        return await PhotoLoader.load(source, size: size)
    }

    @Test func photoLoaderDecodesLoopbackImage() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let source = try #require(PhotoSource(server.url("picture")))
        let image = try #require(await load(source))
        #expect(image.first.width == 64)
        #expect(image.first.height == 32)
        #expect(server.requests == ["/picture"])
    }

    @Test func photoSourcesRejectNonAbsoluteHTTPAddresses() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let authority = String(server.base.absoluteString.dropFirst("http://".count))
        for refused in ["", "/picture", "//\(authority)/picture", "ftp://\(authority)/picture",
                        server.fileURL.absoluteString, "data:image/png;base64,AAAA", "https://"] {
            #expect(PhotoSource(refused) == nil)
        }
        #expect(PhotoSource(" HTTPS://\(authority)/picture ")?.url.scheme == "https")
        #expect(server.requests.isEmpty)
    }

    @Test func photoLoaderFollowsHTTPRedirectsWithoutCookiesOrReferrers() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let redirect = try #require(PhotoSource(server.url("redirect")))
        #expect(await load(redirect) != nil)
        #expect(server.requests == ["/redirect", "/picture"])
    }

    @Test func photoLoaderRefusesNonHTTPRedirectsAndBoundsRedirectLoops() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let file = try #require(PhotoSource(server.url("file-redirect")))
        let loop = try #require(PhotoSource(server.url("loop")))
        #expect(await load(file) == nil)
        #expect(await load(loop) == nil)
        #expect(server.requests == ["/file-redirect", "/loop"])
    }

    @Test func photoLoaderLimitsBodyAndDecodedSize() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let paths = ["broken", "large", "dimension", "pixels"]
        for path in paths {
            let source = try #require(PhotoSource(server.url(path)))
            #expect(await load(source) == nil, "Unexpectedly decoded \(path)")
        }
        #expect(server.requests == paths.map { "/" + $0 })
    }

    @Test func imageBodyDecodesAt404AndAfterTwentyRedirects() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let missing = try #require(PhotoSource(server.url("missing")))
        let many = try #require(PhotoSource(server.url("hops/20")))
        let tooMany = try #require(PhotoSource(server.url("hops/21")))
        #expect(await load(missing) != nil)
        #expect(await load(many) != nil)
        #expect(await load(tooMany) == nil)
        let throughTwenty = (0...20).reversed().map { "/hops/\($0)" }
        let throughTwentyOne = (1...21).reversed().map { "/hops/\($0)" }
        #expect(server.requests == ["/missing"] + throughTwenty + throughTwentyOne)
        #expect(server.metric("hops-0-User-Agent") == "Image")
        #expect(server.metric("hops-0-Accept-Language") == "*")
    }

    @Test func imageRequestsRefuseCredentialsAndOmitIdentifyingHeaders() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let host = server.base.absoluteString.dropFirst("http://".count)
        if let credentials = PhotoSource("http://photo-user:secret@\(host)/credential-target") {
            _ = await load(credentials)
            Issue.record("A URL with credentials was admitted")
        }
        let redirect = try #require(PhotoSource(server.url("credential-redirect")))
        let challenged = try #require(PhotoSource(server.url("auth")))
        let digest = try #require(PhotoSource(server.url("digest")))
        let ntlm = try #require(PhotoSource(server.url("ntlm")))
        let challengedWithImage = try #require(PhotoSource(server.url("auth-with-image")))
        let headers = try #require(PhotoSource(server.url("headers")))
        #expect(await load(redirect) == nil)
        #expect(await load(challenged) == nil)
        #expect(await load(digest) == nil)
        #expect(await load(ntlm) == nil)
        #expect(await load(challengedWithImage) != nil)
        #expect(await load(headers) != nil)
        #expect(Set(server.requests) == ["/credential-redirect", "/auth", "/digest", "/ntlm", "/auth-with-image", "/headers"])
        #expect(server.metric("headers-User-Agent") == "Image")
        #expect(server.metric("headers-Accept-Language") == "*")
    }

    @Test(arguments: [
        NSURLAuthenticationMethodHTTPBasic, NSURLAuthenticationMethodHTTPDigest,
        NSURLAuthenticationMethodNTLM, NSURLAuthenticationMethodNegotiate,
        NSURLAuthenticationMethodClientCertificate, NSURLAuthenticationMethodServerTrust,
    ])
    func authenticationDelegatesAnswerWithTheExpectedDisposition(_ method: String) async {
        let session = URLSession(configuration: .ephemeral)
        defer { session.invalidateAndCancel() }
        let space = URLProtectionSpace(host: "127.0.0.1", port: 443, protocol: "https",
                                       realm: "photo", authenticationMethod: method)
        let credential = method == NSURLAuthenticationMethodHTTPBasic
            ? URLCredential(user: "fixture", password: "proposed", persistence: .none) : nil
        let challenge = URLAuthenticationChallenge(protectionSpace: space, proposedCredential: credential,
                                                  previousFailureCount: 0, failureResponse: nil, error: nil,
                                                  sender: ChallengeSender())
        let url = URL(fileURLWithPath: "/")
        let redirects = PhotoRedirects(url)
        let expected: URLSession.AuthChallengeDisposition = method == NSURLAuthenticationMethodServerTrust
            ? .performDefaultHandling : .rejectProtectionSpace
        let sessionReply = await withCheckedContinuation { continuation in
            PhotoAuthentication().urlSession(session, didReceive: challenge) { disposition, credential in
                continuation.resume(returning: (disposition, credential == nil))
            }
        }
        let task = session.dataTask(with: url)
        let taskReply = await withCheckedContinuation { continuation in
            redirects.urlSession(session, task: task, didReceive: challenge) { disposition, credential in
                continuation.resume(returning: (disposition, credential == nil))
            }
        }
        #expect(sessionReply.0 == expected && sessionReply.1, "Session delegate: \(method)")
        #expect(taskReply.0 == expected && taskReply.1, "Task delegate: \(method)")
    }

    @Test func imageBodyReaderClosesTheConnectionBeforeReadingTheExcess() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let source = try #require(PhotoSource(server.url("streamed")))
        let started = ContinuousClock.now
        #expect(await load(source) == nil)
        try await waitForCollectionEvent("the fixture's streamed-body byte count") {
            server.metric("streamed") != nil
        }
        let sent = try #require(server.metric("streamed").flatMap(Int.init))
        print("photo-body-limit transport sent \(sent) bytes")
        // Kernel receive buffers and CFNetwork can read ahead beyond 8 MiB.
        #expect((8 * 1024 * 1024...16 * 1024 * 1024).contains(sent), "Server sent \(sent) bytes")
        #expect(started.duration(to: .now) < .seconds(10))
    }

    @Test func photoLoaderTimesOutWhenAResponseStalls() async throws {
        let server = try await PhotoLoopback.start()
        defer { server.stop() }
        let source = try #require(PhotoSource(server.url("stalled")))
        // The fixture eventually sends a valid PNG. A request that waits for
        // it would decode, regardless of when Swift Testing resumes the test.
        #expect(await load(source) == nil)
        #expect(server.requests.first == "/stalled")
    }
}


private final class ChallengeSender: NSObject, URLAuthenticationChallengeSender {
    func use(_ credential: URLCredential, for challenge: URLAuthenticationChallenge) {}
    func continueWithoutCredential(for challenge: URLAuthenticationChallenge) {}
    func cancel(_ challenge: URLAuthenticationChallenge) {}
}
