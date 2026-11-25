import OpenUINative
import SwiftUI
import WebKit

/// If an HtmlView script loops forever, the app and other HtmlView pages stay responsive and dismissing the view releases it.
/// The page's WebContent process keeps running at background priority, using about one available CPU core and 29 MiB of resident memory until the app quits, because WebKit exposes no public API to end it sooner.
struct HtmlView: View {
    let element: Element
    @Environment(\.liveProgram) private var live
    @Environment(\.intelligentTheme) private var theme
    @Environment(\.colorScheme) private var scheme
    @State private var measuredHeight: CGFloat?

    private var fixedHeight: CGFloat? {
        guard let height = element.prop("height")?.ruleScalar?.number, height > 0 else { return nil }
        return Self.clamp(height)
    }

    private var title: String {
        let title = element.prop("title")?.ruleScalar?.string ?? ""
        return title.isEmpty ? "Interactive view" : title
    }

    private static func clamp(_ height: Double) -> CGFloat { min(2400, max(80, height)) }

    var body: some View {
        let fixed = fixedHeight
        let height = fixed ?? measuredHeight ?? 360
        let outline = RoundedRectangle(cornerRadius: theme.radiusLg, style: .circular)
        Group {
            if live?.isStreaming == true {
                HtmlViewStatus(text: "Preparing the interactive view…", color: NSColor(theme.muted.color))
            } else {
                let document = HtmlViewDocument(html: element.prop("html")?.ruleScalar?.string ?? "", dark: scheme == .dark)
                HtmlViewPage(document: document, title: title, autoSizing: fixed == nil) {
                    measuredHeight = Self.clamp($0)
                }
                .id(document)
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity)
        .frame(height: height - 2)
        .clipContainer(to: RoundedRectangle(cornerRadius: theme.radiusLg - 1, style: .circular))
        .padding(1)
        .background(theme.canvas.color, in: outline)
        .overlay(outline.strokeBorder(theme.border.color).allowsHitTesting(false))
    }
}

private struct HtmlViewStatus: NSViewRepresentable {
    let text: String
    var color: NSColor = .labelColor

    func makeNSView(context: Context) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.font = .systemFont(ofSize: 13)
        label.alignment = .center
        label.setAccessibilityElement(true)
        label.setAccessibilityRole(.staticText)
        return label
    }

    func updateNSView(_ label: NSTextField, context: Context) {
        label.stringValue = text
        label.textColor = color
        label.setAccessibilityLabel(text)
    }
}

private struct HtmlViewPage: View {
    let document: HtmlViewDocument
    let title: String
    let autoSizing: Bool
    let onHeight: (Double) -> Void
    @State private var stopped = false
    @State private var hovering = false
    @FocusState private var stopFocused: Bool

    var body: some View {
        if stopped {
            VStack(spacing: 4) {
                HtmlViewStatus(text: "Interactive view stopped")
                Button("Reload") { stopped = false }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            HtmlViewBrowser(document: document, title: title, autoSizing: autoSizing, onHeight: onHeight) {
                stopped = true
            }
            // WebKit does not report a stalled script through its public
            // delegate. This control works without a reply from the page.
            .overlay(alignment: .topTrailing) {
                Button { stopped = true } label: {
                    Image(systemName: "stop.circle")
                }
                .buttonStyle(.bordered)
                .accessibilityLabel("Stop interactive view")
                .help("Stop interactive view")
                .focused($stopFocused)
                .opacity(hovering || stopFocused ? 1 : 0)
                .allowsHitTesting(hovering || stopFocused)
                .padding(4)
            }
            .onHover { hovering = $0 }
            .accessibilityAction(named: "Stop interactive view") { stopped = true }
        }
    }
}

struct HtmlViewBrowser: NSViewRepresentable {
    let document: HtmlViewDocument
    let title: String
    let autoSizing: Bool
    let onHeight: (Double) -> Void
    let onStop: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(autoSizing: autoSizing, onHeight: onHeight, onStop: onStop)
    }

    func makeNSView(context: Context) -> NSView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false

        let coordinator = context.coordinator
        let content = configuration.userContentController
        content.addUserScript(WKUserScript(source: Self.heightRelay(revision: coordinator.activeRevision),
                                           injectionTime: .atDocumentStart, forMainFrameOnly: true, in: .page))
        content.addUserScript(WKUserScript(source: Self.heightBridge,
                                           injectionTime: .atDocumentStart, forMainFrameOnly: true, in: .defaultClient))
        content.add(coordinator, contentWorld: .defaultClient, name: "htmlViewHeight")

        let surface = NSView()
        let webView = WKWebView(frame: surface.bounds, configuration: configuration)
        webView.autoresizingMask = [.width, .height]
        webView.navigationDelegate = coordinator
        webView.allowsLinkPreview = false
        webView.setAccessibilityLabel(title)
        surface.addSubview(webView)
        coordinator.prepare(document, in: webView)
        return surface
    }

    func updateNSView(_ surface: NSView, context: Context) {
        let coordinator = context.coordinator
        coordinator.webView?.setAccessibilityLabel(title)
        coordinator.onHeight = onHeight
        coordinator.onStop = onStop
        coordinator.setAutoSizing(autoSizing)
    }

    static func dismantleNSView(_ surface: NSView, coordinator: Coordinator) {
        coordinator.detach()
    }

    // The wrapper receives the frame's postMessage in the page world, as
    // upstream does. Both scripts run only in the trusted main frame.
    private static func heightRelay(revision: Int?) -> String {
        """
        (() => {
          let revision = \(revision.map(String.init) ?? "null");
          let pending;
          let scheduled = false;
          function measure() {
            if (revision !== null)
              document.querySelector("iframe")?.contentWindow?.postMessage("openui-intelligent-html-view-measure", "*");
          }
          globalThis.openUISetHeightRevision = value => {
            if (value === revision) return;
            revision = value;
            pending = undefined;
            measure();
          };
          addEventListener("message", event => {
            const frame = document.querySelector("iframe");
            const data = event.data;
            if (revision === null || !frame || event.source !== frame.contentWindow ||
                data?.type !== "openui-intelligent-html-view-height" ||
                !Number.isFinite(data.height)) return;
            pending = [revision, data.height];
            if (scheduled) return;
            scheduled = true;
            // Height reports still need to arrive when WebKit pauses painting.
            setTimeout(() => {
              scheduled = false;
              if (pending) {
                document.documentElement.setAttribute("data-openui-height", JSON.stringify(pending));
                document.dispatchEvent(new Event("openui-html-view-height-ready"));
              }
              pending = undefined;
            }, 0);
          });
          addEventListener("DOMContentLoaded", measure);
        })();
        """
    }

    // The two worlds share the wrapper DOM. Only the isolated world has the
    // Swift handler, and the opaque-origin child cannot touch the wrapper DOM.
    private static let heightBridge = """
    document.addEventListener("openui-html-view-height-ready", () => {
      const report = document.documentElement.getAttribute("data-openui-height");
      if (report !== null)
        window.webkit.messageHandlers.htmlViewHeight.postMessage(JSON.parse(report));
    });
    """

    @MainActor final class Coordinator: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
        var onHeight: (Double) -> Void
        var onStop: () -> Void
        private(set) weak var webView: WKWebView?
        private(set) var documentFrame: WKFrameInfo?
        private(set) var lastHeight: Double?
        private(set) var lastProvisionalFailure: NSError?
        private(set) var lastNavigationFailure: NSError?
        private(set) var lastRelayFailure: NSError?
        private(set) var contentProcessTerminated = false
        private var autoSizing: Bool
        private var revision = 0
        private var preparation: Task<Void, Never>?
        private var initialNavigation: WKNavigation?
        private var committed = false
        private var openedWrapper = false
        private var invalidated = false

        var activeRevision: Int? { autoSizing ? revision : nil }

        init(autoSizing: Bool, onHeight: @escaping (Double) -> Void, onStop: @escaping () -> Void) {
            self.autoSizing = autoSizing
            self.onHeight = onHeight
            self.onStop = onStop
        }

        func prepare(_ document: HtmlViewDocument, in webView: WKWebView) {
            self.webView = webView
            preparation = Task.detached(priority: .userInitiated) { [weak self] in
                guard let wrapper = try? document.wrapper() else { return }
                await self?.load(wrapper)
            }
        }

        private func load(_ wrapper: String) {
            guard !invalidated, let webView else { return }
            preparation = nil
            initialNavigation = webView.loadHTMLString(wrapper, baseURL: nil)
            if initialNavigation == nil { stop() }
        }

        func detach() {
            invalidate()
            guard let webView else { return }
            webView.navigationDelegate = nil
            webView.configuration.userContentController.removeScriptMessageHandler(
                forName: "htmlViewHeight", contentWorld: .defaultClient)
            webView.stopLoading()
            // Release the browser even if SwiftUI retains the representable's
            // empty AppKit surface. No WebContent reply is needed.
            webView.removeFromSuperview()
        }

        private func invalidate() {
            invalidated = true
            preparation?.cancel()
            preparation = nil
        }

        private func stop() {
            guard !invalidated else { return }
            invalidate()
            onStop()
        }

        func setAutoSizing(_ enabled: Bool) {
            guard enabled != autoSizing, !invalidated else { return }
            autoSizing = enabled
            revision += 1
            if committed, let webView { updateHeightRelay(in: webView) }
        }

        private func updateHeightRelay(in webView: WKWebView) {
            let value = activeRevision.map(String.init) ?? "null"
            webView.evaluateJavaScript("globalThis.openUISetHeightRevision?.(\(value))", in: nil, in: .page) { [weak self] result in
                if case let .failure(error) = result {
                    self?.lastRelayFailure = error as NSError
                    self?.handleFailure(nil, error: error)
                }
            }
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
            guard !invalidated, let target = action.targetFrame, let url = action.request.url?.absoluteString else {
                decisionHandler(.cancel)
                return
            }
            // Admit only the trusted wrapper and its initial srcdoc.
            switch (target.isMainFrame, url) {
            case (true, "about:blank") where !openedWrapper:
                openedWrapper = true
                decisionHandler(.allow)
            case (false, "about:srcdoc") where documentFrame == nil:
                documentFrame = target
                decisionHandler(.allow)
            default:
                decisionHandler(.cancel)
            }
        }

        func webView(_ webView: WKWebView, didCommit navigation: WKNavigation!) {
            guard !invalidated, let initialNavigation, navigation === initialNavigation else { return }
            committed = true
            updateHeightRelay(in: webView)
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            guard !invalidated, let initialNavigation, navigation === initialNavigation else { return }
            updateHeightRelay(in: webView)
            self.initialNavigation = nil
        }

        func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
            contentProcessTerminated = true
            stop()
        }

        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            lastProvisionalFailure = error as NSError
            handleFailure(navigation, error: error)
        }

        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            lastNavigationFailure = error as NSError
            handleFailure(navigation, error: error)
        }

        private func handleFailure(_ navigation: WKNavigation?, error: Error) {
            if (error as? WKError)?.code == .webContentProcessTerminated ||
                (navigation != nil && navigation === initialNavigation && (error as? URLError)?.code != .cancelled) {
                stop()
            }
        }

        func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
            guard !invalidated, message.webView === webView, message.world == .defaultClient, message.frameInfo.isMainFrame,
                  let report = message.body as? [Double], report.count == 2,
                  let height = report.last, height.isFinite else { return }
            lastHeight = height
            guard let activeRevision, report.first == Double(activeRevision) else { return }
            onHeight(height)
        }
    }
}
