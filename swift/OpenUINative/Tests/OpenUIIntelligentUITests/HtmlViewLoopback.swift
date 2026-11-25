import Foundation
import Security
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

    static func start() async throws -> HtmlViewLoopback {
        let directory = FileManager.default.temporaryDirectory.appending(path: "HtmlView-\(UUID())", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        let server = Process()
        server.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        server.arguments = ["-c", script, directory.path]
        server.currentDirectoryURL = directory
        do {
            try server.run()
            for _ in 0..<100 {
                if let fixture = try? HtmlViewLoopback(directory: directory, server: server) { return fixture }
                guard server.isRunning else { break }
                try await Task.sleep(for: .milliseconds(50))
            }
            throw CollectionInputFailure(description: "The local HtmlView fixture did not start")
        } catch {
            stop(server, directory: directory)
            throw error
        }
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
        if server.isRunning {
            server.terminate()
            server.waitUntilExit()
        }
        try? FileManager.default.removeItem(at: directory)
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
    ], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
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
    https.serve_forever()
    """#
}

// Forward navigation policy and callbacks to the real coordinator. The one
// change is trusting the fixture's exact leaf on its loopback port.
@MainActor final class HtmlViewFixtureTrust: NSObject, WKNavigationDelegate {
    private weak var original: (any WKNavigationDelegate)?
    private let port: Int
    private let certificate: Data

    init(webView: WKWebView, url: URL, certificate: Data) throws {
        original = webView.navigationDelegate
        port = try #require(url.port)
        self.certificate = certificate
        super.init()
        webView.navigationDelegate = self
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                 decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        if original?.webView?(webView, decidePolicyFor: action, decisionHandler: decisionHandler) == nil {
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
        guard protection.host == "127.0.0.1", protection.port == port,
              protection.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = protection.serverTrust,
              let leaf = (SecTrustCopyCertificateChain(trust) as? [SecCertificate])?.first,
              SecCertificateCopyData(leaf) as Data == certificate else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        completionHandler(.useCredential, URLCredential(trust: trust))
    }
}
