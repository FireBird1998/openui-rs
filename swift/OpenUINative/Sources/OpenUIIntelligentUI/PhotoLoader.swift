import Foundation
import Synchronization

struct PhotoSource: Hashable, Sendable {
    let url: URL

    init?(_ text: String) {
        guard let url = webURL(text), Self.admits(url) else { return nil }
        self.url = url
    }

    static func admits(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased(), scheme == "http" || scheme == "https",
              let host = url.host(percentEncoded: false), !host.isEmpty else { return false }
        return url.user(percentEncoded: false) == nil && url.password(percentEncoded: false) == nil
    }
}

enum PhotoLoader {
    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = PhotoLimits.inactivityTimeout
        configuration.timeoutIntervalForResource = PhotoLimits.resourceTimeout
        configuration.urlCache = nil
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.urlCredentialStorage = nil
        return URLSession(configuration: configuration, delegate: PhotoAuthentication(), delegateQueue: nil)
    }()

    // Keep network iteration and decoding off SwiftUI's main actor.
    @concurrent static func load(_ source: PhotoSource, size: PhotoSize) async -> PhotoRaster? {
        do {
            try Task.checkCancellation()
            let redirects = PhotoRedirects(source.url)
            let (bytes, response) = try await session.bytes(for: request(to: source.url), delegate: redirects)
            let task = bytes.task
            defer { task.cancel() }
            guard let http = response as? HTTPURLResponse, !redirects.refused,
                  http.expectedContentLength <= PhotoLimits.bodyBytes else { return nil }
            return try await withTaskCancellationHandler {
                var body = Data()
                var input = bytes.makeAsyncIterator()
                for _ in 0...PhotoLimits.bodyBytes {
                    guard let byte = try await input.next() else {
                        return Task.isCancelled ? nil : PhotoRaster.decode(body, size: size)
                    }
                    guard body.count < PhotoLimits.bodyBytes else { return nil }
                    body.append(byte)
                }
                return nil
            } onCancel: {
                task.cancel()
            }
        } catch {
            return nil
        }
    }

    fileprivate static func request(to url: URL) -> URLRequest {
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData)
        request.httpShouldHandleCookies = false
        // CFNetwork supplies process and locale defaults for unset headers.
        request.setValue("Image", forHTTPHeaderField: "User-Agent")
        request.setValue("*", forHTTPHeaderField: "Accept-Language")
        return request
    }
}

final class PhotoRedirects: NSObject, URLSessionTaskDelegate {
    private struct Route {
        var visited: Set<URL>
        var refused = false
    }

    private let route: Mutex<Route>
    var refused: Bool { route.withLock { $0.refused } }

    init(_ url: URL) {
        route = Mutex(Route(visited: [Self.withoutFragment(url)]))
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void) {
        let destination = route.withLock { route -> URL? in
            // Check the original Location as well as CFNetwork's proposed URL.
            guard let from = response.url, let location = response.value(forHTTPHeaderField: "Location"),
                  let resolved = URL(string: location, relativeTo: from)?.absoluteURL, PhotoSource.admits(resolved),
                  let url = request.url, PhotoSource.admits(url),
                  route.visited.count <= PhotoLimits.redirects, route.visited.insert(Self.withoutFragment(url)).inserted else {
                route.refused = true
                return nil
            }
            return url
        }
        completionHandler(destination.map { PhotoLoader.request(to: $0) })
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didReceive challenge: URLAuthenticationChallenge,
                    completionHandler: @escaping @Sendable (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        completionHandler(PhotoAuthentication.disposition(for: challenge), nil)
    }

    private static func withoutFragment(_ url: URL) -> URL {
        var parts = URLComponents(url: url, resolvingAgainstBaseURL: true)
        parts?.fragment = nil
        return parts?.url ?? url
    }
}

final class PhotoAuthentication: NSObject, URLSessionDelegate {
    func urlSession(_ session: URLSession, didReceive challenge: URLAuthenticationChallenge,
                    completionHandler: @escaping @Sendable (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        completionHandler(Self.disposition(for: challenge), nil)
    }

    static func disposition(for challenge: URLAuthenticationChallenge) -> URLSession.AuthChallengeDisposition {
        // TLS uses system trust. Both delegate entry points refuse
        // credential-bearing challenges, including client certificates.
        challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust
            ? .performDefaultHandling : .rejectProtectionSpace
    }
}
