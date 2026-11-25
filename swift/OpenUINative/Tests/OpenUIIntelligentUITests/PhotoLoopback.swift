import Foundation
import Testing

// A local HTTP fixture with generated pixels and a log of requests.
@MainActor final class PhotoLoopback {
    private let directory: URL
    private let server: Process
    let base: URL

    private init(directory: URL, server: Process, port: UInt16, ipv6: Bool) throws {
        self.directory = directory
        self.server = server
        base = try #require(URL(string: "http://\(ipv6 ? "[::1]" : "127.0.0.1"):\(port)"))
    }

    static func start(ipv6: Bool = false) async throws -> PhotoLoopback {
        let directory = FileManager.default.temporaryDirectory.appending(path: "Photo-\(UUID())")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
        let server = Process()
        server.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
        server.currentDirectoryURL = directory
        server.arguments = ["-c", script, ipv6 ? "6" : "4"]
        do {
            try server.run()
            for _ in 0..<100 {
                let ready = try? String(contentsOf: directory.appending(path: "ready"), encoding: .utf8)
                if let ready, let port = UInt16(ready) {
                    return try PhotoLoopback(directory: directory, server: server, port: port, ipv6: ipv6)
                }
                guard server.isRunning else { break }
                try await Task.sleep(for: .milliseconds(25))
            }
            throw CollectionInputFailure(description: "The local Image fixture did not start")
        } catch {
            stop(server, directory: directory)
            throw error
        }
    }

    func url(_ path: String) -> String { base.appending(path: path).absoluteString }
    func serveSVG(_ xml: String, as name: String) throws {
        try Data(xml.utf8).write(to: directory.appending(path: "vector-" + name))
    }

    var fileURL: URL { directory.appending(path: "local.png") }

    var requests: [String] {
        let log = try? String(contentsOf: directory.appending(path: "requests"), encoding: .utf8)
        return log?.split(separator: "\n").map(String.init) ?? []
    }

    func stop() { Self.stop(server, directory: directory) }

    func metric(_ name: String) -> String? {
        try? String(contentsOf: directory.appending(path: name), encoding: .utf8)
    }

    private static func stop(_ server: Process, directory: URL) {
        if server.isRunning {
            server.terminate()
            server.waitUntilExit()
        }
        try? FileManager.default.removeItem(at: directory)
    }

    private static let script = #"""
    import http.server
    import pathlib
    import socket
    import struct
    import threading
    import time
    import sys
    import zlib

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    def png(width, height, rows, depth=8):
        header = struct.pack(">IIBBBBB", width, height, depth, 2, 0, 0, 0)
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) +
                chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))

    # A 2:1 photo with equal red and blue halves.
    sample = png(64, 32, (b"\x00" + b"\xff\x00\x00" * 32 + b"\x00\x00\xff" * 32) * 32)
    pathlib.Path("local.png").write_bytes(sample)
    log_lock = threading.Lock()
    active = peak = 0
    held = {}

    def write_metric(name, value):
        temp = pathlib.Path(name + ".tmp")
        temp.write_text(str(value))
        temp.replace(name)

    def gif(count):
        return (b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\x00\x00\x00\x00\xff" +
                b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00" +
                b"".join(b"\x21\xf9\x04\x00\x32\x00\x00\x00"
                         b"\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02" +
                         (b"\x44" if i % 2 == 0 else b"\x4c") + b"\x01\x00" for i in range(count)) +
                b"\x3b")

    square = png(1024, 1024, (b"\x00" + b"\xff\x00\x00" * 1024) * 1024)

    class Fixture(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            with log_lock, open("requests", "a") as log:
                log.write(self.path + "\n")
                for header in ("Cookie", "Referer", "Authorization"):
                    if header in self.headers:
                        log.write(header + "\n")
                if self.path == "/headers" or self.path == "/hops/0":
                    for header in ("User-Agent", "Accept-Language"):
                        write_metric(self.path[1:].replace("/", "-") + "-" + header,
                                     self.headers.get(header, "<absent>"))
            if self.path.startswith("/hold"):
                with log_lock:
                    held[self.path] = held.get(self.path, 0) + 1
                    write_metric(self.path[1:], held[self.path])
                self.send_response(200)
                self.send_header("Content-Length", str(len(sample)))
                self.end_headers()
                self.connection.settimeout(4)
                try:
                    self.connection.recv(1)
                except (ConnectionResetError, socket.timeout):
                    pass
                finally:
                    with log_lock:
                        held[self.path] -= 1
                        write_metric(self.path[1:], held[self.path])
                return
            if self.path.startswith("/busy"):
                global active, peak
                with log_lock:
                    active += 1
                    peak = max(peak, active)
                    write_metric("peak", peak)
                try:
                    time.sleep(0.8)
                finally:
                    with log_lock:
                        active -= 1
            if self.path.startswith("/hops/"):
                hops = int(self.path.split("/")[-1])
                if hops:
                    self.send_response(302)
                    self.send_header("Location", "/hops/" + str(hops - 1))
                    self.end_headers()
                    return
            redirects = {"/redirect": "/picture", "/loop": "/loop",
                         "/credential-redirect": "http://photo-user:secret@127.0.0.1:%d/credential-target" % self.server.server_port,
                         "/file-redirect": pathlib.Path("local.png").resolve().as_uri()}
            if self.path in redirects:
                self.send_response(302)
                self.send_header("Location", redirects[self.path])
                self.send_header("Set-Cookie", "photo=true")
                self.end_headers()
                return
            if self.path in ("/auth", "/digest", "/ntlm", "/auth-with-image", "/credential-target") and "Authorization" not in self.headers:
                self.send_response(401)
                challenge = {"/digest": 'Digest realm="photo", nonce="photo", qop="auth"',
                             "/ntlm": "NTLM"}.get(self.path, 'Basic realm="photo"')
                self.send_header("WWW-Authenticate", challenge)
                challenge_body = sample if self.path == "/auth-with-image" else b""
                self.send_header("Content-Length", str(len(challenge_body)))
                self.end_headers()
                self.wfile.write(challenge_body)
                return
            if self.path == "/stalled":
                time.sleep(12)
            body = sample
            kind = "image/png"
            if self.path == "/broken":
                body = b"not an image"
            elif self.path.startswith("/square"):
                body = square
            elif self.path == "/large16":
                body = png(4096, 4096, (b"\x00" + b"\xff\xff\x00\x00\x00\x00" * 4096) * 4096, 16)
            elif self.path in ("/animated", "/many-frames"):
                kind = "image/gif"
                body = gif(2 if self.path == "/animated" else 61)
            elif self.path in ("/svg", "/svg-active"):
                kind = "image/svg+xml"
                body = b'<svg xmlns="http://www.w3.org/2000/svg" width="64" height="32"><rect width="64" height="32" fill="red"/></svg>'
                if self.path == "/svg-active":
                    address = ("http://127.0.0.1:%d/" % self.server.server_port).encode()
                    body = (b'<?xml-stylesheet href="' + address + b'external-css" type="text/css"?>' +
                            body[:-6] + b'<image href="' + address + b'external-image" width="64" height="32"/>' +
                            b'<style>@import url("' + address + b'css-import");</style>' +
                            b'<script>fetch("' + address + b'script");</script>' +
                            b'<rect width="1" height="1" fill="url(' + address + b'external-fill) none"/></svg>')
            elif self.path.startswith("/vector-"):
                kind = "image/svg+xml"
                body = pathlib.Path(self.path[1:]).read_bytes()
            elif self.path == "/dimension":
                body = png(8193, 1, bytes(1 + 8193 * 3))
            elif self.path == "/pixels":
                body = png(4097, 4096, bytes((1 + 4097 * 3) * 4096))
            elif self.path == "/large":
                body += bytes(8 * 1024 * 1024 + 1 - len(body))
            self.send_response(404 if self.path == "/missing" else 200)
            self.send_header("Content-Type", kind)
            if self.path != "/streamed":
                self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if self.path == "/streamed":
                sent = 0
                try:
                    for _ in range(1024):
                        self.wfile.write(bytes(64 * 1024))
                        sent += 64 * 1024
                except (BrokenPipeError, ConnectionResetError):
                    pass
                finally:
                    write_metric("streamed", sent)
                return
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def log_message(self, format, *args):
            pass

    class IPv6Server(http.server.ThreadingHTTPServer):
        address_family = socket.AF_INET6

    server = (IPv6Server(("::1", 0), Fixture) if sys.argv[1] == "6" else
              http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture))
    pathlib.Path("starting").write_text(str(server.server_port))
    pathlib.Path("starting").rename("ready")
    server.serve_forever()
    """#
}
