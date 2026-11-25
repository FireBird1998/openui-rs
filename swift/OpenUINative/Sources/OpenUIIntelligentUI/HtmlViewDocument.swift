// The policy, base style and reporter come from the pinned Intelligent UI
// components/HtmlView/HtmlView.tsx (7b7a519, MIT).

struct HtmlViewDocument: Hashable, Sendable {
    let html: String
    let dark: Bool

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.dark == rhs.dark && lhs.html.utf8.elementsEqual(rhs.html.utf8)
    }

    private var scheme: String { dark ? "dark" : "light" }

    private static let cdns = "https://cdn.jsdelivr.net https://unpkg.com https://cdnjs.cloudflare.com https://esm.sh"
    private static let policy = [
        "default-src 'none'",
        "script-src 'unsafe-inline' \(cdns)",
        "style-src 'unsafe-inline' \(cdns) https://fonts.googleapis.com",
        "font-src https://fonts.gstatic.com data:",
        "img-src data: blob: https:",
        "media-src data: blob:",
        "connect-src \(cdns)",
        "frame-src 'none'",
        "form-action 'none'",
        "base-uri 'none'",
    ].joined(separator: "; ")

    private static let reporter = """
    <script>
    (() => {
      let last = 0;
      function send() {
        const height = Math.ceil(document.documentElement.getBoundingClientRect().height);
        if (Math.abs(height - last) > 1) {
          last = height;
          parent.postMessage({type: "openui-intelligent-html-view-height", height}, "*");
        }
      }
      addEventListener("message", event => {
        if (event.source === parent && event.data === "openui-intelligent-html-view-measure") {
          last = 0;
          send();
        }
      });
      addEventListener("load", send);
      new ResizeObserver(send).observe(document.documentElement);
      send();
    })();
    </script>
    """

    var page: String {
        """
        <!doctype html><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="\(Self.policy)"><meta name="viewport" content="width=device-width,initial-scale=1"><style>:root{color-scheme:\(scheme)}html,body{margin:0}body{font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif}</style>\(Self.reporter)\(html)
        """
    }

    func wrapper() throws -> String {
        // Only an escaped attribute of the trusted document contains model
        // bytes. WebKit decodes it back to HTML inside the sandboxed frame.
        let pageBytes = page.utf8
        let ampersand = "&amp;".utf8
        let quote = "&quot;".utf8
        let lessThan = "&lt;".utf8
        let greaterThan = "&gt;".utf8
        var escaped: [UInt8] = []
        escaped.reserveCapacity(pageBytes.count)
        for (offset, byte) in pageBytes.enumerated() {
            if offset.isMultiple(of: 4096) { try Task.checkCancellation() }
            switch byte {
            case 38: escaped.append(contentsOf: ampersand)
            case 34: escaped.append(contentsOf: quote)
            case 60: escaped.append(contentsOf: lessThan)
            case 62: escaped.append(contentsOf: greaterThan)
            default: escaped.append(byte)
            }
        }
        let source = String(decoding: escaped, as: UTF8.self)
        return """
        <!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer">
        <style>
        :root{color-scheme:\(scheme)}html,body{width:100%;height:100%;margin:0;overflow:hidden}
        iframe{display:block;width:100%;height:100%;border:0}
        </style>
        <iframe sandbox="allow-scripts" referrerpolicy="no-referrer" title="Interactive view" srcdoc="\(source)"></iframe>
        """
    }
}
