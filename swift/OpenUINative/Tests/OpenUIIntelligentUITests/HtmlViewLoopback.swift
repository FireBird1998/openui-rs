import Darwin
import Foundation
import Security
import SwiftTestSupport
import Testing
import WebKit

// The fixture listens only on loopback. Each test owns its subprocess and files.
@MainActor final class HtmlViewLoopback {
    let http: URL
    let https: URL
    let certificate: Data
    let directory: URL
    private let server: Process?

    var fileProbe: URL { directory.appending(path: "file.js") }
    var requests: [String] {
        let log = try? String(contentsOf: directory.appending(path: "requests"), encoding: .utf8)
        return (log ?? "").split(separator: "\n").map(String.init)
    }

    // A helper-startup bound matching the shared watchdog's 60 s. It stays
    // local: httpCacheIsSeparateAcrossViewsAndAppProcesses starts the fixture
    // under a longer named watchdog limit (its fresh-process check needs 2
    // minutes), and unlike the watchdog this bound kills the child and fails
    // in-process.
    static let readinessBound: Duration = .seconds(60)

    static func start() async throws -> HtmlViewLoopback {
        NativeTestWatchdog.beginWaiting("the HtmlView loopback server to start and publish its ports")
        defer { NativeTestWatchdog.finishedWaiting() }
        let directory = FileManager.default.temporaryDirectory.appending(path: "HtmlView-\(UUID())", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        let server = Process()
        server.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        server.arguments = ["-c", script, directory.path]
        server.currentDirectoryURL = directory
        let readiness = Pipe()
        server.standardOutput = readiness
        do {
            try server.run()
            try await awaitReadiness(of: server, on: readiness, within: readinessBound)
            return try HtmlViewLoopback(directory: directory, server: server)
        } catch {
            stop(server, directory: directory)
            throw error
        }
    }

    // The child writes one `R` to `readiness` once its ports are published.
    // Waits at most `bound`; on expiry the child is killed and the wait fails.
    // A child that exits or closes the pipe first fails the wait at once.
    static func awaitReadiness(of child: Process, on readiness: Pipe, within bound: Duration) async throws {
        let reader = readiness.fileHandleForReading
        defer { try? reader.close() }
        let pid = child.processIdentifier
        NativeTestWatchdog.beginWaiting("the HtmlView fixture PID \(pid) to signal readiness")
        defer { NativeTestWatchdog.finishedWaiting() }
        let clock = ContinuousClock()
        let deadline = clock.now + bound
        let limit = "the HtmlView fixture didn't signal readiness within \(bound.components.seconds) s; PID \(pid)"
        while true {
            var descriptor = pollfd(fd: reader.fileDescriptor, events: Int16(POLLIN), revents: 0)
            if poll(&descriptor, 1, 0) > 0 {
                var byte: UInt8 = 0
                let count = read(reader.fileDescriptor, &byte, 1)
                if count == 1, byte == UInt8(ascii: "R") {
                    if child.isRunning { return }
                    throw CollectionInputFailure(description: "\(limit) signalled readiness, then \(exitDescription(child)).")
                }
                if count == 1 {
                    forceExit(child)
                    throw CollectionInputFailure(description: "\(limit) sent \(byte) instead of the readiness signal.")
                }
                // End of file (or a read error): the child closed its stdout without signalling.
                _ = waitForExit(child, for: .seconds(2))
                let ending = exitDescription(child)
                forceExit(child)
                throw CollectionInputFailure(description: "\(limit) \(ending) before signalling.")
            }
            if !child.isRunning {
                throw CollectionInputFailure(description: "\(limit) \(exitDescription(child)) before signalling.")
            }
            if clock.now >= deadline {
                forceExit(child)
                throw CollectionInputFailure(description: "\(limit) was still running at the bound and was killed.")
            }
            do { try await clock.sleep(until: min(deadline, clock.now + .milliseconds(20))) }
            catch { forceExit(child); throw error }
        }
    }

    private static func exitDescription(_ child: Process) -> String {
        child.isRunning ? "closed its readiness pipe while running"
            : "exited with status \(child.terminationStatus)"
    }

    private static func forceExit(_ child: Process) {
        guard child.isRunning else { return }
        Darwin.kill(child.processIdentifier, SIGKILL)
        _ = waitForExit(child, for: .seconds(2))
    }

    init(directory: URL, server: Process? = nil) throws {
        self.server = server
        self.directory = directory
        let addresses = try String(contentsOf: directory.appending(path: "ready"), encoding: .utf8).split(separator: " ")
        try #require(addresses.count == 2)
        let httpPort = try #require(addresses.first)
        let httpsPort = try #require(addresses.last)
        http = try #require(URL(string: "http://127.0.0.1:\(httpPort)"))
        https = try #require(URL(string: "https://127.0.0.1:\(httpsPort)"))
        certificate = try Data(contentsOf: directory.appending(path: "certificate.der"))
    }

    func stop() {
        if let server { Self.stop(server, directory: directory) }
    }

    private static func stop(_ server: Process, directory: URL) {
        NativeTestWatchdog.beginWaiting("the HtmlView loopback server to exit")
        defer { NativeTestWatchdog.finishedWaiting() }
        if server.isRunning {
            server.terminate()
            if !waitForExit(server, for: .seconds(2)) {
                let pid = server.processIdentifier
                if server.isRunning { kill(pid, SIGKILL) }
                let exited = waitForExit(server, for: .seconds(1))
                Issue.record("HtmlView fixture PID \(pid) exceeded its shutdown deadline. Exited after SIGKILL: \(exited).")
            }
        }
        try? FileManager.default.removeItem(at: directory)
    }

    private static func waitForExit(_ server: Process, for timeout: Duration) -> Bool {
        let deadline = ContinuousClock.now + timeout
        while server.isRunning && ContinuousClock.now < deadline {
            Thread.sleep(forTimeInterval: 0.02)
        }
        return !server.isRunning
    }

    // Python and OpenSSL ship with the macOS test tools. The leaf is generated
    // per fixture and accepted only by the test delegate below.
    private static let script = #"""
    import http.server
    import pathlib
    import ssl
    import subprocess
    import sys
    import threading

    directory = pathlib.Path(sys.argv[1])
    subprocess.run([
        "/usr/bin/openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
        "-keyout", "key.pem", "-out", "certificate.pem", "-days", "1",
        "-subj", "/CN=127.0.0.1", "-addext", "subjectAltName=IP:127.0.0.1",
    ], check=True, timeout=5, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    (directory / "certificate.der").write_bytes(
        ssl.PEM_cert_to_DER_cert((directory / "certificate.pem").read_text()))
    (directory / "file.js").write_text("document.body.append('local file ran');")
    log_lock = threading.Lock()

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            with log_lock, (directory / "requests").open("a") as log:
                log.write(self.path + "\n")
            if self.path == "/cache.svg":
                body = b'<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1"/></svg>'
                self.send_response(200)
                self.send_header("Content-Type", "image/svg+xml")
                self.send_header("Cache-Control", "public, max-age=3600")
            else:
                body = b"loopback target reached"
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        do_POST = do_GET

        def log_message(self, format, *args):
            pass

    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    https = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain("certificate.pem", "key.pem")
    https.socket = tls.wrap_socket(https.socket, server_side=True)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    (directory / "starting").write_text(f"{httpd.server_port} {https.server_port}")
    (directory / "starting").rename(directory / "ready")
    print("R", end="", flush=True)
    sys.stdout = open("/dev/null", "w")  # the parent stops reading after R; later prints must not hit a closed pipe
    https.serve_forever()
    """#
}

// Forward navigation to the real coordinator. Tests can observe its policy
// decision or trust the fixture's exact leaf on its loopback port.
@MainActor final class HtmlViewFixtureNavigation: NSObject, WKNavigationDelegate {
    private weak var original: (any WKNavigationDelegate)?
    private let trust: (port: Int, certificate: Data)?
    private let observePolicy: @MainActor (WKNavigationAction, WKNavigationActionPolicy) -> Void

    init(webView: WKWebView, trust: (URL, Data)? = nil,
         observePolicy: @escaping @MainActor (WKNavigationAction, WKNavigationActionPolicy) -> Void = { _, _ in }) throws {
        original = webView.navigationDelegate
        self.trust = try trust.map { (try #require($0.0.port), $0.1) }
        self.observePolicy = observePolicy
        super.init()
        webView.navigationDelegate = self
    }

    func restore(in webView: WKWebView) {
        if webView.navigationDelegate === self { webView.navigationDelegate = original }
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        let forward: @MainActor (WKNavigationActionPolicy) -> Void = { policy in
            decisionHandler(policy)
            self.observePolicy(action, policy)
        }
        if original?.webView?(webView, decidePolicyFor: action, decisionHandler: forward) == nil {
            decisionHandler(.cancel)
        }
    }

    func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
        original?.webView?(webView, didCommit: navigation)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        original?.webView?(webView, didFinish: navigation)
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        original?.webView?(webView, didFailProvisionalNavigation: navigation, withError: error)
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        original?.webView?(webView, didFail: navigation, withError: error)
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        original?.webViewWebContentProcessDidTerminate?(webView)
    }

    func webView(_ webView: WKWebView, didReceive challenge: URLAuthenticationChallenge,
                 completionHandler: @escaping @MainActor (URLSession.AuthChallengeDisposition, URLCredential?) -> Void) {
        let protection = challenge.protectionSpace
        guard let trust, protection.host == "127.0.0.1", protection.port == trust.port,
              protection.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let serverTrust = protection.serverTrust,
              let leaf = (SecTrustCopyCertificateChain(serverTrust) as? [SecCertificate])?.first,
              SecCertificateCopyData(leaf) as Data == trust.certificate else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        completionHandler(.useCredential, URLCredential(trust: serverTrust))
    }
}
